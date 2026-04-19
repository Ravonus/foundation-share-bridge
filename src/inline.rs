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
    collections::{HashMap, HashSet},
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    AppError, AppState, OperationStatus,
    html::{
        render::{
            artist::render_artist_summary,
            page::render_page,
            settings::{render_export_card, render_gateway_card},
            status::render_live_status_panel,
        },
        scripts::{
            autolink::ROOT_AUTOLINK_SCRIPT,
            inventory::INVENTORY_BROWSER_SCRIPT,
            live_status::LIVE_STATUS_SCRIPT,
            settings::{SETTINGS_CONTROLS_SCRIPT, SETTINGS_GATEWAY_HELPER_SCRIPT},
        },
        styles::settings::SETTINGS_PAGE_STYLE,
    },
    model::{
        config::{
            BridgeConfigResponse, RootPageQuery, SettingsPageQuery, ShareWorkViewQuery,
            UpdateBridgeConfigFormRequest, UpdateBridgeConfigRequest, bridge_config_uses_yaml,
            service::{
                apply_config_update, load_bridge_config, load_persistent_state,
                persist_bridge_config, persist_bridge_state,
            },
        },
        pin::{
            AddFilesResult, AddedFileEntry, ExportQuery, PinCidRequest, PinCidResult,
            PinVerification, PinsPageQuery, PinsPageResponse, PinsResponse, RepairNowResponse,
            RetryPinResponse, RetrySyncResponse, SetPinTagsRequest, SetPinTagsResponse,
            SyncNowResponse, UnwatchPinsRequest, UnwatchPinsResponse, VerifyPinsRequest,
            VerifyPinsResponse, WatchPinInput,
            client::{
                kubo::pin_single_cid,
                remote::submit_to_remote_pinning_service,
                sync::{sync_cid_if_enabled, sync_cid_to_download_dir},
            },
            inventory::{
                INVENTORY_PAGE_SIZE, categorize_pin_error, inventory_work_group_key,
                parse_inventory_cursor, render_inventory_fallback_table,
                resolve_inventory_page_size,
            },
            service::{
                check_cid_network_providers, diagnose_pin, list_local_pin_inventory,
                list_local_pin_inventory_page, pin_and_watch_cid, pin_work_payload,
                remember_pin_verification, remember_watched_pin, repair_watched_pins,
                resolve_verify_targets, sync_all_watched_pins,
            },
        },
        relay::{
            PairingDeepLink, RelayForceDisconnectMessage, RelayJobMessage, RelayJobResultMessage,
            RelayLinkFormRequest, RelayLinkRequest, RelayLinkResponse,
            RelayRequestInventoryMessage, RelayShareWorkPayload, RelayUnlinkResponse,
            RelayWelcomeMessage, ShareProfileRequest, ShareProfileResponse, ShareWorkRequest,
            ShareWorkResponse, build_relay_socket_url, relay_is_connected,
            service::{
                clear_relay_link, perform_relay_link, perform_relay_unlink, remember_relay_error,
                remember_relay_success, send_relay_inventory, share_profile_inner,
                share_work_inner,
            },
        },
        session::{
            BridgeSession, ConnectSessionRequest, ConnectSessionResponse, DisconnectSessionRequest,
            DisconnectSessionResponse, SessionSummary, service::validate_session,
        },
        system::{
            ArtistEntry, ArtistSummary, DiagnoseResponse, GatewayHealthResponse, HealthResponse,
            StorageSnapshot,
            probe::{detect_public_ipv4, gateway_health_probe},
            service::{build_config_response, build_storage_snapshot, notify_work_share_success},
        },
    },
    util::{
        data::unique_trimmed_strings,
        format::{format_bytes_human, format_timestamp},
        text::{csv_escape, escape_html, sanitize_custom_tag},
        url::{build_direct_ip_gateway_base_url, encode_query_component, trim_trailing_slash},
    },
};

use anyhow::{Context, anyhow};
use axum::{
    Form, Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path as AxumPath, Query, State},
    http::{
        StatusCode,
        header::{HeaderName, HeaderValue},
    },
    middleware,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt, stream};
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
use uuid::Uuid;

const VERIFY_CONCURRENCY: usize = 6;
const MAX_UPLOAD_BYTES: usize = 5 * 1024 * 1024 * 1024;

async fn add_private_network_access_header(mut response: Response) -> Response {
    response.headers_mut().insert(
        HeaderName::from_static("access-control-allow-private-network"),
        HeaderValue::from_static("true"),
    );
    response
}

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

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let (active_sessions, watched_pin_count, last_repair_cycle_at) = {
        let sessions = state.sessions.read().await;
        let persistent = state.persistent.read().await;
        (sessions.len(), persistent.watched_pins.len(), persistent.last_repair_cycle_at)
    };
    let (
        download_root_dir,
        sync_enabled,
        local_gateway_base_url,
        public_gateway_base_url,
        relay_enabled,
        relay_server_url,
        relay_device_name,
        relay_device_id,
        relay_device_label,
        relay_last_connected_at,
        relay_last_error,
        remote_pinning_enabled,
        onboarded,
    ) = {
        let config = state.config.read().await;
        (
            config.download_root_dir.clone(),
            config.sync_enabled,
            config.local_gateway_base_url.clone(),
            config.public_gateway_base_url.clone(),
            config.relay_enabled,
            config.relay_server_url.clone(),
            config.relay_device_name.clone(),
            config.relay_device_id.clone(),
            config.relay_device_label.clone(),
            config.relay_last_connected_at,
            config.relay_last_error.clone(),
            config.remote_pinning_enabled,
            config.onboarded_at.is_some(),
        )
    };
    let storage = build_storage_snapshot(&state).await;
    let operation = state.operation.read().await.clone();

    Json(HealthResponse {
        status: "ok",
        service: "foundation-share-bridge",
        ipfs_api_url: state.ipfs_api_url.clone(),
        state_file: state.state_file.display().to_string(),
        config_file: state.config_file.display().to_string(),
        active_sessions,
        watched_pin_count,
        repair_interval_seconds: state.repair_interval_seconds,
        last_repair_cycle_at,
        download_root_dir,
        sync_enabled,
        local_gateway_base_url,
        public_gateway_base_url,
        relay_enabled,
        relay_server_url,
        relay_device_name,
        relay_device_id,
        relay_device_label,
        relay_last_connected_at,
        relay_last_error,
        now: Utc::now(),
        storage,
        operation,
        remote_pinning_enabled,
        onboarded,
    })
}

