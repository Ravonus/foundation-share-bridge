//! Wire the provisioned libp2p tunnel hostname into Kubo so that ipfs.io /
//! dweb.link can actually dial this node for pinned content.
//!
//! The archive's provisioning service hands us a second subdomain on the same
//! cloudflared tunnel. Traffic for `wss://{hostname}` lands at
//! `http://localhost:{port}` on this machine — that port needs to be a Kubo
//! libp2p WebSocket listener. We do three things:
//!
//!   1. Ensure `Addresses.Swarm` includes `/ip4/0.0.0.0/tcp/{port}/ws` so
//!      Kubo actually accepts incoming connections on that port.
//!   2. Ensure `Addresses.AppendAnnounce` advertises
//!      `/dns4/{hostname}/tcp/443/tls/ws/p2p/{peer_id}` to the DHT, so
//!      remote gateways discover the dialable address.
//!   3. Mark the config as "applied". Kubo picks up address changes on
//!      restart — the bridge surfaces a "restart IPFS to activate" hint in
//!      the UI when the applied timestamp is older than the tunnel
//!      provisioning timestamp OR the running listener set does not match.
//!
//! Idempotent: safe to call on every tunnel loop tick.

use anyhow::{Context, anyhow};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AppState, model::config::service::persist_bridge_config};

/// Default local port we ask Kubo to bind its WS listener to. Matches the
/// `DEFAULT_LIBP2P_WS_PORT` in the archive's tunnel-service ingress rule.
pub const DEFAULT_LIBP2P_WS_PORT: u16 = 4002;

#[derive(Debug, Deserialize)]
struct KuboIdResponse {
    #[serde(rename = "ID")]
    id: String,
}

#[derive(Debug, Deserialize)]
struct KuboConfigValue {
    #[serde(rename = "Value")]
    value: Value,
}

fn ws_swarm_address(port: u16) -> String {
    format!("/ip4/0.0.0.0/tcp/{port}/ws")
}

fn ws_announce_address(hostname: &str, peer_id: &str) -> String {
    format!("/dns4/{hostname}/tcp/443/tls/ws/p2p/{peer_id}")
}

async fn fetch_kubo_peer_id(state: &AppState) -> anyhow::Result<String> {
    let url = format!("{}/api/v0/id", state.ipfs_api_url.trim_end_matches('/'));
    let mut request = state.http.post(&url);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }
    let response =
        request.send().await.with_context(|| format!("Unable to reach Kubo API at {url}"))?;
    if !response.status().is_success() {
        return Err(anyhow!("Kubo /api/v0/id returned HTTP {}", response.status()));
    }
    let parsed = response.json::<KuboIdResponse>().await.context("Malformed /api/v0/id payload")?;
    Ok(parsed.id)
}

async fn fetch_kubo_config_array(state: &AppState, key: &str) -> anyhow::Result<Vec<String>> {
    let url = format!("{}/api/v0/config?arg={key}", state.ipfs_api_url.trim_end_matches('/'));
    let mut request = state.http.post(&url);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }
    let response =
        request.send().await.with_context(|| format!("Unable to read Kubo config {key}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("Kubo config {key} returned HTTP {status}"));
    }
    let wrapped =
        response.json::<KuboConfigValue>().await.context("Malformed Kubo config payload")?;
    Ok(value_to_string_vec(&wrapped.value))
}

fn value_to_string_vec(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => {
            items.iter().filter_map(|entry| entry.as_str().map(str::to_string)).collect()
        }
        _ => Vec::new(),
    }
}

async fn write_kubo_config(state: &AppState, key: &str, items: &[String]) -> anyhow::Result<()> {
    let json_body =
        serde_json::to_string(&json!(items)).context("Unable to encode Kubo config value")?;
    let url = format!(
        "{base}/api/v0/config?arg={key}&arg={value}&json=true",
        base = state.ipfs_api_url.trim_end_matches('/'),
        key = key,
        value = urlencoding_encode(&json_body),
    );

    let mut request = state.http.post(&url);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }
    let response =
        request.send().await.with_context(|| format!("Unable to write Kubo config {key}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Kubo config write {key} returned HTTP {status}: {body}"));
    }
    Ok(())
}

