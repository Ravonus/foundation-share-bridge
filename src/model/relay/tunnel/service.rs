//! Tunnel background loop — owns the `cloudflared` child process and calls
//! the archive web backend to provision/revoke a named tunnel for this
//! device.
#![allow(clippy::too_many_lines, clippy::cognitive_complexity, clippy::pedantic, clippy::nursery)]

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use anyhow::{Context, anyhow};
use chrono::Utc;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tokio::{
    process::{Child, Command},
    sync::Mutex,
    time::{Duration, sleep},
};
use tracing::{info, warn};

use crate::{
    AppState,
    model::{
        config::service::persist_bridge_config,
        relay::tunnel::{
            install::ensure_cloudflared_binary,
            kubo_announce::try_ensure_kubo_wss_advertisement,
        },
    },
};

#[derive(Debug, Serialize)]
struct ProvisionRequest<'a> {
    #[serde(rename = "deviceToken")]
    device_token: &'a str,
    #[serde(rename = "localService", skip_serializing_if = "Option::is_none")]
    local_service: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProvisionResponse {
    hostname: Option<String>,
    subdomain: Option<String>,
    #[serde(rename = "tunnelToken")]
    tunnel_token: Option<String>,
    #[serde(rename = "libp2pHostname", default)]
    libp2p_hostname: Option<String>,
    #[serde(rename = "libp2pSubdomain", default)]
    libp2p_subdomain: Option<String>,
}

#[derive(Debug, Serialize)]
struct RevokeRequest<'a> {
    #[serde(rename = "deviceToken")]
    device_token: &'a str,
}

/// Spawn the tunnel supervisor loop. It is cheap to spawn and idempotent —
/// safe to call once at bridge startup. All state lives inside the task.
pub fn spawn_tunnel_loop(state: AppState) {
    tokio::spawn(async move {
        run_tunnel_supervisor(state).await;
    });
}

async fn run_tunnel_supervisor(state: AppState) {
    let child_slot: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    let mut provision_backoff = 2u64;

    loop {
        let (enabled, device_token, relay_server_url, token_in_config, local_service) = {
            let config = state.config.read().await;
            (
                config.tunnel_enabled,
                config.relay_device_token.clone(),
                config.relay_server_url.clone(),
                config.tunnel_token.clone(),
                config.local_gateway_base_url.clone(),
            )
        };

        if !enabled {
            // Tunnel turned off (or never on). Kill the cloudflared subprocess
            // if we were running one, and if we still hold a token in config
            // it means the user just flipped it off — revoke it.
            kill_child(&child_slot).await;

            if token_in_config.is_some() {
                match revoke_tunnel(&state, &relay_server_url, device_token.as_deref()).await {
                    Ok(()) => {
                        clear_tunnel_fields_in_config(&state).await;
                        let _ = persist_bridge_config(&state).await;
                        info!("cloudflare tunnel revoked");
                    }
                    Err(error) => {
                        warn!("tunnel revoke failed: {error}");
                        write_tunnel_error(&state, format!("revoke failed: {error}")).await;
                    }
                }
            }

            sleep(Duration::from_secs(2)).await;
            continue;
        }

        let Some(device_token) =
            device_token.as_deref().map(str::trim).filter(|value| !value.is_empty())
        else {
            write_tunnel_error(
                &state,
                "Pair this desktop app with the archive before enabling the public tunnel."
                    .to_string(),
            )
            .await;
            sleep(Duration::from_secs(5)).await;
            continue;
        };

        // Ensure we have a token + hostname. Provision if missing.
        let tunnel_token = if let Some(existing) = token_in_config {
            existing
        } else {
            match provision_tunnel(&state, &relay_server_url, device_token, &local_service).await {
                Ok(provisioned) => {
                    apply_provisioned(&state, &provisioned).await;
                    let _ = persist_bridge_config(&state).await;
                    provision_backoff = 2;
                    match provisioned.tunnel_token {
                        Some(token) => token,
                        None => {
                            sleep(Duration::from_secs(provision_backoff)).await;
                            continue;
                        }
                    }
                }
                Err(error) => {
                    warn!("tunnel provision failed: {error}");
                    write_tunnel_error(&state, format!("provision failed: {error}")).await;
                    sleep(Duration::from_secs(provision_backoff)).await;
                    provision_backoff = (provision_backoff * 2).min(60);
                    continue;
                }
            }
        };

        // Ensure cloudflared is running.
        let needs_spawn = {
            let mut guard = child_slot.lock().await;
            match guard.as_mut() {
                Some(child) => match child.try_wait() {
                    Ok(Some(status)) => {
                        warn!("cloudflared exited unexpectedly: {status}");
                        write_tunnel_error(&state, format!("cloudflared exited: {status}")).await;
                        *guard = None;
                        true
                    }
                    Ok(None) => false,
                    Err(error) => {
                        warn!("cloudflared status check failed: {error}");
                        *guard = None;
                        true
                    }
                },
                None => true,
            }
        };

        if needs_spawn {
            let cache_dir = cloudflared_cache_dir(&state);
            match spawn_cloudflared(&cache_dir, &tunnel_token).await {
                Ok(child) => {
                    info!("cloudflared started");
                    *child_slot.lock().await = Some(child);
                    clear_tunnel_error_in_config(&state).await;
                    let _ = persist_bridge_config(&state).await;
                }
                Err(error) => {
                    warn!("cloudflared spawn failed: {error}");
                    write_tunnel_error(&state, format!("cloudflared spawn failed: {error}")).await;
                    sleep(Duration::from_secs(10)).await;
                    continue;
                }
            }
        }

        // Re-apply the Kubo WS listener + WSS announce every tick. Cheap to
        // call when nothing's changed (GET + compare) and idempotent when
        // the tunnel hostname is rotated or Kubo's config drifts.
        try_ensure_kubo_wss_advertisement(&state).await;

        sleep(Duration::from_secs(3)).await;
    }
}