async fn root_page(
    State(state): State<AppState>,
    Query(query): Query<RootPageQuery>,
) -> Result<Html<String>, AppError> {
    let persistent = state.persistent.read().await.clone();
    let sessions = state.sessions.read().await.clone();
    let config = state.config.read().await.clone();

    let selected_session = query.session_id.as_deref().and_then(|session_id| {
        sessions.values().find(|session| session.session_id == session_id).cloned()
    });

    let inventory = list_local_pin_inventory(&state).await.map_err(AppError::internal)?;

    let relay_connected = relay_is_connected(&config);
    let relay_server_value =
        query.relay_server_url.as_deref().unwrap_or(config.relay_server_url.as_str());
    let pairing_code_value = query.pairing_code.as_deref().unwrap_or("");
    let device_name_value = query
        .device_name
        .as_deref()
        .or(Some(config.relay_device_name.as_str()))
        .unwrap_or("Foundation desktop helper");
    let autolink_requested =
        query.autolink.as_deref() == Some("1") && !pairing_code_value.trim().is_empty();

    let connection_block = if relay_connected {
        format!(
            r#"<section id="connection" class="card">
  <p class="eyebrow">Archive relay</p>
  <h2>Connected</h2>
  <dl class="kv" style="margin-top: 14px;">
    <dt>Device</dt><dd>{device}</dd>
    <dt>Server</dt><dd>{server}</dd>
    <dt>Last connected</dt><dd>{last}</dd>
  </dl>
  <form action="/relay/unlink/form" method="post" class="btn-row">
    <button type="submit" class="btn ghost">Disconnect this app</button>
  </form>
</section>"#,
            device = escape_html(
                config
                    .relay_device_label
                    .as_deref()
                    .or(config.relay_device_id.as_deref())
                    .unwrap_or("Connected")
            ),
            server = escape_html(&config.relay_server_url),
            last = escape_html(
                &config
                    .relay_last_connected_at
                    .map(format_timestamp)
                    .unwrap_or_else(|| "not yet".to_string())
            ),
        )
    } else if autolink_requested {
        format!(
            r#"<section id="connection" class="card">
  <p class="eyebrow">Pair with archive</p>
  <h2>Finishing your connection…</h2>
  <p class="muted" style="margin-top: 10px;">This local helper page opened from the archive site. It will confirm the one-time pairing automatically so you can see the connection happen here instead of guessing in the background.</p>
  <dl class="kv" style="margin-top: 16px;">
    <dt>Archive server</dt><dd>{server}</dd>
    <dt>Desktop name</dt><dd>{name}</dd>
    <dt>Pairing code</dt><dd><code>{code}</code></dd>
  </dl>
  <form id="autolink-form" action="/relay/link/form" method="post" class="btn-row" style="margin-top: 24px;">
    <input type="hidden" name="relay_server_url" value="{server_attr}" />
    <input type="hidden" name="pairing_code" value="{code_attr}" />
    <input type="hidden" name="device_name" value="{name_attr}" />
    <button type="submit" class="btn">Finish connection now</button>
    <a class="btn ghost" href="/settings">Open settings</a>
  </form>
  <p class="muted" id="autolink-status" style="margin-top: 12px;">Waiting for this helper to confirm with the archive site…</p>
</section>
<script>{script}</script>"#,
            server = escape_html(relay_server_value),
            name = escape_html(device_name_value),
            code = escape_html(pairing_code_value),
            server_attr = escape_html(relay_server_value),
            code_attr = escape_html(pairing_code_value),
            name_attr = escape_html(device_name_value),
            script = ROOT_AUTOLINK_SCRIPT,
        )
    } else {
        format!(
            r#"<section id="connection" class="card">
  <p class="eyebrow">Pair with archive</p>
  <h2>Connect with a pairing code</h2>
  <p class="muted" style="margin-top: 10px;">Open the app link from the archive site, or paste the pairing details here. The socket only stays active after this link is confirmed.</p>
  <form action="/relay/link/form" method="post">
    <label class="field">
      <span>Archive server URL</span>
      <input name="relay_server_url" value="{server}" placeholder="https://archive.example.com" />
    </label>
    <label class="field">
      <span>Pairing code</span>
      <input name="pairing_code" value="{code}" placeholder="ABCD1234" />
    </label>
    <label class="field">
      <span>Desktop name</span>
      <input name="device_name" value="{name}" placeholder="Studio MacBook" />
    </label>
    <div class="btn-row">
      <button type="submit" class="btn">Link this app</button>
    </div>
  </form>
</section>"#,
            server = escape_html(relay_server_value),
            code = escape_html(pairing_code_value),
            name = escape_html(device_name_value),
        )
    };

    let flash_block = if query.linked.as_deref() == Some("1") {
        r#"<div class="flash ok">Archive relay connected. This desktop app can now receive live pin jobs.</div>"#.to_string()
    } else if query.unlinked.as_deref() == Some("1") {
        r#"<div class="flash warn">Archive relay disconnected on this desktop app.</div>"#
            .to_string()
    } else if let Some(error) = query.error.as_deref() {
        format!(r#"<div class="flash err">{}</div>"#, escape_html(error))
    } else {
        String::new()
    };

    let session_block = selected_session
        .map(|session| {
            format!(
                r#"<section class="card">
  <p class="eyebrow">Session</p>
  <h2>{id}</h2>
  <dl class="kv" style="margin-top: 14px;">
    <dt>Origin</dt><dd>{origin}</dd>
    <dt>Started</dt><dd>{started}</dd>
  </dl>
</section>"#,
                id = escape_html(&session.session_id),
                origin = escape_html(&session.website_origin),
                started = escape_html(&format_timestamp(session.connected_at))
            )
        })
        .unwrap_or_default();

    let connection_status = if relay_connected { "Live" } else { "Not linked" };
    let connection_pill_class = if relay_connected { "pill ok" } else { "pill" };

    let inventory_body = if inventory.items.is_empty() {
        r#"<div class="empty">No pins yet. Once the archive site hands you something to rescue, it will appear here.</div>"#.to_string()
    } else {
        let fallback_table = render_inventory_fallback_table(&inventory.items);
        format!(
            r#"<div class="inventory-browser-head">
  <p class="muted">Live previews load {page_size} pins at a time so the bridge doesn&apos;t hit every gateway all at once.</p>
</div>
<div id="inventory-browser" class="inventory-browser" data-page-size="{page_size}">
  <div id="inventory-grid" class="pin-grid" aria-live="polite"></div>
  <div id="inventory-empty" class="empty" hidden>No pins are available right now.</div>
  <div class="inventory-load-row">
    <button type="button" id="inventory-load-more" class="btn ghost" hidden>Load more pins</button>
    <p id="inventory-status" class="muted inventory-status">Loading previews…</p>
  </div>
  <div id="inventory-sentinel" class="inventory-sentinel" aria-hidden="true"></div>
</div>
<noscript>{fallback}</noscript>
<script>{script}</script>"#,
            page_size = INVENTORY_PAGE_SIZE,
            fallback = fallback_table,
            script = INVENTORY_BROWSER_SCRIPT,
        )
    };

    let pinned_count = inventory.pinned_count;
    let managed_count = inventory.managed_count;
    let repair_interval = state.repair_interval_seconds;
    let last_repair = persistent
        .last_repair_cycle_at
        .map(format_timestamp)
        .unwrap_or_else(|| "never".to_string());

    let storage_snapshot = build_storage_snapshot(&state).await;
    let disk_used = match storage_snapshot.repo_size_bytes {
        Some(bytes) => format_bytes_human(bytes),
        None => "—".to_string(),
    };
    let disk_body = match (storage_snapshot.quota_gb, storage_snapshot.quota_used_fraction) {
        (Some(gb), Some(fraction)) => {
            format!("Quota {:.1} GB · {}% used", gb, (fraction * 100.0).round() as i64)
        }
        _ => {
            if storage_snapshot.ipfs_daemon_reachable {
                "Reported by the Kubo repo/stat API.".to_string()
            } else {
                "IPFS daemon not reachable — start Kubo to see usage.".to_string()
            }
        }
    };

    let pending_failures =
        persistent.watched_pins.values().filter(|pin| pin.last_error.is_some()).count();
    let final_failures = persistent
        .watched_pins
        .values()
        .filter(|pin| pin.final_failure_reported_at.is_some())
        .count();
    let failure_banner = if pending_failures == 0 {
        String::new()
    } else {
        let cls = if final_failures > 0 { "flash err" } else { "flash warn" };
        let copy = if final_failures > 0 {
            format!(
                "{pending_failures} pin{} report errors right now, and {final_failures} have exhausted their retry budget. Open a card to diagnose or retry sooner.",
                if pending_failures == 1 { "" } else { "s" }
            )
        } else {
            format!(
                "{pending_failures} pin{} are waiting for a retry. Open a card to diagnose or retry sooner.",
                if pending_failures == 1 { "" } else { "s" }
            )
        };
        format!(r#"<div class="{cls}">{}</div>"#, escape_html(&copy))
    };

    let artist_summary_html = {
        let sessions_guard = state.sessions.read().await;
        render_artist_summary(&persistent, &sessions_guard)
    };
    let gateway_card = render_gateway_card(&config);
    let export_card = render_export_card();
    let live_status_block = {
        let op_guard = state.operation.read().await;
        render_live_status_panel(&op_guard)
    };

    let body = format!(
        r##"<main class="shell">
  <div class="stack">
    <section class="section-head">
      <p class="eyebrow">Agorix · Share bridge</p>
      <h1>Keep rescued IPFS roots pinned and self-repaired.</h1>
      <p class="lead">This local companion app for the Agorix Foundation archive keeps a memory of watched CIDs, re-checks them forever, and re-pins anything your IPFS node drops. Pair it with the archive site once, then leave it running.</p>
      <div class="btn-row">
        <a class="pill {conn_pill}" href="#connection">{conn_status}</a>
        <span class="pill">{repair_interval}s repair cadence</span>
        <a class="btn ghost" href="/settings">Open settings</a>
      </div>
    </section>

    {flash}
    {failure_banner}

    {live_status_block}

    <section id="status">
      <div class="stats">
        <div class="stat">
          <p class="eyebrow">Pinned now</p>
          <p class="stat-value">{pinned}</p>
          <p class="stat-body">Currently present in your local IPFS node.</p>
        </div>
        <div class="stat">
          <p class="eyebrow">Managed forever</p>
          <p class="stat-value">{managed}</p>
          <p class="stat-body">Watched roots this app will keep repairing.</p>
        </div>
        <div class="stat">
          <p class="eyebrow">Disk used</p>
          <p class="stat-value" style="font-size: 1.4rem;">{disk_used}</p>
          <p class="stat-body">{disk_body}</p>
        </div>
        <div class="stat">
          <p class="eyebrow">Last repair</p>
          <p class="stat-value" style="font-size: 1rem; font-family: ui-monospace, Menlo, Consolas, monospace;">{last_repair}</p>
          <p class="stat-body">{repair_interval}s cadence · missing pins are restored on the next cycle.</p>
        </div>
      </div>
    </section>

    <section class="two-col">
      {connection}
      {session}
    </section>

    {artist_summary_html}

    <section id="inventory">
      <div class="section-head" style="border-bottom: 0; padding-bottom: 0;">
        <p class="eyebrow">Local inventory</p>
        <h2 style="margin-top: 8px;">Everything this node has pinned</h2>
        <p class="lead">Foundation-linked roots keep their rescue context. Each card now shows retry state, provider count, and action buttons to diagnose or retry a pin individually.</p>
      </div>
      <div style="margin-top: 20px;">{inventory_body}</div>
    </section>

    <section class="two-col">
      {gateway_card}
      {export_card}
    </section>

    <p class="footer">Agorix share bridge · local-only · {repair_interval}s repair interval · last cycle {last_repair}</p>
  </div>
</main>
<script>{live_status_script}</script>"##,
        conn_pill = connection_pill_class,
        conn_status = connection_status,
        pinned = pinned_count,
        managed = managed_count,
        repair_interval = repair_interval,
        last_repair = escape_html(&last_repair),
        disk_used = escape_html(&disk_used),
        disk_body = escape_html(&disk_body),
        flash = flash_block,
        failure_banner = failure_banner,
        live_status_block = live_status_block,
        connection = connection_block,
        session = session_block,
        artist_summary_html = artist_summary_html,
        inventory_body = inventory_body,
        gateway_card = gateway_card,
        export_card = export_card,
        live_status_script = LIVE_STATUS_SCRIPT,
    );

    Ok(Html(render_page("Foundation Share Bridge", &body)))
}

async fn settings_page(
    State(state): State<AppState>,
    Query(query): Query<SettingsPageQuery>,
) -> Result<Html<String>, AppError> {
    let config = state.config.read().await.clone();
    let relay_connected = relay_is_connected(&config);
    let relay_status_label = if relay_connected {
        "Connected"
    } else if config.relay_enabled {
        "Waiting to link"
    } else {
        "Not linked"
    };
    let relay_status_class = if relay_connected { "pill ok" } else { "pill" };
    let sync_checked = if config.sync_enabled { "checked" } else { "" };
    let relay_checked = if config.relay_enabled { "checked" } else { "" };
    let remote_pinning_checked = if config.remote_pinning_enabled { "checked" } else { "" };
    let storage_quota_display =
        config.storage_quota_gb.map(|value| format!("{value}")).unwrap_or_default();
    let max_retry_attempts_display =
        config.max_retry_attempts.map(|value| format!("{value}")).unwrap_or_default();
    let remote_pinning_service_name_display =
        config.remote_pinning_service_name.clone().unwrap_or_default();
    let remote_pinning_service_url_display =
        config.remote_pinning_service_url.clone().unwrap_or_default();
    let token_saved = config
        .remote_pinning_access_token
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let (token_badge, token_placeholder) = if token_saved {
        (
            r#"<span class="token-badge saved" title="A token is saved">saved</span>"#,
            "•••••••• leave blank to keep",
        )
    } else {
        (r#"<span class="token-badge empty" title="No token saved">empty</span>"#, "Paste token")
    };

    let flash_block = if query.saved.as_deref() == Some("1") {
        r#"<div class="flash ok">Settings saved. The helper updated its YAML config file for you.</div>"#
            .to_string()
    } else if let Some(error) = query.error.as_deref() {
        format!(r#"<div class="flash err">{}</div>"#, escape_html(error))
    } else {
        String::new()
    };

    let relay_note = config
        .relay_last_error
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|message| {
            format!(r#"<div class="flash warn">Relay note: {}</div>"#, escape_html(message))
        })
        .unwrap_or_default();

    let linked_device = config
        .relay_device_label
        .as_deref()
        .or(config.relay_device_id.as_deref())
        .unwrap_or("None yet");
    let linked_at = config
        .relay_last_connected_at
        .map(format_timestamp)
        .unwrap_or_else(|| "Not linked yet".to_string());
    let yaml_path = state.config_file.display().to_string();
    let detected_public_ipv4 = detect_public_ipv4(&state).await;
    let current_external_gateway = escape_html(&config.public_gateway_base_url);

    let gateway_helper = {
        let detected_ip_line = detected_public_ipv4
            .as_deref()
            .map(|ip| {
                format!(
                    r#"<div class="gw-detected">Detected public IP <code>{ip}</code> · <button type="button" class="gw-link" id="gateway_fill_ip" data-gateway-url="{url}">Use IP directly</button></div>"#,
                    ip = escape_html(ip),
                    url = escape_html(&build_direct_ip_gateway_base_url(ip)),
                )
            })
            .unwrap_or_else(|| r#"<div class="gw-detected muted">Public IPv4 not detected.</div>"#.to_string());

        format!(
            r#"<details class="gw-helper">
  <summary>Help me build this URL</summary>
  <div class="gw-helper-body">
    <div class="gw-row">
      <input type="text" id="gateway_hostname_input" placeholder="ipfs.example.com" />
      <button type="button" class="btn ghost" id="gateway_fill_hostname">Use hostname</button>
    </div>
    {detected_ip_line}
    <div class="gw-preview">Preview · <code id="gateway_helper_preview_value">{current_external_gateway}</code></div>
  </div>
</details>"#,
            detected_ip_line = detected_ip_line,
            current_external_gateway = current_external_gateway,
        )
    };

    let _ = (linked_device, linked_at, yaml_path);

    let body = format!(
        r#"<main class="shell narrow settings-shell">
  <div class="stack">
    <header class="settings-head">
      <div>
        <p class="eyebrow">Settings</p>
        <h1>Bridge preferences</h1>
      </div>
      <div class="settings-head-meta">
        <span class="{relay_class}">{relay_label}</span>
        <a class="btn ghost" href="/">← Back</a>
      </div>
    </header>

    {flash}
    {relay_note}

    <form action="/settings/form" method="post" class="settings-form-v2" id="settings-form-v2">
      <section class="settings-card">
        <h2>Storage</h2>
        <div class="settings-field">
          <label for="field_download_root_dir">Download folder</label>
          <input type="text" id="field_download_root_dir" name="download_root_dir" value="{download_root_dir}" placeholder="/Users/you/Archive Pins" spellcheck="false" />
        </div>
        <div class="settings-row">
          <div class="settings-row-text">
            <strong>Keep synced copies on disk</strong>
            <span>Mirror each pin into the download folder.</span>
          </div>
          <label class="toggle" aria-label="Keep synced copies on disk">
            <input type="checkbox" name="sync_enabled" value="1" {sync_checked} />
            <span class="toggle-track"><span class="toggle-thumb"></span></span>
          </label>
        </div>
        <div class="settings-pair">
          <div class="settings-field">
            <label for="field_storage_quota_gb">Quota (GB)</label>
            <div class="num-stepper">
              <button type="button" data-step="-1" aria-label="Decrease">−</button>
              <input type="number" id="field_storage_quota_gb" step="0.1" min="0" name="storage_quota_gb" value="{storage_quota_gb}" placeholder="none" inputmode="decimal" />
              <button type="button" data-step="1" aria-label="Increase">+</button>
            </div>
          </div>
          <div class="settings-field">
            <label for="field_max_retry_attempts">Max retries</label>
            <div class="num-stepper">
              <button type="button" data-step="-1" aria-label="Decrease">−</button>
              <input type="number" id="field_max_retry_attempts" step="1" min="1" max="20" name="max_retry_attempts" value="{max_retry_attempts}" placeholder="10" inputmode="numeric" />
              <button type="button" data-step="1" aria-label="Increase">+</button>
            </div>
          </div>
        </div>
      </section>

      <section class="settings-card">
        <h2>Gateways</h2>
        <div class="settings-field">
          <label for="field_local_gateway_base_url">Local gateway</label>
          <input type="url" id="field_local_gateway_base_url" name="local_gateway_base_url" value="{local_gateway_base_url}" placeholder="http://127.0.0.1:8080" spellcheck="false" />
        </div>
        <div class="settings-field">
          <label for="public_gateway_base_url">External gateway</label>
          <input type="url" id="public_gateway_base_url" name="public_gateway_base_url" value="{public_gateway_base_url}" placeholder="https://ipfs.example.com" spellcheck="false" />
        </div>
        {gateway_helper}
      </section>

      <section class="settings-card">
        <h2>Remote pin fallback</h2>
        <div class="settings-row">
          <div class="settings-row-text">
            <strong>Enable remote fallback</strong>
            <span>Used only after local retries are exhausted.</span>
          </div>
          <label class="toggle" aria-label="Enable remote pin fallback">
            <input type="checkbox" name="remote_pinning_enabled" value="1" {remote_pinning_checked} />
            <span class="toggle-track"><span class="toggle-thumb"></span></span>
          </label>
        </div>
        <div class="settings-pair">
          <div class="settings-field">
            <label for="field_remote_pinning_service_name">Service name</label>
            <input type="text" id="field_remote_pinning_service_name" name="remote_pinning_service_name" value="{remote_pinning_service_name}" placeholder="Pinata" spellcheck="false" />
          </div>
          <div class="settings-field">
            <label for="field_remote_pinning_service_url">API base URL</label>
            <input type="url" id="field_remote_pinning_service_url" name="remote_pinning_service_url" value="{remote_pinning_service_url}" placeholder="https://api.pinata.cloud/psa" spellcheck="false" />
          </div>
        </div>
        <div class="settings-field">
          <label for="field_remote_pinning_access_token">Access token {token_badge}</label>
          <div class="password-field">
            <input type="password" id="field_remote_pinning_access_token" name="remote_pinning_access_token" value="" placeholder="{token_placeholder}" autocomplete="off" spellcheck="false" />
            <button type="button" class="password-reveal" data-reveal>Show</button>
          </div>
        </div>
      </section>

      <section class="settings-card">
        <h2>Archive relay</h2>
        <div class="settings-row">
          <div class="settings-row-text">
            <strong>Enable relay link</strong>
            <span>Lets the archive site hand work to this helper.</span>
          </div>
          <label class="toggle" aria-label="Enable archive relay link">
            <input type="checkbox" name="relay_enabled" value="1" {relay_checked} />
            <span class="toggle-track"><span class="toggle-thumb"></span></span>
          </label>
        </div>
        <div class="settings-pair">
          <div class="settings-field">
            <label for="field_relay_server_url">Archive server URL</label>
            <input type="url" id="field_relay_server_url" name="relay_server_url" value="{relay_server_url}" placeholder="https://foundation.agorix.io" spellcheck="false" />
          </div>
          <div class="settings-field">
            <label for="field_relay_device_name">Desktop name</label>
            <input type="text" id="field_relay_device_name" name="relay_device_name" value="{relay_device_name}" placeholder="Studio MacBook" />
          </div>
        </div>
      </section>

      <div class="settings-save-bar" id="settings-save-bar">
        <span class="settings-save-hint" id="settings-save-hint">All changes saved.</span>
        <button type="submit" class="btn">Save settings</button>
      </div>
    </form>
  </div>
</main>
<style>{settings_css}</style>
<script>{settings_gateway_script}</script>
<script>{settings_controls_script}</script>"#,
        relay_class = relay_status_class,
        relay_label = escape_html(relay_status_label),
        flash = flash_block,
        relay_note = relay_note,
        download_root_dir = escape_html(&config.download_root_dir),
        sync_checked = sync_checked,
        storage_quota_gb = escape_html(&storage_quota_display),
        max_retry_attempts = escape_html(&max_retry_attempts_display),
        remote_pinning_checked = remote_pinning_checked,
        remote_pinning_service_name = escape_html(&remote_pinning_service_name_display),
        remote_pinning_service_url = escape_html(&remote_pinning_service_url_display),
        token_badge = token_badge,
        token_placeholder = token_placeholder,
        local_gateway_base_url = escape_html(&config.local_gateway_base_url),
        public_gateway_base_url = escape_html(&config.public_gateway_base_url),
        relay_checked = relay_checked,
        relay_server_url = escape_html(&config.relay_server_url),
        relay_device_name = escape_html(&config.relay_device_name),
        gateway_helper = gateway_helper,
        settings_css = SETTINGS_PAGE_STYLE,
        settings_gateway_script = SETTINGS_GATEWAY_HELPER_SCRIPT,
        settings_controls_script = SETTINGS_CONTROLS_SCRIPT,
    );

    Ok(Html(render_page("Bridge settings", &body)))
}

async fn list_sessions(
    State(state): State<AppState>,
) -> Result<Json<Vec<SessionSummary>>, AppError> {
    let sessions = state.sessions.read().await;
    let data = sessions
        .values()
        .map(|session| SessionSummary {
            session_id: session.session_id.clone(),
            website_origin: session.website_origin.clone(),
            account_address: session.account_address.clone(),
            profile_username: session.profile_username.clone(),
            client_name: session.client_name.clone(),
            connected_at: session.connected_at,
        })
        .collect();

    Ok(Json(data))
}

async fn session_by_id(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<SessionSummary>, AppError> {
    let sessions = state.sessions.read().await;
    let session = sessions
        .values()
        .find(|session| session.session_id == session_id)
        .ok_or_else(|| AppError::bad_request("Session was not found"))?;

    Ok(Json(SessionSummary {
        session_id: session.session_id.clone(),
        website_origin: session.website_origin.clone(),
        account_address: session.account_address.clone(),
        profile_username: session.profile_username.clone(),
        client_name: session.client_name.clone(),
        connected_at: session.connected_at,
    }))
}

async fn get_config(State(state): State<AppState>) -> Result<Json<BridgeConfigResponse>, AppError> {
    let config = state.config.read().await;
    Ok(Json(build_config_response(&state, &config)))
}

async fn update_config(
    State(state): State<AppState>,
    Json(input): Json<UpdateBridgeConfigRequest>,
) -> Result<Json<BridgeConfigResponse>, AppError> {
    let updated = apply_config_update(&state, input).await?;
    Ok(Json(updated))
}

async fn update_config_form(
    State(state): State<AppState>,
    Form(input): Form<UpdateBridgeConfigFormRequest>,
) -> Result<Redirect, AppError> {
    let quota = input.storage_quota_gb.as_deref().map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            trimmed.parse::<f64>().ok().filter(|value| *value > 0.0)
        }
    });

    let retries = input.max_retry_attempts.as_deref().map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() { None } else { trimmed.parse::<u32>().ok() }
    });

    let name = input.remote_pinning_service_name.as_deref().map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
    });

    let url = input.remote_pinning_service_url.as_deref().map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
    });

    let token = input.remote_pinning_access_token.as_deref().map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
    });

    let request = UpdateBridgeConfigRequest {
        download_root_dir: Some(input.download_root_dir),
        sync_enabled: Some(input.sync_enabled.is_some()),
        local_gateway_base_url: Some(input.local_gateway_base_url),
        public_gateway_base_url: Some(input.public_gateway_base_url),
        relay_enabled: Some(input.relay_enabled.is_some()),
        relay_server_url: Some(input.relay_server_url),
        relay_device_name: Some(input.relay_device_name),
        storage_quota_gb: quota,
        max_retry_attempts: retries,
        remote_pinning_enabled: Some(input.remote_pinning_enabled.is_some()),
        remote_pinning_service_name: name,
        remote_pinning_service_url: url,
        remote_pinning_access_token: token,
    };

    match apply_config_update(&state, request).await {
        Ok(_) => Ok(Redirect::to("/settings?saved=1")),
        Err(error) => {
            Ok(Redirect::to(&format!("/settings?error={}", encode_query_component(&error.message))))
        }
    }
}