fn urlencoding_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let mut encoded = String::with_capacity(value.len());
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            encoded.push(c);
            continue;
        }

        let mut buf = [0u8; 4];
        for byte in c.encode_utf8(&mut buf).bytes() {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }

    encoded
}

fn merge_unique(existing: &[String], additional: &str) -> Option<Vec<String>> {
    if existing.iter().any(|entry| entry == additional) {
        return None;
    }
    let mut next = existing.to_vec();
    next.push(additional.to_string());
    Some(next)
}

/// Sync the announce list so `keeper` is present while every older
/// `libp2p-*.agorix.io` ingress (from a previous provisioning run) is
/// dropped. Returns `Some(new_list)` only when the list changes.
fn reconcile_libp2p_announce(existing: &[String], keeper: &str) -> Option<Vec<String>> {
    let filtered: Vec<String> = existing
        .iter()
        .filter(|entry| {
            // Keep anything that isn't one of our managed wss ingresses,
            // plus the current keeper. Stale `libp2p-*.agorix.io/tcp/443/tls/ws`
            // lines get trimmed.
            let is_managed = entry.contains("/dns4/libp2p-")
                && entry.contains(".agorix.io/")
                && entry.contains("/tls/ws/");
            !is_managed || entry.as_str() == keeper
        })
        .cloned()
        .collect();

    let mut next = filtered;
    if !next.iter().any(|entry| entry == keeper) {
        next.push(keeper.to_string());
    }

    if next.as_slice() == existing { None } else { Some(next) }
}

/// Ensure Kubo is set up to (a) listen for WS libp2p on the proxied local
/// port and (b) advertise the public WSS multiaddr to the DHT. Returns
/// `true` when the config was mutated and Kubo needs a restart to activate
/// the new listener.
pub async fn ensure_kubo_wss_advertisement(state: &AppState) -> anyhow::Result<bool> {
    let (enabled, hostname, port) = {
        let config = state.config.read().await;
        (
            config.tunnel_enabled,
            config.libp2p_hostname.clone(),
            config.libp2p_ws_local_port.unwrap_or(DEFAULT_LIBP2P_WS_PORT),
        )
    };
    let Some(hostname) = hostname.filter(|value| !value.trim().is_empty()) else {
        return Ok(false);
    };
    if !enabled {
        return Ok(false);
    }

    let peer_id = fetch_kubo_peer_id(state).await?;
    let swarm_addr = ws_swarm_address(port);
    let announce_addr = ws_announce_address(&hostname, &peer_id);

    let mut mutated = false;

    let current_swarm = fetch_kubo_config_array(state, "Addresses.Swarm").await?;
    if let Some(next) = merge_unique(&current_swarm, &swarm_addr) {
        write_kubo_config(state, "Addresses.Swarm", &next).await?;
        mutated = true;
    }

    let current_announce = fetch_kubo_config_array(state, "Addresses.AppendAnnounce").await?;
    if let Some(next) = reconcile_libp2p_announce(&current_announce, &announce_addr) {
        write_kubo_config(state, "Addresses.AppendAnnounce", &next).await?;
        mutated = true;
    }

    {
        let mut config = state.config.write().await;
        config.libp2p_applied_at = Some(Utc::now());
        config.libp2p_last_error = None;
        if config.libp2p_ws_local_port.is_none() {
            config.libp2p_ws_local_port = Some(DEFAULT_LIBP2P_WS_PORT);
        }
    }
    persist_bridge_config(state).await?;
    Ok(mutated)
}

/// Ensure `ensure_kubo_wss_advertisement` runs without surfacing errors to
/// callers. Intended for the tunnel supervisor loop which shouldn't crash on
/// transient Kubo API hiccups.
pub async fn try_ensure_kubo_wss_advertisement(state: &AppState) {
    match ensure_kubo_wss_advertisement(state).await {
        Ok(_mutated) => {}
        Err(error) => {
            let message = format!("{error:#}");
            tracing::warn!("libp2p public advertisement failed: {message}");
            {
                let mut config = state.config.write().await;
                config.libp2p_last_error = Some(message);
            }
            let _ = persist_bridge_config(state).await;
        }
    }
}