async fn spawn_cloudflared(cache_dir: &Path, tunnel_token: &str) -> anyhow::Result<Child> {
    let binary = ensure_cloudflared_binary(cache_dir).await?;

    let mut command = Command::new(&binary);
    command
        .arg("tunnel")
        .arg("--no-autoupdate")
        .arg("run")
        .arg("--token")
        .arg(tunnel_token)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    command.spawn().with_context(|| format!("Unable to spawn {} tunnel run", binary.display()))
}

async fn kill_child(slot: &Arc<Mutex<Option<Child>>>) {
    let mut guard = slot.lock().await;
    if let Some(mut child) = guard.take() {
        let _ = child.kill().await;
    }
}

async fn provision_tunnel(
    state: &AppState,
    relay_server_url: &str,
    device_token: &str,
    local_service: &str,
) -> anyhow::Result<ProvisionedTunnel> {
    let url = build_provision_url(relay_server_url, "provision")?;

    let response = state
        .http
        .post(url.clone())
        .json(&ProvisionRequest { device_token, local_service: Some(local_service.to_string()) })
        .send()
        .await
        .with_context(|| format!("Unable to POST {url}"))?;

    let status = response.status();
    let body_text = response.text().await.unwrap_or_default();

    if !status.is_success() {
        let message = serde_json::from_str::<serde_json::Value>(&body_text)
            .ok()
            .and_then(|value| value.get("error").and_then(|e| e.as_str().map(String::from)))
            .unwrap_or_else(|| {
                if status.as_u16() == 404 {
                    format!(
                        "{relay_server_url} is missing the tunnel provisioning routes (404). Deploy the latest backend or point relay_server_url at a dev build that has /api/relay/bridge/tunnel/*."
                    )
                } else {
                    format!("tunnel provisioning failed ({status})")
                }
            });
        return Err(anyhow!("{message}"));
    }

    let parsed: ProvisionResponse =
        serde_json::from_str(&body_text).context("Unable to parse tunnel provisioning payload")?;

    Ok(ProvisionedTunnel {
        hostname: parsed.hostname,
        subdomain: parsed.subdomain,
        tunnel_token: parsed.tunnel_token,
        libp2p_hostname: parsed.libp2p_hostname,
        libp2p_subdomain: parsed.libp2p_subdomain,
    })
}

async fn revoke_tunnel(
    state: &AppState,
    relay_server_url: &str,
    device_token: Option<&str>,
) -> anyhow::Result<()> {
    let Some(device_token) = device_token.map(str::trim).filter(|value| !value.is_empty()) else {
        // No device token means we can't reach the backend — leave local
        // state as-is and hope the backend cleans up on device unpair.
        return Ok(());
    };

    let url = build_provision_url(relay_server_url, "revoke")?;
    let response = state
        .http
        .post(url.clone())
        .json(&RevokeRequest { device_token })
        .send()
        .await
        .with_context(|| format!("Unable to POST {url}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("tunnel revoke failed ({status}): {body}"));
    }

    Ok(())
}

fn build_provision_url(base: &str, action: &str) -> anyhow::Result<Url> {
    let base = base.trim_end_matches('/');
    let raw = format!("{base}/api/relay/bridge/tunnel/{action}");
    Url::parse(&raw).with_context(|| format!("Unable to parse tunnel URL {raw}"))
}

struct ProvisionedTunnel {
    hostname: Option<String>,
    subdomain: Option<String>,
    tunnel_token: Option<String>,
    libp2p_hostname: Option<String>,
    libp2p_subdomain: Option<String>,
}

async fn apply_provisioned(state: &AppState, provisioned: &ProvisionedTunnel) {
    let mut config = state.config.write().await;
    config.tunnel_hostname = provisioned.hostname.clone();
    config.tunnel_subdomain = provisioned.subdomain.clone();
    config.tunnel_token = provisioned.tunnel_token.clone();
    config.tunnel_provisioned_at = Some(Utc::now());
    config.tunnel_last_error = None;
    config.libp2p_hostname = provisioned.libp2p_hostname.clone();
    config.libp2p_subdomain = provisioned.libp2p_subdomain.clone();
}

async fn clear_tunnel_fields_in_config(state: &AppState) {
    let mut config = state.config.write().await;
    config.tunnel_hostname = None;
    config.tunnel_subdomain = None;
    config.tunnel_token = None;
    config.tunnel_provisioned_at = None;
    config.tunnel_last_error = None;
    config.libp2p_hostname = None;
    config.libp2p_subdomain = None;
    config.libp2p_last_error = None;
    config.libp2p_applied_at = None;
}

async fn write_tunnel_error(state: &AppState, message: String) {
    let mut config = state.config.write().await;
    config.tunnel_last_error = Some(message);
}

async fn clear_tunnel_error_in_config(state: &AppState) {
    let mut config = state.config.write().await;
    config.tunnel_last_error = None;
}

fn cloudflared_cache_dir(state: &AppState) -> PathBuf {
    state.state_file.parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf).join("tools")
}