async fn link_relay_device(
    State(state): State<AppState>,
    Json(input): Json<RelayLinkRequest>,
) -> Result<Json<RelayLinkResponse>, AppError> {
    let payload = perform_relay_link(&state, input).await?;
    Ok(Json(payload))
}

async fn link_relay_device_form(
    State(state): State<AppState>,
    Form(input): Form<RelayLinkFormRequest>,
) -> Result<Redirect, AppError> {
    let redirect_relay_server_url = input.relay_server_url.clone();
    let redirect_pairing_code = input.pairing_code.clone();
    let redirect_device_name = input.device_name.clone();

    match perform_relay_link(
        &state,
        RelayLinkRequest {
            relay_server_url: Some(input.relay_server_url),
            pairing_code: input.pairing_code,
            device_name: input.device_name,
        },
    )
    .await
    {
        Ok(_) => Ok(Redirect::to("/?linked=1")),
        Err(error) => Ok(Redirect::to(&format!(
            "/?error={}&relay_server_url={}&pairing_code={}&device_name={}",
            encode_query_component(&error.message),
            encode_query_component(&redirect_relay_server_url),
            encode_query_component(&redirect_pairing_code),
            encode_query_component(redirect_device_name.as_deref().unwrap_or("")),
        ))),
    }
}

