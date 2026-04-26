//! Foundation Share Bridge — library entry point and crate-core primitives.
//!
//! This crate powers a per-user desktop IPFS pinning companion for the
//! Foundation Archive. The binary in `src/main.rs` is a thin shell that
//! initialises logging and delegates to [`run`].
//!
//! The three ubiquitous types — [`AppState`], [`AppError`], [`OperationStatus`]
//! — live here at the crate root so every other module has a single place to
//! look. `AppState` must stay `Clone`; a compile-time assertion guards it.

#![forbid(unsafe_code)]

use std::{collections::HashMap, env, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    http::{HeaderValue, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Serialize;
use tokio::{net::TcpListener, sync::RwLock};
use tower_governor::{
    GovernorLayer, governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor,
};
use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::info;

pub(crate) mod html;
pub(crate) mod model;
pub(crate) mod util;

use crate::{
    html::handler::{
        root::root_page,
        settings::settings_page,
        share::{share_work_form, share_work_view},
    },
    model::{
        catalog::handler::{archive_all_for_artist, archive_all_for_artist_form},
        config::{
            BridgeConfig, BridgePersistentState, bridge_config_uses_yaml,
            handler::{get_config, update_config, update_config_form},
            service::{
                bridge_config_file_from_env, bridge_state_file_from_env, load_bridge_config,
                load_persistent_state, persist_bridge_config,
            },
        },
        pin::service::{
            handler::{
                add_files, add_files_form, diagnose_single_pin, list_pins, list_pins_page, pin_cid,
                repair_now, retry_pin_now, retry_sync_single, set_pin_tags, sync_now, unwatch_pins,
                verify_pins, verify_single_pin,
            },
            lifecycle::spawn_repair_loop,
        },
        relay::{
            handler::{
                link_relay_device, link_relay_device_form, share_profile, share_work,
                unlink_relay_device, unlink_relay_device_form,
            },
            service::handle_deep_link_command,
            socket::spawn_relay_socket_loop,
            tunnel::spawn_tunnel_loop,
        },
        session::{
            BridgeSession,
            handler::{
                connect_session, disconnect_session, disconnect_session_by_id,
                disconnect_session_by_id_form, list_sessions, session_by_id,
            },
        },
        system::handler::{
            add_private_network_access_header, artist_summary_handler, export_pins_handler,
            gateway_health_handler, health, live_status_handler, storage_stats_handler,
        },
    },
};

/// Shared, cheaply-cloneable handle to every mutable and immutable piece of
/// bridge state. All async handlers receive this via `axum::extract::State`.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) http: Client,
    pub(crate) ipfs_api_url: String,
    pub ipfs_api_auth_header: Option<String>,
    pub state_file: PathBuf,
    pub config_file: PathBuf,
    pub repair_interval_seconds: u64,
    pub sessions: Arc<RwLock<HashMap<String, BridgeSession>>>,
    pub persistent: Arc<RwLock<BridgePersistentState>>,
    pub config: Arc<RwLock<BridgeConfig>>,
    pub operation: Arc<RwLock<OperationStatus>>,
}

// Compile-time invariant: AppState must remain Clone.
// Background loops spawn tokio tasks with `state.clone()`; losing Clone silently
// breaks them at spawn time.
const _: fn() = || {
    const fn assert_clone<T: Clone>() {}
    assert_clone::<AppState>();
};

/// Progress indicator for long-running operations (repair cycle, sync, etc.).
/// Exposed via `GET /status/live` for the dashboard's live progress bar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OperationStatus {
    pub phase: String,
    pub detail: Option<String>,
    pub progress_current: Option<usize>,
    pub progress_total: Option<usize>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl OperationStatus {
    pub fn idle() -> Self {
        let now = Utc::now();
        Self {
            phase: "idle".to_string(),
            detail: None,
            progress_current: None,
            progress_total: None,
            started_at: now,
            updated_at: now,
        }
    }

    pub fn busy(phase: &str, detail: Option<String>, total: Option<usize>) -> Self {
        let now = Utc::now();
        Self {
            phase: phase.to_string(),
            detail,
            progress_current: Some(0),
            progress_total: total,
            started_at: now,
            updated_at: now,
        }
    }
}

/// Crate-wide HTTP error type. Every handler returns `Result<T, AppError>`.
#[derive(Debug)]
pub(crate) struct AppError {
    pub status: StatusCode,
    pub message: String,
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, message: message.into() }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self { status: StatusCode::UNAUTHORIZED, message: message.into() }
    }

    // `anyhow::Error` is taken by value so callers can write
    // `.map_err(AppError::internal)?` without borrowing.
    #[allow(clippy::needless_pass_by_value)]
    pub fn internal(error: anyhow::Error) -> Self {
        Self { status: StatusCode::INTERNAL_SERVER_ERROR, message: error.to_string() }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({ "error": self.message }))).into_response()
    }
}

/// Per-request upload cap. Kept modest because the current upload handler
/// buffers the whole multipart body in memory before forwarding it to Kubo;
/// full streaming is a v0.2 item. Reducing the cap contains the memory-DoS
/// blast radius without touching `add_files`.
const MAX_UPLOAD_BYTES: usize = 500 * 1024 * 1024;

/// Extract the deep-link URL from argv if the binary was invoked as
/// `handle-url <url>` / `open-url <url>`. Kept separate from `run()` so the
/// non-`Send` `env::Args` iterator never crosses an `.await`.
fn deep_link_url_from_args() -> Option<String> {
    let mut args = env::args().skip(1);
    let command = args.next()?;
    if command == "handle-url" || command == "open-url" { args.next() } else { None }
}

