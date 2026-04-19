//! Transitional monolith module. Everything here is cherry-picked into focused
//! sibling modules over refactor stages 2–10; the module itself goes away in
//! stage 11.
#![allow(clippy::too_many_lines)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::cognitive_complexity)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::pedantic)]
#![allow(clippy::nursery)]

use std::{
    collections::HashMap,
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    AppState, OperationStatus,
    html::handler::{
        root::root_page,
        settings::settings_page,
        share::{share_work_form, share_work_view},
    },
    model::{
        config::{
            bridge_config_uses_yaml,
            handler::{get_config, update_config, update_config_form},
            service::{load_bridge_config, load_persistent_state, persist_bridge_config},
        },
        pin::service::{
            handler::{
                add_files, diagnose_single_pin, list_pins, list_pins_page, pin_cid, repair_now,
                retry_pin_now, retry_sync_single, set_pin_tags, sync_now, unwatch_pins,
                verify_pins, verify_single_pin,
            },
            pin_work_payload, repair_watched_pins,
        },
        relay::{
            PairingDeepLink, RelayForceDisconnectMessage, RelayJobMessage, RelayJobResultMessage,
            RelayRequestInventoryMessage, RelayShareWorkPayload, RelayWelcomeMessage,
            build_relay_socket_url,
            handler::{
                link_relay_device, link_relay_device_form, share_profile, share_work,
                unlink_relay_device, unlink_relay_device_form,
            },
            service::{
                clear_relay_link, remember_relay_error, remember_relay_success,
                send_relay_inventory,
            },
        },
        session::handler::{connect_session, disconnect_session, list_sessions, session_by_id},
        system::{
            handler::{
                add_private_network_access_header, artist_summary_handler, export_pins_handler,
                gateway_health_handler, health, live_status_handler, storage_stats_handler,
            },
            service::notify_work_share_success,
        },
    },
    util::url::trim_trailing_slash,
};

use anyhow::{Context, anyhow};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use tokio::time::sleep;
use tokio::{
    net::TcpListener,
    sync::RwLock,
    time::{Duration, interval},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::{error, info, warn};
use url::Url;

const MAX_UPLOAD_BYTES: usize = 5 * 1024 * 1024 * 1024;

fn bridge_origin_from_env() -> String {
    let host = env::var("BRIDGE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("BRIDGE_PORT").unwrap_or_else(|_| "43128".to_string());
    format!("http://{host}:{port}")
}

fn parse_pairing_deep_link(raw: &str) -> anyhow::Result<PairingDeepLink> {
    let url = Url::parse(raw).with_context(|| format!("Unable to parse deep link: {raw}"))?;
    let scheme = url.scheme().to_ascii_lowercase();
    if scheme != "foundationsharebridge" && scheme != "foundation-share-bridge" {
        anyhow::bail!("Unsupported deep link scheme: {}", url.scheme());
    }

    let action = url.host_str().unwrap_or_default();
    if action != "pair" && url.path().trim_matches('/') != "pair" {
        anyhow::bail!("Unsupported deep link action. Expected pair.");
    }

    let mut relay_server_url = None;
    let mut pairing_code = None;
    let mut device_name = None;

    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "relay_server_url" => relay_server_url = Some(value.into_owned()),
            "pairing_code" => pairing_code = Some(value.into_owned()),
            "device_name" => device_name = Some(value.into_owned()),
            _ => {}
        }
    }

    let relay_server_url = relay_server_url
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("relay_server_url is required"))?;
    let pairing_code = pairing_code
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("pairing_code is required"))?;
    let device_name =
        device_name.map(|value| value.trim().to_string()).filter(|value| !value.is_empty());

    Ok(PairingDeepLink { relay_server_url, pairing_code, device_name })
}

async fn wait_for_local_bridge_ready(client: &Client, bridge_origin: &str) -> anyhow::Result<()> {
    let health_url = format!("{}/health", trim_trailing_slash(bridge_origin));

    for _ in 0..40 {
        if let Ok(response) = client.get(&health_url).send().await
            && response.status().is_success()
        {
            return Ok(());
        }

        sleep(Duration::from_millis(500)).await;
    }

    Err(anyhow!("The local bridge did not come online at {} in time.", health_url))
}