async fn unlink_relay_device(
    State(state): State<AppState>,
) -> Result<Json<RelayUnlinkResponse>, AppError> {
    perform_relay_unlink(&state, true).await.map_err(AppError::internal)?;

    Ok(Json(RelayUnlinkResponse { unlinked: true }))
}

async fn unlink_relay_device_form(State(state): State<AppState>) -> Result<Redirect, AppError> {
    perform_relay_unlink(&state, true).await.map_err(AppError::internal)?;

    Ok(Redirect::to("/?unlinked=1"))
}

async fn connect_session(
    State(state): State<AppState>,
    Json(input): Json<ConnectSessionRequest>,
) -> Result<Json<ConnectSessionResponse>, AppError> {
    if input.website_origin.trim().is_empty() {
        return Err(AppError::bad_request("website_origin is required"));
    }

    let session = BridgeSession {
        session_id: Uuid::new_v4().to_string(),
        session_secret: Uuid::new_v4().to_string(),
        website_origin: input.website_origin.trim().to_string(),
        account_address: input.account_address.filter(|value| !value.trim().is_empty()),
        profile_username: input.profile_username.filter(|value| !value.trim().is_empty()),
        client_name: input.client_name.filter(|value| !value.trim().is_empty()),
        connected_at: Utc::now(),
    };

    let mut sessions = state.sessions.write().await;
    sessions.insert(session.session_secret.clone(), session.clone());

    Ok(Json(ConnectSessionResponse {
        session,
        message: "Session connected. The website can now hand work or profile share requests to the local bridge.",
    }))
}