/// Build the CORS layer for the bridge HTTP API.
///
/// Only allow the archive site and loopback origins. Same-origin local UI
/// (served from this binary) does not need CORS; the allowlist exists so
/// browsers running the archive site or a dev gateway can talk to the
/// bridge without opening it up to the entire public web.
fn bridge_cors_layer() -> CorsLayer {
    let allowlist = AllowOrigin::predicate(|origin: &HeaderValue, _request_head| {
        let Ok(origin_str) = origin.to_str() else { return false };
        origin_str == "https://foundation.agorix.io"
            || origin_str.ends_with(".agorix.io")
            || origin_str.starts_with("http://127.0.0.1:")
            || origin_str.starts_with("http://localhost:")
    });
    CorsLayer::new()
        .allow_origin(allowlist)
        .allow_headers(Any)
        .allow_methods(Any)
        .allow_credentials(false)
}

/// Build the per-IP rate-limit layer. 30-request burst, refilled at 5 rps
/// per source IP — sized for the archive site's normal call pattern (health
/// polling + inventory pagination) but choking anything that floods. Uses
/// [`SmartIpKeyExtractor`] so forwarded headers are honored when the bridge
/// is reached through a trusted proxy / Cloudflare tunnel, falling back to
/// the raw connect-info IP otherwise.
///
/// Returns the [`GovernorLayer`] ready to plug into an [`axum::Router`]; the
/// `RespBody` generic is inferred as `axum::body::Body` at the router call
/// site. `expect` is used on the builder because the hardcoded 5 rps / 30
/// burst values cannot fail to validate.
#[allow(clippy::expect_used)]
fn bridge_governor_layer()
-> GovernorLayer<SmartIpKeyExtractor, governor::middleware::NoOpMiddleware, axum::body::Body> {
    let config = GovernorConfigBuilder::default()
        .per_second(5)
        .burst_size(30)
        .key_extractor(SmartIpKeyExtractor)
        .finish()
        .expect("governor config is valid");
    GovernorLayer::new(config)
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
#[allow(clippy::too_many_lines)]
pub async fn run() -> anyhow::Result<()> {
    if let Some(raw_url) = deep_link_url_from_args() {
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
        .map_or(900, |value| value.max(60));
    let state_file = bridge_state_file_from_env()?;
    let config_file = bridge_config_file_from_env(&state_file)?;
    let should_seed_config_file = bridge_config_uses_yaml(&config_file) && !config_file.exists();

    let address: SocketAddr = format!("{host}:{port}")
        .parse()
        .with_context(|| format!("Unable to parse bridge bind address {host}:{port}"))?;

    let persistent = load_persistent_state(&state_file).await?;
    let config = load_bridge_config(&config_file, &state_file).await?;

    // Hydrate the in-memory session map from whatever survived the last run
    // so the archive site's auto-reconnect reuses the deterministic session
    // instead of minting a fresh one.
    let sessions = persistent.sessions.clone();

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
        sessions: Arc::new(RwLock::new(sessions)),
        persistent: Arc::new(RwLock::new(persistent)),
        config: Arc::new(RwLock::new(config)),
        operation: Arc::new(RwLock::new(OperationStatus::idle())),
    };

    if should_seed_config_file {
        persist_bridge_config(&state).await?;
    }

    spawn_repair_loop(state.clone());
    spawn_relay_socket_loop(state.clone());
    spawn_tunnel_loop(state.clone());

    let health_routes = Router::new()
        .route("/health", get(health))
        .route("/gateway/health", get(gateway_health_handler))
        .route("/storage/stats", get(storage_stats_handler))
        .route("/status/live", get(live_status_handler));

    let protected_routes = Router::new()
        .route("/", get(root_page))
        .route("/settings", get(settings_page))
        .route("/sessions", get(list_sessions))
        .route("/session/connect", post(connect_session))
        .route("/session/disconnect", post(disconnect_session))
        .route("/session/{session_id}", get(session_by_id))
        .route("/session/{session_id}/disconnect", post(disconnect_session_by_id))
        .route("/session/{session_id}/disconnect/form", post(disconnect_session_by_id_form))
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
        .route("/artists/summary", get(artist_summary_handler))
        .route("/sync/run", post(sync_now))
        .route("/ipfs/pin", post(pin_cid))
        .route("/ipfs/add", post(add_files).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)))
        .route(
            "/ipfs/add/form",
            post(add_files_form).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .route("/artists/{username}/archive-all", post(archive_all_for_artist))
        .route("/artists/archive-all/form", post(archive_all_for_artist_form))
        .route("/share/work", post(share_work))
        .route("/share/work/view", get(share_work_view))
        .route("/share/work/form", post(share_work_form))
        .route("/share/profile", post(share_profile))
        // Keep expensive/session-mutating routes guarded, but do not throttle
        // readiness polling. Browser tabs and the menu companion can ask
        // `/health` often without consuming this bucket.
        .layer(bridge_governor_layer());

    let app = Router::new()
        .merge(health_routes)
        .merge(protected_routes)
        .layer(bridge_cors_layer())
        .layer(middleware::map_response(add_private_network_access_header))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("Unable to bind bridge listener on {address}"))?;

    info!("foundation-share-bridge listening on http://{address}");
    // `into_make_service_with_connect_info::<SocketAddr>()` is required so
    // the per-IP rate limiter can fall back to the raw connect-info IP when
    // no `X-Forwarded-For` / `Forwarded` / `X-Real-IP` header is present
    // (i.e. for direct browser hits to the loopback listener).
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context("Bridge server stopped unexpectedly")?;

    Ok(())
}