async fn handle_deep_link_command(raw: &str) -> anyhow::Result<()> {
    let deep_link = parse_pairing_deep_link(raw)?;
    let bridge_origin = bridge_origin_from_env();
    let client = Client::builder()
        .user_agent("foundation-share-bridge/0.1 deeplink")
        .build()
        .context("Unable to build HTTP client for deep link handling")?;

    wait_for_local_bridge_ready(&client, &bridge_origin).await?;

    let response = client
        .post(format!("{}/relay/link", trim_trailing_slash(&bridge_origin)))
        .json(&serde_json::json!({
            "relay_server_url": deep_link.relay_server_url,
            "pairing_code": deep_link.pairing_code,
            "device_name": deep_link.device_name,
        }))
        .send()
        .await
        .context("Unable to send deep link pairing request to the local bridge")?;

    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Deep link pairing failed: {}", body);
    }

    info!("desktop pairing deep link forwarded successfully");
    Ok(())
}

/// Start the HTTP server. Invoked from `src/main.rs` after tracing init.
///
/// Honours env vars `BRIDGE_HOST`, `BRIDGE_PORT`, `IPFS_API_URL`,
/// `IPFS_API_AUTH_HEADER`, `SELF_REPAIR_INTERVAL_SECONDS`, plus the
/// `BRIDGE_STATE_FILE` / `BRIDGE_CONFIG_FILE` path overrides.
///
/// Also dispatches the `handle-url` / `open-url` CLI subcommands for deep-link
/// pairing.
///
/// # Errors
///
/// Returns an error if bind, config load, persistence, or HTTP client init
/// fails, or if the axum server exits unexpectedly.
pub async fn run() -> anyhow::Result<()> {
    let mut args = env::args().skip(1);
    if let Some(command) = args.next()
        && (command == "handle-url" || command == "open-url")
    {
        let raw_url = args
            .next()
            .ok_or_else(|| anyhow!("Usage: foundation-share-bridge handle-url <app-url>"))?;
        return handle_deep_link_command(&raw_url).await;
    }

    let host = env::var("BRIDGE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("BRIDGE_PORT").unwrap_or_else(|_| "43128".to_string());
    let ipfs_api_url =
        env::var("IPFS_API_URL").unwrap_or_else(|_| "http://127.0.0.1:5001".to_string());
    let ipfs_api_auth_header = env::var("IPFS_API_AUTH_HEADER").ok();
    let repair_interval_seconds = env::var("SELF_REPAIR_INTERVAL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.max(60))
        .unwrap_or(900);
    let state_file = bridge_state_file_from_env()?;
    let config_file = bridge_config_file_from_env(&state_file)?;
    let should_seed_config_file = bridge_config_uses_yaml(&config_file) && !config_file.exists();

    let address: SocketAddr = format!("{host}:{port}")
        .parse()
        .with_context(|| format!("Unable to parse bridge bind address {host}:{port}"))?;

    let persistent = load_persistent_state(&state_file).await?;
    let config = load_bridge_config(&config_file, &state_file).await?;

    let state = AppState {
        http: Client::builder()
            .user_agent("foundation-share-bridge/0.1")
            .build()
            .context("Unable to build HTTP client")?,
        ipfs_api_url,
        ipfs_api_auth_header,
        state_file,
        config_file,
        repair_interval_seconds,
        sessions: Arc::new(RwLock::new(HashMap::new())),
        persistent: Arc::new(RwLock::new(persistent)),
        config: Arc::new(RwLock::new(config)),
        operation: Arc::new(RwLock::new(OperationStatus::idle())),
    };

    if should_seed_config_file {
        persist_bridge_config(&state).await?;
    }

    spawn_repair_loop(state.clone());
    spawn_relay_socket_loop(state.clone());

    let app = Router::new()
        .route("/", get(root_page))
        .route("/settings", get(settings_page))
        .route("/health", get(health))
        .route("/sessions", get(list_sessions))
        .route("/session/connect", post(connect_session))
        .route("/session/disconnect", post(disconnect_session))
        .route("/session/{session_id}", get(session_by_id))
        .route("/config", get(get_config).post(update_config))
        .route("/settings/form", post(update_config_form))
        .route("/relay/link", post(link_relay_device))
        .route("/relay/unlink", post(unlink_relay_device))
        .route("/relay/link/form", post(link_relay_device_form))
        .route("/relay/unlink/form", post(unlink_relay_device_form))
        .route("/pins", get(list_pins))
        .route("/pins/page", get(list_pins_page))
        .route("/pins/repair", post(repair_now))
        .route("/pins/verify", post(verify_pins))
        .route("/pins/unwatch", post(unwatch_pins))
        .route("/pins/item/{cid}/verify", post(verify_single_pin))
        .route("/pins/item/{cid}/diagnose", post(diagnose_single_pin))
        .route("/pins/item/{cid}/retry", post(retry_pin_now))
        .route("/pins/item/{cid}/retry-sync", post(retry_sync_single))
        .route("/pins/item/{cid}/tags", post(set_pin_tags))
        .route("/pins/export", get(export_pins_handler))
        .route("/gateway/health", get(gateway_health_handler))
        .route("/storage/stats", get(storage_stats_handler))
        .route("/status/live", get(live_status_handler))
        .route("/artists/summary", get(artist_summary_handler))
        .route("/sync/run", post(sync_now))
        .route("/ipfs/pin", post(pin_cid))
        .route("/ipfs/add", post(add_files).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)))
        .route("/share/work", post(share_work))
        .route("/share/work/view", get(share_work_view))
        .route("/share/work/form", post(share_work_form))
        .route("/share/profile", post(share_profile))
        .layer(CorsLayer::new().allow_origin(Any).allow_headers(Any).allow_methods(Any))
        .layer(middleware::map_response(add_private_network_access_header))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("Unable to bind bridge listener on {address}"))?;

    info!("foundation-share-bridge listening on http://{address}");
    axum::serve(listener, app).await.context("Bridge server stopped unexpectedly")?;

    Ok(())
}