async fn disconnect_session(
    State(state): State<AppState>,
    Json(input): Json<DisconnectSessionRequest>,
) -> Result<Json<DisconnectSessionResponse>, AppError> {
    let mut sessions = state.sessions.write().await;
    let removed = sessions.remove(&input.session_secret).is_some();

    Ok(Json(DisconnectSessionResponse { disconnected: removed }))
}

async fn list_pins(State(state): State<AppState>) -> Result<Json<PinsResponse>, AppError> {
    let response = list_local_pin_inventory(&state).await.map_err(AppError::internal)?;
    Ok(Json(response))
}

async fn list_pins_page(
    State(state): State<AppState>,
    Query(query): Query<PinsPageQuery>,
) -> Result<Json<PinsPageResponse>, AppError> {
    let cursor = parse_inventory_cursor(query.cursor.as_deref());
    let limit = resolve_inventory_page_size(query.limit);
    let response =
        list_local_pin_inventory_page(&state, cursor, limit).await.map_err(AppError::internal)?;
    Ok(Json(response))
}

async fn repair_now(State(state): State<AppState>) -> Result<Json<RepairNowResponse>, AppError> {
    let outcome = repair_watched_pins(&state).await.map_err(AppError::internal)?;

    Ok(Json(RepairNowResponse {
        repaired: outcome.repaired,
        healthy: outcome.healthy,
        failed: outcome.failed,
        message: "Repair cycle completed.",
    }))
}

