//! Live network-reachability probes for the IPFS gateways and public IPv4
//! detection. Used by the `/health`, `/gateway/health`, and `/diagnose`
//! endpoints so the UI can pinpoint which hop is failing.

use std::{net::Ipv4Addr, time::Duration};

use chrono::Utc;
use reqwest::Client;
use serde::Deserialize;

use super::types::GatewayHealthResponse;
use crate::{
    AppState,
    util::url::{PUBLIC_UTILITY_GATEWAY_BASE_URL, build_gateway_url},
};

/// Best-effort public IPv4 discovery via ipify. Returns `None` when the probe
/// times out, the payload parses to a non-IPv4 address, or the network is
/// unavailable — callers fall back to `None` when the field is unknown.
pub async fn detect_public_ipv4(state: &AppState) -> Option<String> {
    #[derive(Debug, Deserialize)]
    struct IpifyResponse {
        ip: String,
    }

    let response = state
        .http
        .get("https://api4.ipify.org?format=json")
        .timeout(Duration::from_secs(4))
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let payload = response.json::<IpifyResponse>().await.ok()?;
    let parsed = payload.ip.parse::<Ipv4Addr>().ok()?;
    Some(parsed.to_string())
}

/// HEAD-probes a single gateway URL, returning `Some(true)` for a 2xx/3xx
/// response, `Some(false)` for any other HTTP status, and `None` when the
/// request itself couldn't be completed (DNS, timeout, connection refused).
pub async fn probe_gateway(client: &Client, url: &str) -> Option<bool> {
    let response = client.head(url).timeout(Duration::from_secs(5)).send().await.ok()?;
    Some(response.status().is_success() || response.status().is_redirection())
}

/// Fan-out of [`probe_gateway`] against both configured gateways for a given
/// CID. Used by `/diagnose/:cid` so the UI can tell whether the local daemon
/// or the public mirror is the one failing.
pub async fn check_gateway_reachability(
    state: &AppState,
    cid: &str,
) -> (Option<bool>, Option<bool>) {
    let (local_base, public_base) = {
        let config = state.config.read().await;
        (config.local_gateway_base_url.clone(), config.public_gateway_base_url.clone())
    };
    let local = probe_gateway(&state.http, &build_gateway_url(&local_base, cid)).await;
    let public = probe_gateway(&state.http, &build_gateway_url(&public_base, cid)).await;
    (local, public)
}

/// `/gateway/health` response — probes the configured local, public, and
/// hard-coded utility gateways using a tiny well-known CID (`bafkqaaa`, the
/// empty file) so the call is cheap even when Kubo is cold.
pub async fn gateway_health_probe(state: &AppState) -> GatewayHealthResponse {
    const PROBE_CID: &str = "bafkqaaa";

    let (local_base, public_base) = {
        let config = state.config.read().await;
        (config.local_gateway_base_url.clone(), config.public_gateway_base_url.clone())
    };
    let local_ok = probe_gateway(&state.http, &build_gateway_url(&local_base, PROBE_CID)).await;
    let public_ok = probe_gateway(&state.http, &build_gateway_url(&public_base, PROBE_CID)).await;
    let utility_ok =
        probe_gateway(&state.http, &build_gateway_url(PUBLIC_UTILITY_GATEWAY_BASE_URL, PROBE_CID))
            .await;
    GatewayHealthResponse {
        local_gateway_base_url: local_base,
        public_gateway_base_url: public_base,
        utility_gateway_base_url: PUBLIC_UTILITY_GATEWAY_BASE_URL,
        local_ok,
        public_ok,
        utility_ok,
        checked_at: Utc::now(),
    }
}