fn bridge_state_file_from_env() -> anyhow::Result<PathBuf> {
    if let Some(value) = env::var("BRIDGE_STATE_FILE").ok().filter(|value| !value.trim().is_empty())
    {
        return Ok(PathBuf::from(value));
    }

    let cwd = env::current_dir().context("Unable to determine current directory")?;
    Ok(cwd.join("bridge-state.json"))
}

fn bridge_config_file_from_env(state_file: &Path) -> anyhow::Result<PathBuf> {
    if let Some(value) =
        env::var("BRIDGE_CONFIG_FILE").ok().filter(|value| !value.trim().is_empty())
    {
        return Ok(PathBuf::from(value));
    }

    if let Some(parent) = state_file.parent() {
        let yaml_path = parent.join("bridge-config.yaml");
        if yaml_path.exists() {
            return Ok(yaml_path);
        }

        let yml_path = parent.join("bridge-config.yml");
        if yml_path.exists() {
            return Ok(yml_path);
        }

        let json_path = parent.join("bridge-config.json");
        if json_path.exists() {
            return Ok(json_path);
        }

        return Ok(yaml_path);
    }

    let cwd = env::current_dir().context("Unable to determine current directory")?;
    Ok(cwd.join("bridge-config.yaml"))
}

fn spawn_repair_loop(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(state.repair_interval_seconds));

        loop {
            ticker.tick().await;

            if let Err(error) = repair_watched_pins(&state).await {
                error!("self-repair cycle failed: {error}");
            }
        }
    });
}

fn spawn_relay_socket_loop(state: AppState) {
    tokio::spawn(async move {
        let mut backoff_seconds = 2u64;

        loop {
            let config = { state.config.read().await.clone() };

            if !config.relay_enabled
                || config.relay_server_url.trim().is_empty()
                || config.relay_device_token.as_deref().map(str::trim).unwrap_or("").is_empty()
            {
                sleep(Duration::from_secs(2)).await;
                backoff_seconds = 2;
                continue;
            }

            match run_relay_socket_session(&state).await {
                Ok(()) => {
                    backoff_seconds = 2;
                    sleep(Duration::from_secs(2)).await;
                }
                Err(error) => {
                    warn!("relay socket cycle failed: {error}");
                    if state.config.read().await.relay_enabled {
                        let _ = remember_relay_error(&state, error.to_string()).await;
                    }
                    sleep(Duration::from_secs(backoff_seconds)).await;
                    backoff_seconds = (backoff_seconds * 2).min(30);
                }
            }
        }
    });
}