async fn verify_pins(
    State(state): State<AppState>,
    Json(input): Json<VerifyPinsRequest>,
) -> Result<Json<VerifyPinsResponse>, AppError> {
    let targets = resolve_verify_targets(&state, input.cids.as_deref()).await;
    let mut results = stream::iter(targets.into_iter().enumerate().map(|(index, cid)| {
        let state = state.clone();
        async move { (index, check_cid_network_providers(&state, &cid).await) }
    }))
    .buffer_unordered(VERIFY_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;
    results.sort_by_key(|(index, _)| *index);

    let mut ordered_results = Vec::with_capacity(results.len());
    for (_, result) in results {
        remember_pin_verification(&state, &result).await?;
        ordered_results.push(result);
    }

    Ok(Json(VerifyPinsResponse { checked_at: Utc::now(), results: ordered_results }))
}

async fn unwatch_pins(
    State(state): State<AppState>,
    Json(input): Json<UnwatchPinsRequest>,
) -> Result<Json<UnwatchPinsResponse>, AppError> {
    let cids = unique_trimmed_strings(input.cids);
    if cids.is_empty() {
        return Err(AppError::bad_request(
            "Provide at least one CID to remove from the forever-watch list.",
        ));
    }

    let mut removed = 0_usize;
    let mut missing = 0_usize;
    {
        let mut persistent = state.persistent.write().await;
        persistent.updated_at = Some(Utc::now());

        for cid in cids {
            if persistent.watched_pins.remove(&cid).is_some() {
                removed += 1;
            } else {
                missing += 1;
            }
        }
    }

    persist_bridge_state(&state).await.map_err(AppError::internal)?;

    Ok(Json(UnwatchPinsResponse {
        removed,
        missing,
        message: "Removed these roots from the forever-watch list. Existing IPFS pins were left alone.",
    }))
}

async fn verify_single_pin(
    AxumPath(cid): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<PinVerification>, AppError> {
    let result = check_cid_network_providers(&state, &cid).await;
    remember_pin_verification(&state, &result).await?;
    Ok(Json(result))
}

async fn sync_now(State(state): State<AppState>) -> Result<Json<SyncNowResponse>, AppError> {
    let outcome = sync_all_watched_pins(&state, true).await.map_err(AppError::internal)?;

    Ok(Json(SyncNowResponse {
        synced: outcome.synced,
        failed: outcome.failed,
        skipped: outcome.skipped,
        message: "Sync cycle completed.",
    }))
}

async fn pin_cid(
    State(state): State<AppState>,
    Json(input): Json<PinCidRequest>,
) -> Result<Json<PinCidResult>, AppError> {
    if let Some(secret) = input.session_secret.as_deref() {
        validate_session(&state, secret).await?;
    }

    let result = pin_and_watch_cid(
        &state,
        WatchPinInput {
            cid: input.cid.clone(),
            label: input.label.clone(),
            preferred_file_name: None,
            source_kind: "manual".to_string(),
            title: None,
            contract_address: None,
            token_id: None,
            foundation_url: None,
            artist_username: None,
            account_address: None,
            username: None,
        },
    )
    .await?;

    Ok(Json(result))
}

async fn add_files(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<AddFilesResult>, AppError> {
    let mut session_secret: Option<String> = None;
    let mut label: Option<String> = None;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total_bytes: u64 = 0;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| AppError::bad_request(format!("Unable to read upload: {error}")))?
    {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "session_secret" => {
                let value = field.text().await.map_err(|error| {
                    AppError::bad_request(format!("Bad session_secret: {error}"))
                })?;
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    session_secret = Some(trimmed.to_string());
                }
            }
            "label" => {
                let value = field
                    .text()
                    .await
                    .map_err(|error| AppError::bad_request(format!("Bad label: {error}")))?;
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    label = Some(trimmed.to_string());
                }
            }
            "file" | "files" => {
                let filename = field
                    .file_name()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "file".to_string());
                let bytes = field.bytes().await.map_err(|error| {
                    AppError::bad_request(format!("Upload read failed: {error}"))
                })?;
                total_bytes = total_bytes.saturating_add(bytes.len() as u64);
                files.push((filename, bytes.to_vec()));
            }
            _ => {
                // Drain unknown fields so the body is fully consumed.
                let _ = field.bytes().await;
            }
        }
    }

    if let Some(secret) = session_secret.as_deref() {
        validate_session(&state, secret).await?;
    }

    if files.is_empty() {
        return Err(AppError::bad_request(
            "At least one file is required. Use form field name `file` or `files`.",
        ));
    }

    let wrap = files.len() > 1 || files.iter().any(|(name, _)| name.contains('/'));

    let mut form = reqwest::multipart::Form::new();
    for (filename, bytes) in files.drain(..) {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename)
            .mime_str("application/octet-stream")
            .map_err(|error| AppError::internal(anyhow!("Bad upload part: {error}")))?;
        form = form.part("file", part);
    }

    let endpoint = format!(
        "{}/api/v0/add?pin=true{}",
        state.ipfs_api_url.trim_end_matches('/'),
        if wrap { "&wrap-with-directory=true" } else { "" }
    );

    let mut request = state.http.post(endpoint);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request
        .multipart(form)
        .send()
        .await
        .map_err(|error| AppError::internal(anyhow!("Failed to reach IPFS API: {error}")))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::internal(anyhow!(
            "IPFS add failed with status {}: {}",
            status,
            body
        )));
    }

    let body_text = response
        .text()
        .await
        .map_err(|error| AppError::internal(anyhow!("Unable to read IPFS response: {error}")))?;

    let mut entries: Vec<AddedFileEntry> = Vec::new();
    for line in body_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(trimmed).map_err(|error| {
            AppError::internal(anyhow!("IPFS returned malformed line: {error}"))
        })?;
        let name = value.get("Name").and_then(|value| value.as_str()).unwrap_or("").to_string();
        let cid = value.get("Hash").and_then(|value| value.as_str()).unwrap_or("").to_string();
        if cid.is_empty() {
            continue;
        }
        let size = value
            .get("Size")
            .and_then(|value| value.as_str())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        entries.push(AddedFileEntry { name, cid, size });
    }

    if entries.is_empty() {
        return Err(AppError::internal(anyhow!("IPFS add returned no entries")));
    }

    let root_cid = if wrap {
        entries
            .iter()
            .find(|entry| entry.name.is_empty())
            .map(|entry| entry.cid.clone())
            .unwrap_or_else(|| entries.last().map(|entry| entry.cid.clone()).unwrap_or_default())
    } else {
        entries.last().map(|entry| entry.cid.clone()).unwrap_or_default()
    };

    if root_cid.is_empty() {
        return Err(AppError::internal(anyhow!("IPFS add did not return a root CID")));
    }

    let file_count = entries.iter().filter(|entry| !entry.name.is_empty()).count();
    let file_count = if file_count == 0 { entries.len() } else { file_count };

    let derived_label = label.clone().or_else(|| {
        if wrap {
            entries.iter().find(|entry| entry.name.is_empty()).and_then(|entry| {
                entries.iter().find(|inner| !inner.name.is_empty()).map(|inner| {
                    inner.name.split('/').next().unwrap_or(entry.cid.as_str()).to_string()
                })
            })
        } else {
            entries.iter().find(|entry| !entry.name.is_empty()).map(|entry| entry.name.clone())
        }
    });

    let preferred_file_name = if !wrap {
        entries.iter().find(|entry| !entry.name.is_empty()).map(|entry| entry.name.clone())
    } else {
        None
    };

    remember_watched_pin(
        &state,
        WatchPinInput {
            cid: root_cid.clone(),
            label: derived_label.clone(),
            preferred_file_name,
            source_kind: "upload".to_string(),
            title: None,
            contract_address: None,
            token_id: None,
            foundation_url: None,
            artist_username: None,
            account_address: None,
            username: None,
        },
        Some(root_cid.clone()),
        None,
        true,
    )
    .await?;

    if let Err(error) = sync_cid_if_enabled(&state, &root_cid).await {
        warn!("sync after upload failed for {}: {}", root_cid, error);
    }

    Ok(Json(AddFilesResult {
        root_cid: root_cid.clone(),
        label: derived_label,
        pinned: true,
        provider: "kubo",
        pin_reference: root_cid,
        requested_at: Utc::now(),
        file_count,
        total_bytes,
        wrapped: wrap,
        entries,
    }))
}

async fn share_work(
    State(state): State<AppState>,
    Json(input): Json<ShareWorkRequest>,
) -> Result<Json<ShareWorkResponse>, AppError> {
    let response = share_work_inner(&state, input).await?;
    Ok(Json(response))
}

