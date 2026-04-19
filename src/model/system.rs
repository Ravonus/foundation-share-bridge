//! System-health DTOs — `/health`, `/storage/stats`, `/gateway/health`,
//! `/diagnose/:cid`, the underlying Kubo `repo/stat` shape, and the artist
//! summary surfaced on the dashboard — plus the live gateway-reachability
//! probes that feed the `/health` and `/diagnose` endpoints.

// `struct_excessive_bools`: the `/health` wire format is fixed; the response
// intentionally exposes every relevant boolean state.
#![allow(clippy::struct_excessive_bools)]

use std::{net::Ipv4Addr, time::Duration};

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{
    AppState, OperationStatus,
    util::url::{PUBLIC_UTILITY_GATEWAY_BASE_URL, build_gateway_url},
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: &'static str,
    pub service: &'static str,
    pub ipfs_api_url: String,
    pub state_file: String,
    pub config_file: String,
    pub active_sessions: usize,
    pub watched_pin_count: usize,
    pub repair_interval_seconds: u64,
    pub last_repair_cycle_at: Option<DateTime<Utc>>,
    pub download_root_dir: String,
    pub sync_enabled: bool,
    pub local_gateway_base_url: String,
    pub public_gateway_base_url: String,
    pub relay_enabled: bool,
    pub relay_server_url: String,
    pub relay_device_name: String,
    pub relay_device_id: Option<String>,
    pub relay_device_label: Option<String>,
    pub relay_last_connected_at: Option<DateTime<Utc>>,
    pub relay_last_error: Option<String>,
    pub now: DateTime<Utc>,
    pub storage: StorageSnapshot,
    pub operation: OperationStatus,
    pub remote_pinning_enabled: bool,
    pub onboarded: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageSnapshot {
    pub repo_size_bytes: Option<u64>,
    pub storage_max_bytes: Option<u64>,
    pub num_objects: Option<u64>,
    pub synced_bytes_on_disk: u64,
    pub quota_gb: Option<f64>,
    pub quota_used_fraction: Option<f64>,
    pub ipfs_daemon_reachable: bool,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayHealthResponse {
    pub local_gateway_base_url: String,
    pub public_gateway_base_url: String,
    pub utility_gateway_base_url: &'static str,
    pub local_ok: Option<bool>,
    pub public_ok: Option<bool>,
    pub utility_ok: Option<bool>,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseResponse {
    pub cid: String,
    pub pinned_locally: bool,
    pub provider_count: usize,
    pub reachable_on_dht: bool,
    pub error_category: Option<String>,
    pub error_hint: Option<String>,
    pub last_error: Option<String>,
    pub raw_error: Option<String>,
    pub checked_at: DateTime<Utc>,
    pub gateway_local_ok: Option<bool>,
    pub gateway_public_ok: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KuboRepoStat {
    #[serde(rename = "RepoSize")]
    pub repo_size: Option<u64>,
    #[serde(rename = "StorageMax")]
    pub storage_max: Option<u64>,
    #[serde(rename = "NumObjects")]
    pub num_objects: Option<u64>,
    #[serde(rename = "RepoPath")]
    pub repo_path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistSummary {
    pub total_works_managed: usize,
    pub works_by_you: usize,
    pub artists_tracked: usize,
    pub top_artists: Vec<ArtistEntry>,
    pub total_copies_pinned: usize,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ArtistEntry {
    pub artist_username: String,
    pub works: usize,
}

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