async fn run_relay_socket_session(state: &AppState) -> anyhow::Result<()> {
    let config = { state.config.read().await.clone() };
    let device_token = config
        .relay_device_token
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("Relay device token is missing"))?;

    let socket_url = build_relay_socket_url(&config.relay_server_url, &device_token)?;
    let (mut socket, response) = connect_async(socket_url.as_str())
        .await
        .context("Unable to connect to the archive relay websocket")?;

    remember_relay_success(
        state,
        config.relay_device_id.clone(),
        config.relay_device_label.clone(),
    )
    .await?;

    info!("relay websocket connected: {} ({})", response.status(), config.relay_server_url);

    send_relay_inventory(state, &mut socket).await?;

    while let Some(message) = socket.next().await {
        match message? {
            Message::Text(text) => {
                let value = serde_json::from_str::<serde_json::Value>(&text)
                    .context("Unable to parse relay websocket message")?;
                let message_type = value.get("type").and_then(|item| item.as_str()).unwrap_or("");

                match message_type {
                    "relay.welcome" => {
                        let payload = serde_json::from_value::<RelayWelcomeMessage>(value)?;
                        remember_relay_success(state, payload.device_id, payload.device_label)
                            .await?;
                    }
                    "relay.requestInventory" => {
                        let _ = serde_json::from_value::<RelayRequestInventoryMessage>(value)?;
                        send_relay_inventory(state, &mut socket).await?;
                    }
                    "relay.job" => {
                        let payload = serde_json::from_value::<RelayJobMessage>(value)?;
                        let result = match payload.kind.as_str() {
                            "SHARE_WORK" => {
                                let input = serde_json::from_str::<RelayShareWorkPayload>(
                                    &payload.payload,
                                )?;
                                let work_title = input.title.clone();
                                let pins = pin_work_payload(state, input)
                                    .await
                                    .map_err(|error| anyhow!(error.message))?;
                                notify_work_share_success(&work_title, pins.len());
                                serde_json::to_string(&serde_json::json!({ "pins": pins }))
                                    .map_err(anyhow::Error::from)
                            }
                            other => Err(anyhow!("Unsupported relay job kind: {other}")),
                        };

                        match result {
                            Ok(result_payload) => {
                                let payload = RelayJobResultMessage {
                                    r#type: "device.jobResult",
                                    job_id: payload.job_id,
                                    status: "COMPLETED",
                                    result_payload: Some(result_payload),
                                    error_message: None,
                                };
                                socket
                                    .send(Message::Text(
                                        serde_json::to_string(&payload)
                                            .context("Unable to encode relay job result")?
                                            .into(),
                                    ))
                                    .await?;
                                send_relay_inventory(state, &mut socket).await?;
                            }
                            Err(error) => {
                                let payload = RelayJobResultMessage {
                                    r#type: "device.jobResult",
                                    job_id: payload.job_id,
                                    status: "FAILED",
                                    result_payload: None,
                                    error_message: Some(error.to_string()),
                                };
                                socket
                                    .send(Message::Text(
                                        serde_json::to_string(&payload)
                                            .context("Unable to encode relay job error")?
                                            .into(),
                                    ))
                                    .await?;
                            }
                        }
                    }
                    "relay.forceDisconnect" => {
                        let payload = serde_json::from_value::<RelayForceDisconnectMessage>(value)?;
                        clear_relay_link(state).await?;
                        return Err(anyhow!(
                            "Archive relay disconnected this desktop app: {}",
                            payload.reason.unwrap_or_else(|| "connection closed".to_string())
                        ));
                    }
                    _ => {}
                }
            }
            Message::Ping(payload) => {
                socket.send(Message::Pong(payload)).await?;
            }
            Message::Close(_) => {
                break;
            }
            _ => {}
        }
    }

    Ok(())
}