async fn share_work_view(
    Query(query): Query<ShareWorkViewQuery>,
) -> Result<Html<String>, AppError> {
    let mut detail_rows = String::new();
    if let Some(cid) = query.metadata_cid.as_deref().filter(|cid| !cid.is_empty()) {
        detail_rows.push_str(&format!(
            r#"<li><span class="muted">Metadata</span><code>{}</code></li>"#,
            escape_html(cid)
        ));
    }
    if let Some(cid) = query.media_cid.as_deref().filter(|cid| !cid.is_empty()) {
        detail_rows.push_str(&format!(
            r#"<li><span class="muted">Media</span><code>{}</code></li>"#,
            escape_html(cid)
        ));
    }

    let details_block = if detail_rows.is_empty() {
        String::new()
    } else {
        format!(r#"<ul class="plain" style="margin-top: 16px;">{}</ul>"#, detail_rows)
    };

    let artist = query
        .artist_username
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown artist");

    let body = format!(
        r#"<main class="shell narrow">
  <div class="stack">
    <section class="section-head">
      <p class="eyebrow">Rescue handoff</p>
      <h1>Pin this rescued Foundation work.</h1>
      <p class="lead">Once you confirm, the bridge pins the rescued roots now and keeps watching them for self-repair later.</p>
    </section>

    <section class="card">
      <p class="eyebrow">Work</p>
      <h2 style="margin-top: 8px;">{title}</h2>
      <p class="muted" style="margin-top: 8px;">{artist} · token #{token}</p>
      {details}

      <form method="post" action="/share/work/form" class="btn-row" style="margin-top: 24px;">
        <input type="hidden" name="session_secret" value="{secret}" />
        <input type="hidden" name="title" value="{title_h}" />
        <input type="hidden" name="contract_address" value="{contract}" />
        <input type="hidden" name="token_id" value="{token_h}" />
        <input type="hidden" name="foundation_url" value="{foundation}" />
        <input type="hidden" name="artist_username" value="{artist_h}" />
        <input type="hidden" name="metadata_cid" value="{meta}" />
        <input type="hidden" name="media_cid" value="{media}" />
        <button type="submit" class="btn">Pin and keep watching forever</button>
        <a class="btn ghost" href="/">Cancel</a>
      </form>
    </section>
  </div>
</main>
<style>
  ul.plain li {{ display: grid; grid-template-columns: 120px 1fr; align-items: center; gap: 16px; }}
  ul.plain li .muted {{
    font-family: ui-monospace, Menlo, Consolas, monospace;
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.22em;
  }}
</style>"#,
        title = escape_html(&query.title),
        artist = escape_html(artist),
        token = escape_html(&query.token_id),
        details = details_block,
        secret = escape_html(&query.session_secret),
        title_h = escape_html(&query.title),
        contract = escape_html(&query.contract_address),
        token_h = escape_html(&query.token_id),
        foundation = escape_html(query.foundation_url.as_deref().unwrap_or("")),
        artist_h = escape_html(query.artist_username.as_deref().unwrap_or("")),
        meta = escape_html(query.metadata_cid.as_deref().unwrap_or("")),
        media = escape_html(query.media_cid.as_deref().unwrap_or("")),
    );

    Ok(Html(render_page("Pin rescued work", &body)))
}

async fn share_work_form(
    State(state): State<AppState>,
    Form(input): Form<ShareWorkRequest>,
) -> Result<Html<String>, AppError> {
    let response = share_work_inner(&state, input).await?;
    let pin_rows = response
        .pins
        .iter()
        .map(|pin| {
            format!(
                r#"<li><span class="muted">{}</span><code>{}</code></li>"#,
                escape_html(pin.label.as_deref().unwrap_or("pin")),
                escape_html(&pin.cid)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let pins_block = if pin_rows.is_empty() {
        String::new()
    } else {
        format!(r#"<ul class="plain" style="margin-top: 16px;">{}</ul>"#, pin_rows)
    };

    let body = format!(
        r#"<main class="shell narrow">
  <div class="stack">
    <section class="section-head">
      <p class="eyebrow">Pinned</p>
      <h1>{title}</h1>
      <p class="lead">{message}</p>
    </section>

    <section class="card">
      <p class="eyebrow">Watched roots</p>
      <h2 style="margin-top: 8px;">Now part of the forever list</h2>
      <p class="muted" style="margin-top: 10px;">The bridge will keep checking these on every repair cycle and re-pin them if they ever disappear.</p>
      {pins}
      <div class="btn-row">
        <a class="btn ghost" href="/">Back to bridge home</a>
      </div>
    </section>
  </div>
</main>
<style>
  ul.plain li {{ display: grid; grid-template-columns: 120px 1fr; align-items: center; gap: 16px; }}
  ul.plain li .muted {{
    font-family: ui-monospace, Menlo, Consolas, monospace;
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.22em;
  }}
</style>"#,
        title = escape_html(&response.title),
        message = escape_html(response.message),
        pins = pins_block,
    );

    Ok(Html(render_page("Pinned", &body)))
}

async fn share_profile(
    State(state): State<AppState>,
    Json(input): Json<ShareProfileRequest>,
) -> Result<Json<ShareProfileResponse>, AppError> {
    let response = share_profile_inner(&state, input).await?;
    Ok(Json(response))
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

async fn diagnose_single_pin(
    AxumPath(cid): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<DiagnoseResponse>, AppError> {
    let trimmed = cid.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("CID is required"));
    }
    Ok(Json(diagnose_pin(&state, trimmed).await))
}

async fn retry_pin_now(
    AxumPath(cid): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<RetryPinResponse>, AppError> {
    let trimmed = cid.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("CID is required"));
    }

    {
        let mut persistent = state.persistent.write().await;
        if let Some(existing) = persistent.watched_pins.get_mut(&trimmed) {
            existing.next_retry_at = None;
        } else {
            return Err(AppError::bad_request("CID is not watched by this bridge"));
        }
        persistent.updated_at = Some(Utc::now());
    }

    let snapshot = {
        state
            .persistent
            .read()
            .await
            .watched_pins
            .get(&trimmed)
            .cloned()
            .ok_or_else(|| AppError::bad_request("CID disappeared during retry"))?
    };

    match pin_single_cid(&state, &trimmed, snapshot.label.clone()).await {
        Ok(_) => {
            remember_watched_pin(
                &state,
                WatchPinInput {
                    cid: snapshot.cid.clone(),
                    label: snapshot.label.clone(),
                    preferred_file_name: snapshot.preferred_file_name.clone(),
                    source_kind: snapshot.source_kind.clone(),
                    title: snapshot.title.clone(),
                    contract_address: snapshot.contract_address.clone(),
                    token_id: snapshot.token_id.clone(),
                    foundation_url: snapshot.foundation_url.clone(),
                    artist_username: snapshot.artist_username.clone(),
                    account_address: snapshot.account_address.clone(),
                    username: snapshot.username.clone(),
                },
                snapshot.pin_reference.clone(),
                None,
                true,
            )
            .await?;
            Ok(Json(RetryPinResponse {
                cid: trimmed,
                pinned: true,
                used_remote_service: None,
                message: "Pin refreshed locally.".to_string(),
            }))
        }
        Err(error) => {
            let message = error.message.clone();
            let (_category_label, hint) = categorize_pin_error(&message);
            let hint_name = snapshot.title.clone().or_else(|| Some(trimmed.clone()));
            let remote_result =
                submit_to_remote_pinning_service(&state, &trimmed, hint_name.as_deref()).await;
            let (used_remote, remote_err) = match remote_result {
                Ok(Some(service)) => (Some(service), None),
                Ok(None) => (None, None),
                Err(err) => (None, Some(err.to_string())),
            };
            {
                let mut persistent = state.persistent.write().await;
                let now = Utc::now();
                if let Some(existing) = persistent.watched_pins.get_mut(&trimmed) {
                    existing.last_error = Some(message.clone());
                    existing.error_category = Some(_category_label.to_string());
                    if let Some(service) = &used_remote {
                        existing.remote_pinned = true;
                        existing.remote_pin_service = Some(service.clone());
                        existing.remote_pin_last_attempt_at = Some(now);
                        existing.remote_pin_last_error = None;
                    } else if let Some(err) = &remote_err {
                        existing.remote_pin_last_error = Some(err.clone());
                        existing.remote_pin_last_attempt_at = Some(now);
                    }
                }
                persistent.updated_at = Some(now);
            }
            persist_bridge_state(&state).await.map_err(AppError::internal)?;
            let reply = if let Some(service) = used_remote.clone() {
                format!(
                    "Local pin failed ({hint}), but the remote pinning service {service} accepted it."
                )
            } else {
                format!("Local pin failed. {hint} Detail: {message}")
            };
            Ok(Json(RetryPinResponse {
                cid: trimmed,
                pinned: false,
                used_remote_service: used_remote,
                message: reply,
            }))
        }
    }
}

async fn retry_sync_single(
    AxumPath(cid): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<RetrySyncResponse>, AppError> {
    let trimmed = cid.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("CID is required"));
    }
    let exists = state.persistent.read().await.watched_pins.contains_key(&trimmed);
    if !exists {
        return Err(AppError::bad_request("CID is not watched by this bridge"));
    }
    match sync_cid_to_download_dir(&state, &trimmed).await {
        Ok(path) => Ok(Json(RetrySyncResponse {
            cid: trimmed,
            synced: true,
            path: Some(path.display().to_string()),
            error: None,
        })),
        Err(error) => Ok(Json(RetrySyncResponse {
            cid: trimmed,
            synced: false,
            path: None,
            error: Some(error.to_string()),
        })),
    }
}

async fn set_pin_tags(
    AxumPath(cid): AxumPath<String>,
    State(state): State<AppState>,
    Json(input): Json<SetPinTagsRequest>,
) -> Result<Json<SetPinTagsResponse>, AppError> {
    let trimmed = cid.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("CID is required"));
    }
    let cleaned: Vec<String> = {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for raw in input.tags {
            if let Some(tag) = sanitize_custom_tag(&raw) {
                let key = tag.to_ascii_lowercase();
                if seen.insert(key) {
                    out.push(tag);
                }
            }
        }
        out
    };
    {
        let mut persistent = state.persistent.write().await;
        let existing = persistent
            .watched_pins
            .get_mut(&trimmed)
            .ok_or_else(|| AppError::bad_request("CID is not watched by this bridge"))?;
        existing.custom_tags = cleaned.clone();
        persistent.updated_at = Some(Utc::now());
    }
    persist_bridge_state(&state).await.map_err(AppError::internal)?;
    Ok(Json(SetPinTagsResponse { cid: trimmed, tags: cleaned }))
}

async fn gateway_health_handler(State(state): State<AppState>) -> Json<GatewayHealthResponse> {
    Json(gateway_health_probe(&state).await)
}

async fn storage_stats_handler(State(state): State<AppState>) -> Json<StorageSnapshot> {
    Json(build_storage_snapshot(&state).await)
}

async fn live_status_handler(State(state): State<AppState>) -> Json<OperationStatus> {
    Json(state.operation.read().await.clone())
}

async fn export_pins_handler(
    State(state): State<AppState>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, AppError> {
    let snapshot = state.persistent.read().await.clone();
    let format = query
        .format
        .as_deref()
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "json".to_string());
    match format.as_str() {
        "csv" => {
            let mut body = String::new();
            body.push_str(
                "cid,title,artist_username,contract_address,token_id,foundation_url,source_kind,label,added_at,last_verified_at,last_repaired_at,verify_count,repair_count,sync_count,last_error,error_category,retry_attempts,remote_pinned,remote_pin_service,custom_tags,sync_path\n",
            );
            for pin in snapshot.watched_pins.values() {
                body.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                    csv_escape(&pin.cid),
                    csv_escape(pin.title.as_deref().unwrap_or("")),
                    csv_escape(pin.artist_username.as_deref().unwrap_or("")),
                    csv_escape(pin.contract_address.as_deref().unwrap_or("")),
                    csv_escape(pin.token_id.as_deref().unwrap_or("")),
                    csv_escape(pin.foundation_url.as_deref().unwrap_or("")),
                    csv_escape(&pin.source_kind),
                    csv_escape(pin.label.as_deref().unwrap_or("")),
                    csv_escape(&pin.added_at.to_rfc3339()),
                    csv_escape(&pin.last_verified_at.map(|t| t.to_rfc3339()).unwrap_or_default()),
                    csv_escape(&pin.last_repaired_at.map(|t| t.to_rfc3339()).unwrap_or_default()),
                    pin.verify_count,
                    pin.repair_count,
                    pin.sync_count,
                    csv_escape(pin.last_error.as_deref().unwrap_or("")),
                    csv_escape(pin.error_category.as_deref().unwrap_or("")),
                    pin.retry_attempts,
                    pin.remote_pinned,
                    csv_escape(pin.remote_pin_service.as_deref().unwrap_or("")),
                    csv_escape(&pin.custom_tags.join(";")),
                    csv_escape(pin.sync_path.as_deref().unwrap_or("")),
                ));
            }
            Ok((
                StatusCode::OK,
                [
                    ("content-type", "text/csv; charset=utf-8"),
                    (
                        "content-disposition",
                        "attachment; filename=\"foundation-share-bridge-pins.csv\"",
                    ),
                ],
                body,
            )
                .into_response())
        }
        _ => {
            let json = serde_json::to_vec_pretty(&snapshot)
                .map_err(|err| AppError::internal(anyhow!("Unable to encode pins: {err}")))?;
            Ok((
                StatusCode::OK,
                [
                    ("content-type", "application/json"),
                    (
                        "content-disposition",
                        "attachment; filename=\"foundation-share-bridge-pins.json\"",
                    ),
                ],
                json,
            )
                .into_response())
        }
    }
}

