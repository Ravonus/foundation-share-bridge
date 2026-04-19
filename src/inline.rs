//! Transitional module — holds only the HTTP server composition root now that
//! every domain has its own home. Stage 11 inlines `run()` into `lib.rs` and
//! deletes this file.

use std::{collections::HashMap, env, net::SocketAddr, sync::Arc};

use anyhow::Context;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
};
use reqwest::Client;
use tokio::{net::TcpListener, sync::RwLock};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::info;

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
            service::{
                bridge_config_file_from_env, bridge_state_file_from_env, load_bridge_config,
                load_persistent_state, persist_bridge_config,
            },
        },
        pin::service::{
            handler::{
                add_files, diagnose_single_pin, list_pins, list_pins_page, pin_cid, repair_now,
                retry_pin_now, retry_sync_single, set_pin_tags, sync_now, unwatch_pins,
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
        },
        session::handler::{connect_session, disconnect_session, list_sessions, session_by_id},
        system::handler::{
            add_private_network_access_header, artist_summary_handler, export_pins_handler,
            gateway_health_handler, health, live_status_handler, storage_stats_handler,
        },
    },
};

const MAX_UPLOAD_BYTES: usize = 5 * 1024 * 1024 * 1024;

/// Extract the deep-link URL from argv if the binary was invoked as
/// `handle-url <url>` / `open-url <url>`. Kept separate from `run()` so the
/// non-`Send` `env::Args` iterator never crosses an `.await`.
fn deep_link_url_from_args() -> Option<String> {
    let mut args = env::args().skip(1);
    let command = args.next()?;
    if command == "handle-url" || command == "open-url" { args.next() } else { None }
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