async fn artist_summary_handler(State(state): State<AppState>) -> Json<ArtistSummary> {
    let persistent = state.persistent.read().await.clone();
    let sessions = state.sessions.read().await.clone();
    let mut artist_counts: HashMap<String, HashSet<String>> = HashMap::new();
    let mut works_by_group: HashSet<String> = HashSet::new();
    let mut total_copies = 0_usize;
    let current_username = sessions.values().filter_map(|s| s.profile_username.clone()).next();
    let mut works_by_you = 0_usize;
    for pin in persistent.watched_pins.values() {
        total_copies += 1;
        let group = inventory_work_group_key(pin).unwrap_or_else(|| pin.cid.clone());
        if works_by_group.insert(group.clone()) {
            let artist = pin.artist_username.clone().unwrap_or_else(|| "unknown".to_string());
            artist_counts.entry(artist).or_default().insert(group.clone());
            if let Some(me) = current_username.as_deref()
                && pin
                    .artist_username
                    .as_deref()
                    .map(|v| v.eq_ignore_ascii_case(me))
                    .unwrap_or(false)
            {
                works_by_you += 1;
            }
        }
    }
    let artists_tracked = artist_counts.len();
    let mut top_artists: Vec<ArtistEntry> = artist_counts
        .into_iter()
        .map(|(username, works)| ArtistEntry { artist_username: username, works: works.len() })
        .collect();
    top_artists.sort_by(|a, b| {
        b.works.cmp(&a.works).then_with(|| a.artist_username.cmp(&b.artist_username))
    });
    top_artists.truncate(5);
    Json(ArtistSummary {
        total_works_managed: works_by_group.len(),
        works_by_you,
        artists_tracked,
        top_artists,
        total_copies_pinned: total_copies,
    })
}
