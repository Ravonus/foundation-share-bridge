use std::{
    collections::HashMap,
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, anyhow};
use async_recursion::async_recursion;
use axum::{
    Form, Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tokio::{
    fs,
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

#[derive(Clone)]
struct AppState {
    http: Client,
    ipfs_api_url: String,
    ipfs_api_auth_header: Option<String>,
    state_file: PathBuf,
    config_file: PathBuf,
    repair_interval_seconds: u64,
    sessions: Arc<RwLock<HashMap<String, BridgeSession>>>,
    persistent: Arc<RwLock<BridgePersistentState>>,
    config: Arc<RwLock<BridgeConfig>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BridgeSession {
    session_id: String,
    session_secret: String,
    website_origin: String,
    account_address: Option<String>,
    profile_username: Option<String>,
    client_name: Option<String>,
    connected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WatchedPin {
    cid: String,
    label: Option<String>,
    source_kind: String,
    title: Option<String>,
    contract_address: Option<String>,
    token_id: Option<String>,
    foundation_url: Option<String>,
    artist_username: Option<String>,
    account_address: Option<String>,
    username: Option<String>,
    added_at: DateTime<Utc>,
    last_verified_at: Option<DateTime<Utc>>,
    last_repaired_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    pin_reference: Option<String>,
    #[serde(default)]
    verify_count: u64,
    #[serde(default)]
    repair_count: u64,
    sync_path: Option<String>,
    local_gateway_url: Option<String>,
    public_gateway_url: Option<String>,
    last_synced_at: Option<DateTime<Utc>>,
    last_sync_error: Option<String>,
    #[serde(default)]
    sync_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct BridgePersistentState {
    watched_pins: HashMap<String, WatchedPin>,
    updated_at: Option<DateTime<Utc>>,
    last_repair_cycle_at: Option<DateTime<Utc>>,
    repair_cycle_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BridgeConfig {
    download_root_dir: String,
    sync_enabled: bool,
    local_gateway_base_url: String,
    public_gateway_base_url: String,
    relay_enabled: bool,
    relay_server_url: String,
    relay_device_name: String,
    relay_device_id: Option<String>,
    relay_device_label: Option<String>,
    relay_device_token: Option<String>,
    relay_last_connected_at: Option<DateTime<Utc>>,
    relay_last_error: Option<String>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    ipfs_api_url: String,
    state_file: String,
    config_file: String,
    active_sessions: usize,
    watched_pin_count: usize,
    repair_interval_seconds: u64,
    last_repair_cycle_at: Option<DateTime<Utc>>,
    download_root_dir: String,
    sync_enabled: bool,
    local_gateway_base_url: String,
    public_gateway_base_url: String,
    relay_enabled: bool,
    relay_server_url: String,
    relay_device_name: String,
    relay_device_id: Option<String>,
    relay_device_label: Option<String>,
    relay_last_connected_at: Option<DateTime<Utc>>,
    relay_last_error: Option<String>,
    now: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct ConnectSessionRequest {
    website_origin: String,
    account_address: Option<String>,
    profile_username: Option<String>,
    client_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct ConnectSessionResponse {
    session: BridgeSession,
    message: &'static str,
}

#[derive(Debug, Deserialize)]
struct DisconnectSessionRequest {
    session_secret: String,
}

#[derive(Debug, Serialize)]
struct DisconnectSessionResponse {
    disconnected: bool,
}

#[derive(Debug, Deserialize)]
struct PinCidRequest {
    session_secret: Option<String>,
    cid: String,
    label: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct PinCidResult {
    cid: String,
    label: Option<String>,
    pinned: bool,
    provider: &'static str,
    pin_reference: String,
    requested_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
struct ShareWorkRequest {
    session_secret: String,
    title: String,
    contract_address: String,
    token_id: String,
    foundation_url: Option<String>,
    metadata_cid: Option<String>,
    media_cid: Option<String>,
    artist_username: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RelayShareWorkPayload {
    title: String,
    contract_address: String,
    token_id: String,
    foundation_url: Option<String>,
    metadata_cid: Option<String>,
    media_cid: Option<String>,
    artist_username: Option<String>,
}

#[derive(Debug, Serialize)]
struct ShareWorkResponse {
    share_id: String,
    title: String,
    contract_address: String,
    token_id: String,
    foundation_url: Option<String>,
    artist_username: Option<String>,
    pins: Vec<PinCidResult>,
    message: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
struct ShareProfileRequest {
    session_secret: String,
    account_address: String,
    username: Option<String>,
    label: Option<String>,
    cids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ShareProfileResponse {
    share_id: String,
    account_address: String,
    username: Option<String>,
    label: Option<String>,
    pinned_count: usize,
    pins: Vec<PinCidResult>,
    message: &'static str,
}

#[derive(Debug, Serialize)]
struct SessionSummary {
    session_id: String,
    website_origin: String,
    account_address: Option<String>,
    profile_username: Option<String>,
    client_name: Option<String>,
    connected_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct PinsResponse {
    total: usize,
    #[serde(rename = "pinnedCount")]
    pinned_count: usize,
    #[serde(rename = "managedCount")]
    managed_count: usize,
    last_repair_cycle_at: Option<DateTime<Utc>>,
    items: Vec<PinInventoryItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinInventoryItem {
    cid: String,
    pinned: bool,
    pin_type: Option<String>,
    managed: bool,
    label: Option<String>,
    source_kind: Option<String>,
    title: Option<String>,
    contract_address: Option<String>,
    token_id: Option<String>,
    foundation_url: Option<String>,
    artist_username: Option<String>,
    account_address: Option<String>,
    username: Option<String>,
    added_at: Option<DateTime<Utc>>,
    last_verified_at: Option<DateTime<Utc>>,
    last_repaired_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    pin_reference: Option<String>,
    verify_count: u64,
    repair_count: u64,
    sync_path: Option<String>,
    local_gateway_url: Option<String>,
    public_gateway_url: Option<String>,
    last_synced_at: Option<DateTime<Utc>>,
    last_sync_error: Option<String>,
    sync_count: u64,
}

#[derive(Debug, Serialize)]
struct RepairNowResponse {
    repaired: usize,
    healthy: usize,
    failed: usize,
    message: &'static str,
}

#[derive(Debug, Deserialize)]
struct VerifyPinsRequest {
    #[serde(default)]
    cids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PinVerification {
    cid: String,
    reachable: bool,
    provider_count: usize,
    checked_at: DateTime<Utc>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VerifyPinsResponse {
    checked_at: DateTime<Utc>,
    results: Vec<PinVerification>,
}

#[derive(Debug, Serialize)]
struct SyncNowResponse {
    synced: usize,
    failed: usize,
    skipped: usize,
    message: &'static str,
}

#[derive(Debug, Serialize)]
struct BridgeConfigResponse {
    download_root_dir: String,
    sync_enabled: bool,
    local_gateway_base_url: String,
    public_gateway_base_url: String,
    relay_enabled: bool,
    relay_server_url: String,
    relay_device_name: String,
    relay_device_id: Option<String>,
    relay_device_label: Option<String>,
    relay_last_connected_at: Option<DateTime<Utc>>,
    relay_last_error: Option<String>,
    config_file: String,
}

#[derive(Debug, Deserialize)]
struct UpdateBridgeConfigRequest {
    download_root_dir: Option<String>,
    sync_enabled: Option<bool>,
    local_gateway_base_url: Option<String>,
    public_gateway_base_url: Option<String>,
    relay_enabled: Option<bool>,
    relay_server_url: Option<String>,
    relay_device_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RelayLinkRequest {
    relay_server_url: Option<String>,
    pairing_code: String,
    device_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct RelayLinkResponse {
    relay_enabled: bool,
    relay_server_url: String,
    relay_device_name: String,
    relay_device_id: String,
    relay_device_label: String,
    linked_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct RelayUnlinkResponse {
    unlinked: bool,
}

#[derive(Debug, Deserialize)]
struct RootPageQuery {
    session_id: Option<String>,
    relay_server_url: Option<String>,
    pairing_code: Option<String>,
    device_name: Option<String>,
    linked: Option<String>,
    unlinked: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ShareWorkViewQuery {
    session_secret: String,
    title: String,
    contract_address: String,
    token_id: String,
    foundation_url: Option<String>,
    metadata_cid: Option<String>,
    media_cid: Option<String>,
    artist_username: Option<String>,
}

#[derive(Debug)]
struct AppError {
    status: StatusCode,
    message: String,
}

#[derive(Debug, Default)]
struct RepairCycleOutcome {
    repaired: usize,
    healthy: usize,
    failed: usize,
}

#[derive(Debug, Default)]
struct SyncOutcome {
    synced: usize,
    failed: usize,
    skipped: usize,
}

#[derive(Debug, Deserialize)]
struct RelayLinkFormRequest {
    relay_server_url: String,
    pairing_code: String,
    device_name: Option<String>,
}

#[derive(Debug)]
struct PairingDeepLink {
    relay_server_url: String,
    pairing_code: String,
    device_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct RelayInventoryMessage {
    r#type: &'static str,
    items: Vec<PinInventoryItem>,
}

#[derive(Debug, Serialize)]
struct RelayJobResultMessage {
    r#type: &'static str,
    job_id: String,
    status: &'static str,
    result_payload: Option<String>,
    error_message: Option<String>,
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
    let device_name = device_name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    Ok(PairingDeepLink {
        relay_server_url,
        pairing_code,
        device_name,
    })
}

async fn wait_for_local_bridge_ready(client: &Client, bridge_origin: &str) -> anyhow::Result<()> {
    let health_url = format!("{}/health", trim_trailing_slash(bridge_origin));

    for _ in 0..40 {
        if let Ok(response) = client.get(&health_url).send().await {
            if response.status().is_success() {
                return Ok(());
            }
        }

        sleep(Duration::from_millis(500)).await;
    }

    Err(anyhow!(
        "The local bridge did not come online at {} in time.",
        health_url
    ))
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
        .post(format!(
            "{}/relay/link",
            trim_trailing_slash(&bridge_origin)
        ))
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

#[derive(Debug, Deserialize)]
struct RelayWelcomeMessage {
    #[serde(rename = "type")]
    _type: String,
    device_id: Option<String>,
    device_label: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RelayRequestInventoryMessage {
    #[serde(rename = "type")]
    _type: String,
}

#[derive(Debug, Deserialize)]
struct RelayForceDisconnectMessage {
    #[serde(rename = "type")]
    _type: String,
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RelayJobMessage {
    #[serde(rename = "type")]
    _type: String,
    job_id: String,
    kind: String,
    payload: String,
}

#[derive(Debug, Clone)]
struct WatchPinInput {
    cid: String,
    label: Option<String>,
    source_kind: String,
    title: Option<String>,
    contract_address: Option<String>,
    token_id: Option<String>,
    foundation_url: Option<String>,
    artist_username: Option<String>,
    account_address: Option<String>,
    username: Option<String>,
}

impl AppError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
        }
    }

    fn internal(error: anyhow::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({
                "error": self.message,
            })),
        )
            .into_response()
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "foundation_share_bridge=info,tower_http=info".into()),
        )
        .init();

    let mut args = env::args().skip(1);
    if let Some(command) = args.next() {
        if command == "handle-url" || command == "open-url" {
            let raw_url = args
                .next()
                .ok_or_else(|| anyhow!("Usage: foundation-share-bridge handle-url <app-url>"))?;
            return handle_deep_link_command(&raw_url).await;
        }
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
    };

    spawn_repair_loop(state.clone());
    spawn_relay_socket_loop(state.clone());

    let app = Router::new()
        .route("/", get(root_page))
        .route("/health", get(health))
        .route("/sessions", get(list_sessions))
        .route("/session/connect", post(connect_session))
        .route("/session/disconnect", post(disconnect_session))
        .route("/session/{session_id}", get(session_by_id))
        .route("/config", get(get_config).post(update_config))
        .route("/relay/link", post(link_relay_device))
        .route("/relay/unlink", post(unlink_relay_device))
        .route("/relay/link/form", post(link_relay_device_form))
        .route("/relay/unlink/form", post(unlink_relay_device_form))
        .route("/pins", get(list_pins))
        .route("/pins/repair", post(repair_now))
        .route("/pins/verify", post(verify_pins))
        .route("/sync/run", post(sync_now))
        .route("/ipfs/pin", post(pin_cid))
        .route("/share/work", post(share_work))
        .route("/share/work/view", get(share_work_view))
        .route("/share/work/form", post(share_work_form))
        .route("/share/profile", post(share_profile))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_headers(Any)
                .allow_methods(Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("Unable to bind bridge listener on {address}"))?;

    info!("foundation-share-bridge listening on http://{address}");
    axum::serve(listener, app)
        .await
        .context("Bridge server stopped unexpectedly")?;

    Ok(())
}

fn bridge_state_file_from_env() -> anyhow::Result<PathBuf> {
    if let Some(value) = env::var("BRIDGE_STATE_FILE")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(PathBuf::from(value));
    }

    let cwd = env::current_dir().context("Unable to determine current directory")?;
    Ok(cwd.join("bridge-state.json"))
}

fn bridge_config_file_from_env(state_file: &Path) -> anyhow::Result<PathBuf> {
    if let Some(value) = env::var("BRIDGE_CONFIG_FILE")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(PathBuf::from(value));
    }

    if let Some(parent) = state_file.parent() {
        return Ok(parent.join("bridge-config.json"));
    }

    let cwd = env::current_dir().context("Unable to determine current directory")?;
    Ok(cwd.join("bridge-config.json"))
}

async fn load_persistent_state(path: &Path) -> anyhow::Result<BridgePersistentState> {
    match fs::read_to_string(path).await {
        Ok(contents) => {
            serde_json::from_str::<BridgePersistentState>(&contents).with_context(|| {
                format!(
                    "Unable to parse persistent bridge state from {}",
                    path.display()
                )
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(BridgePersistentState::default())
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "Unable to read persistent bridge state at {}",
                path.display()
            )
        }),
    }
}

fn default_download_root_dir(state_file: &Path) -> String {
    state_file
        .parent()
        .map(|parent| parent.join("synced-ipfs"))
        .unwrap_or_else(|| PathBuf::from("./synced-ipfs"))
        .display()
        .to_string()
}

fn default_bridge_config(state_file: &Path) -> BridgeConfig {
    BridgeConfig {
        download_root_dir: env::var("BRIDGE_DOWNLOAD_ROOT_DIR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default_download_root_dir(state_file)),
        sync_enabled: env::var("BRIDGE_SYNC_ENABLED")
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false),
        local_gateway_base_url: env::var("LOCAL_IPFS_GATEWAY_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "http://127.0.0.1:8080".to_string()),
        public_gateway_base_url: env::var("PUBLIC_IPFS_GATEWAY_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "https://ipfs.io".to_string()),
        relay_enabled: env::var("BRIDGE_RELAY_ENABLED")
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false),
        relay_server_url: env::var("BRIDGE_RELAY_SERVER_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_default(),
        relay_device_name: env::var("BRIDGE_DEVICE_NAME")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Foundation desktop helper".to_string()),
        relay_device_id: None,
        relay_device_label: None,
        relay_device_token: None,
        relay_last_connected_at: None,
        relay_last_error: None,
    }
}

async fn load_bridge_config(path: &Path, state_file: &Path) -> anyhow::Result<BridgeConfig> {
    let defaults = default_bridge_config(state_file);

    match fs::read_to_string(path).await {
        Ok(contents) => {
            let mut config =
                serde_json::from_str::<BridgeConfig>(&contents).with_context(|| {
                    format!("Unable to parse bridge config from {}", path.display())
                })?;

            if config.download_root_dir.trim().is_empty() {
                config.download_root_dir = defaults.download_root_dir;
            }
            if config.local_gateway_base_url.trim().is_empty() {
                config.local_gateway_base_url = defaults.local_gateway_base_url;
            }
            if config.public_gateway_base_url.trim().is_empty() {
                config.public_gateway_base_url = defaults.public_gateway_base_url;
            }
            if config.relay_device_name.trim().is_empty() {
                config.relay_device_name = defaults.relay_device_name;
            }

            Ok(config)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(defaults),
        Err(error) => Err(error)
            .with_context(|| format!("Unable to read bridge config at {}", path.display())),
    }
}

async fn persist_bridge_state(state: &AppState) -> anyhow::Result<()> {
    let snapshot = { state.persistent.read().await.clone() };

    if let Some(parent) = state.state_file.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Unable to create state directory {}", parent.display()))?;
    }

    let json = serde_json::to_vec_pretty(&snapshot).context("Unable to encode bridge state")?;
    fs::write(&state.state_file, json).await.with_context(|| {
        format!(
            "Unable to write persistent bridge state to {}",
            state.state_file.display()
        )
    })?;

    Ok(())
}

async fn persist_bridge_config(state: &AppState) -> anyhow::Result<()> {
    let snapshot = { state.config.read().await.clone() };

    if let Some(parent) = state.config_file.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Unable to create config directory {}", parent.display()))?;
    }

    let json = serde_json::to_vec_pretty(&snapshot).context("Unable to encode bridge config")?;
    fs::write(&state.config_file, json).await.with_context(|| {
        format!(
            "Unable to write bridge config to {}",
            state.config_file.display()
        )
    })?;

    Ok(())
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
                || config
                    .relay_device_token
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
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
    let sessions = state.sessions.read().await;
    let persistent = state.persistent.read().await;
    let config = state.config.read().await;

    Json(HealthResponse {
        status: "ok",
        service: "foundation-share-bridge",
        ipfs_api_url: state.ipfs_api_url.clone(),
        state_file: state.state_file.display().to_string(),
        config_file: state.config_file.display().to_string(),
        active_sessions: sessions.len(),
        watched_pin_count: persistent.watched_pins.len(),
        repair_interval_seconds: state.repair_interval_seconds,
        last_repair_cycle_at: persistent.last_repair_cycle_at,
        download_root_dir: config.download_root_dir.clone(),
        sync_enabled: config.sync_enabled,
        local_gateway_base_url: config.local_gateway_base_url.clone(),
        public_gateway_base_url: config.public_gateway_base_url.clone(),
        relay_enabled: config.relay_enabled,
        relay_server_url: config.relay_server_url.clone(),
        relay_device_name: config.relay_device_name.clone(),
        relay_device_id: config.relay_device_id.clone(),
        relay_device_label: config.relay_device_label.clone(),
        relay_last_connected_at: config.relay_last_connected_at,
        relay_last_error: config.relay_last_error.clone(),
        now: Utc::now(),
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
        sessions
            .values()
            .find(|session| session.session_id == session_id)
            .cloned()
    });

    let inventory = list_local_pin_inventory(&state)
        .await
        .map_err(AppError::internal)?;

    let rows = if inventory.items.is_empty() {
        String::new()
    } else {
        inventory
            .items
            .iter()
            .take(24)
            .map(|pin| {
                let status_pill = if pin.pinned {
                    format!(
                        r#"<span class="pill ok">{}</span>"#,
                        escape_html(pin.pin_type.as_deref().unwrap_or("pinned"))
                    )
                } else {
                    r#"<span class="pill warn">Repair needed</span>"#.to_string()
                };

                let links = {
                    let mut parts = Vec::new();
                    if let Some(url) = pin.local_gateway_url.as_deref() {
                        parts.push(format!(
                            r#"<a href="{}" target="_blank" rel="noreferrer">local</a>"#,
                            escape_html(url)
                        ));
                    }
                    if let Some(url) = pin.public_gateway_url.as_deref() {
                        parts.push(format!(
                            r#"<a href="{}" target="_blank" rel="noreferrer">public</a>"#,
                            escape_html(url)
                        ));
                    }
                    if parts.is_empty() {
                        r#"<span class="muted">—</span>"#.to_string()
                    } else {
                        parts.join(" · ")
                    }
                };

                format!(
                    r#"<tr><td><div>{}</div><div class="cid">{}</div></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                    escape_html(
                        pin.title
                            .as_deref()
                            .or(pin.label.as_deref())
                            .unwrap_or("Local IPFS pin")
                    ),
                    escape_html(&pin.cid),
                    escape_html(
                        pin.foundation_url
                            .as_deref()
                            .or(pin.contract_address.as_deref())
                            .or(pin.username.as_deref())
                            .unwrap_or("—")
                    ),
                    status_pill,
                    escape_html(
                        &pin.last_verified_at
                            .map(format_timestamp)
                            .unwrap_or_else(|| "never".to_string())
                    ),
                    links,
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let relay_connected = config.relay_enabled
        && !config
            .relay_device_token
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty();

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
            server = escape_html(query.relay_server_url.as_deref().unwrap_or("")),
            code = escape_html(query.pairing_code.as_deref().unwrap_or("")),
            name = escape_html(
                query
                    .device_name
                    .as_deref()
                    .or(Some(config.relay_device_name.as_str()))
                    .unwrap_or("Foundation desktop helper")
            ),
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

    let connection_status = if relay_connected {
        "Live"
    } else {
        "Not linked"
    };
    let connection_pill_class = if relay_connected { "pill ok" } else { "pill" };

    let inventory_body = if rows.is_empty() {
        r#"<div class="empty">No pins yet. Once the archive site hands you something to rescue, it will appear here.</div>"#.to_string()
    } else {
        format!(
            r#"<div class="table-wrap">
  <table>
    <thead>
      <tr>
        <th>Item</th>
        <th>Context</th>
        <th>Status</th>
        <th>Last verified</th>
        <th>Gateway</th>
      </tr>
    </thead>
    <tbody>{rows}</tbody>
  </table>
</div>"#,
            rows = rows
        )
    };

    let pinned_count = inventory.pinned_count;
    let managed_count = inventory.managed_count;
    let repair_interval = state.repair_interval_seconds;
    let last_repair = persistent
        .last_repair_cycle_at
        .map(format_timestamp)
        .unwrap_or_else(|| "never".to_string());

    let body = format!(
        r##"<main class="shell">
  <div class="stack">
    <section class="section-head">
      <p class="eyebrow">Foundation share bridge</p>
      <h1>Keep rescued IPFS roots pinned and self-repaired.</h1>
      <p class="lead">This Rust companion app keeps a local memory of watched CIDs, re-checks them forever, and re-pins anything your IPFS node drops. Pair it with the archive site once, then leave it running.</p>
      <div class="btn-row">
        <a class="pill {conn_pill}" href="#connection">{conn_status}</a>
        <span class="pill">{repair_interval}s repair cadence</span>
      </div>
    </section>

    {flash}

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
          <p class="eyebrow">Last repair</p>
          <p class="stat-value" style="font-size: 1rem; font-family: ui-monospace, Menlo, Consolas, monospace;">{last_repair}</p>
          <p class="stat-body">If a pin disappears, it is restored on the next cycle.</p>
        </div>
        <div class="stat">
          <p class="eyebrow">Repair interval</p>
          <p class="stat-value">{repair_interval}s</p>
          <p class="stat-body">How often this app checks that watched roots are still there.</p>
        </div>
      </div>
    </section>

    <section class="two-col">
      {connection}
      {session}
    </section>

    <section id="inventory">
      <div class="section-head" style="border-bottom: 0; padding-bottom: 0;">
        <p class="eyebrow">Local inventory</p>
        <h2 style="margin-top: 8px;">Everything this node has pinned</h2>
        <p class="lead">Foundation-linked roots keep their rescue context. Other IPFS items show up here too.</p>
      </div>
      <div style="margin-top: 20px;">{inventory_body}</div>
    </section>

    <p class="footer">Local bridge · {repair_interval}s repair interval · last cycle {last_repair}</p>
  </div>
</main>"##,
        conn_pill = connection_pill_class,
        conn_status = connection_status,
        pinned = pinned_count,
        managed = managed_count,
        repair_interval = repair_interval,
        last_repair = escape_html(&last_repair),
        flash = flash_block,
        connection = connection_block,
        session = session_block,
        inventory_body = inventory_body,
    );

    Ok(Html(render_page("Foundation Share Bridge", &body)))
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

    Ok(Json(BridgeConfigResponse {
        download_root_dir: config.download_root_dir.clone(),
        sync_enabled: config.sync_enabled,
        local_gateway_base_url: config.local_gateway_base_url.clone(),
        public_gateway_base_url: config.public_gateway_base_url.clone(),
        relay_enabled: config.relay_enabled,
        relay_server_url: config.relay_server_url.clone(),
        relay_device_name: config.relay_device_name.clone(),
        relay_device_id: config.relay_device_id.clone(),
        relay_device_label: config.relay_device_label.clone(),
        relay_last_connected_at: config.relay_last_connected_at,
        relay_last_error: config.relay_last_error.clone(),
        config_file: state.config_file.display().to_string(),
    }))
}

async fn update_config(
    State(state): State<AppState>,
    Json(input): Json<UpdateBridgeConfigRequest>,
) -> Result<Json<BridgeConfigResponse>, AppError> {
    {
        let mut config = state.config.write().await;

        if let Some(download_root_dir) = input.download_root_dir {
            let trimmed = download_root_dir.trim();
            if trimmed.is_empty() {
                return Err(AppError::bad_request("download_root_dir cannot be empty"));
            }
            config.download_root_dir = trimmed.to_string();
        }

        if let Some(sync_enabled) = input.sync_enabled {
            config.sync_enabled = sync_enabled;
        }

        if let Some(local_gateway_base_url) = input.local_gateway_base_url {
            let trimmed = local_gateway_base_url.trim();
            if trimmed.is_empty() {
                return Err(AppError::bad_request(
                    "local_gateway_base_url cannot be empty",
                ));
            }
            config.local_gateway_base_url = trimmed.to_string();
        }

        if let Some(public_gateway_base_url) = input.public_gateway_base_url {
            let trimmed = public_gateway_base_url.trim();
            if trimmed.is_empty() {
                return Err(AppError::bad_request(
                    "public_gateway_base_url cannot be empty",
                ));
            }
            config.public_gateway_base_url = trimmed.to_string();
        }

        if let Some(relay_enabled) = input.relay_enabled {
            config.relay_enabled = relay_enabled;
        }

        if let Some(relay_server_url) = input.relay_server_url {
            config.relay_server_url = relay_server_url.trim().to_string();
        }

        if let Some(relay_device_name) = input.relay_device_name {
            let trimmed = relay_device_name.trim();
            if trimmed.is_empty() {
                return Err(AppError::bad_request("relay_device_name cannot be empty"));
            }
            config.relay_device_name = trimmed.to_string();
        }
    }

    persist_bridge_config(&state)
        .await
        .map_err(AppError::internal)?;

    get_config(State(state)).await
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
            "/?error={}",
            encode_query_component(&error.message)
        ))),
    }
}

async fn perform_relay_link(
    state: &AppState,
    input: RelayLinkRequest,
) -> Result<RelayLinkResponse, AppError> {
    let pairing_code = input.pairing_code.trim();
    if pairing_code.is_empty() {
        return Err(AppError::bad_request("pairing_code is required"));
    }

    let (relay_server_url, device_name) = {
        let config = state.config.read().await;
        (
            input
                .relay_server_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(config.relay_server_url.trim())
                .to_string(),
            input
                .device_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(config.relay_device_name.trim())
                .to_string(),
        )
    };

    if relay_server_url.is_empty() {
        return Err(AppError::bad_request("relay_server_url is required"));
    }

    let endpoint = format!(
        "{}/api/relay/bridge/claim",
        trim_trailing_slash(&relay_server_url)
    );

    let response = state
        .http
        .post(endpoint)
        .json(&serde_json::json!({
            "pairingCode": pairing_code,
            "deviceLabel": device_name,
        }))
        .send()
        .await
        .map_err(|error| AppError::internal(anyhow!("Unable to reach relay server: {error}")))?;

    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::internal(anyhow!(
            "Relay pairing claim failed: {}",
            body
        )));
    }

    let payload = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| {
            AppError::internal(anyhow!("Unable to parse relay pairing response: {error}"))
        })?;

    let device_id = payload
        .get("deviceId")
        .and_then(|value| value.as_str())
        .ok_or_else(|| AppError::internal(anyhow!("Relay response did not include a deviceId")))?;
    let device_label = payload
        .get("deviceLabel")
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            AppError::internal(anyhow!("Relay response did not include a deviceLabel"))
        })?;
    let device_token = payload
        .get("deviceToken")
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            AppError::internal(anyhow!("Relay response did not include a deviceToken"))
        })?;
    let linked_at = Utc::now();

    {
        let mut config = state.config.write().await;
        config.relay_enabled = true;
        config.relay_server_url = relay_server_url;
        config.relay_device_name = device_name;
        config.relay_device_id = Some(device_id.to_string());
        config.relay_device_label = Some(device_label.to_string());
        config.relay_device_token = Some(device_token.to_string());
        config.relay_last_connected_at = Some(linked_at);
        config.relay_last_error = None;
    }

    persist_bridge_config(&state)
        .await
        .map_err(AppError::internal)?;

    Ok(RelayLinkResponse {
        relay_enabled: true,
        relay_server_url: {
            let config = state.config.read().await;
            config.relay_server_url.clone()
        },
        relay_device_name: {
            let config = state.config.read().await;
            config.relay_device_name.clone()
        },
        relay_device_id: device_id.to_string(),
        relay_device_label: device_label.to_string(),
        linked_at,
    })
}

async fn unlink_relay_device(
    State(state): State<AppState>,
) -> Result<Json<RelayUnlinkResponse>, AppError> {
    perform_relay_unlink(&state, true)
        .await
        .map_err(AppError::internal)?;

    Ok(Json(RelayUnlinkResponse { unlinked: true }))
}

async fn unlink_relay_device_form(State(state): State<AppState>) -> Result<Redirect, AppError> {
    perform_relay_unlink(&state, true)
        .await
        .map_err(AppError::internal)?;

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
        account_address: input
            .account_address
            .filter(|value| !value.trim().is_empty()),
        profile_username: input
            .profile_username
            .filter(|value| !value.trim().is_empty()),
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

    Ok(Json(DisconnectSessionResponse {
        disconnected: removed,
    }))
}

async fn list_pins(State(state): State<AppState>) -> Result<Json<PinsResponse>, AppError> {
    let response = list_local_pin_inventory(&state)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(response))
}

async fn repair_now(State(state): State<AppState>) -> Result<Json<RepairNowResponse>, AppError> {
    let outcome = repair_watched_pins(&state)
        .await
        .map_err(AppError::internal)?;

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
    let mut results = Vec::with_capacity(targets.len());

    for cid in targets {
        let result = check_cid_network_providers(&state, &cid).await;
        results.push(result);
    }

    Ok(Json(VerifyPinsResponse {
        checked_at: Utc::now(),
        results,
    }))
}

async fn resolve_verify_targets(state: &AppState, requested: Option<&[String]>) -> Vec<String> {
    if let Some(raw) = requested {
        let mut seen = std::collections::HashSet::new();
        return raw
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty() && seen.insert(value.clone()))
            .collect();
    }

    let persistent = state.persistent.read().await;
    persistent.watched_pins.keys().cloned().collect()
}

async fn check_cid_network_providers(state: &AppState, cid: &str) -> PinVerification {
    let trimmed = cid.trim();
    let checked_at = Utc::now();

    if trimmed.is_empty() {
        return PinVerification {
            cid: cid.to_string(),
            reachable: false,
            provider_count: 0,
            checked_at,
            error: Some("Empty CID".to_string()),
        };
    }

    // Kubo uses /api/v0/routing/findprovs in newer releases, with
    // /api/v0/dht/findprovs kept as a deprecated alias. We try the modern path
    // first and fall back so older daemons keep working.
    match fetch_provider_count(state, trimmed, "routing/findprovs").await {
        Ok(count) => PinVerification {
            cid: trimmed.to_string(),
            reachable: count > 0,
            provider_count: count,
            checked_at,
            error: None,
        },
        Err(primary_error) => match fetch_provider_count(state, trimmed, "dht/findprovs").await {
            Ok(count) => PinVerification {
                cid: trimmed.to_string(),
                reachable: count > 0,
                provider_count: count,
                checked_at,
                error: None,
            },
            Err(fallback_error) => PinVerification {
                cid: trimmed.to_string(),
                reachable: false,
                provider_count: 0,
                checked_at,
                error: Some(format!(
                    "routing/findprovs failed: {primary_error}; dht/findprovs failed: {fallback_error}"
                )),
            },
        },
    }
}

async fn fetch_provider_count(
    state: &AppState,
    cid: &str,
    endpoint: &str,
) -> anyhow::Result<usize> {
    let url = format!(
        "{}/api/v0/{}?arg={}&num-providers=5&verbose=false",
        state.ipfs_api_url.trim_end_matches('/'),
        endpoint,
        cid
    );

    let mut request = state.http.post(url).timeout(Duration::from_secs(12));
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "IPFS {endpoint} responded with status {status}: {body}"
        ));
    }

    // The findprovs endpoint streams ndjson. Each line is a JSON object with
    // a `Responses` array that contains peer IDs. A non-empty peer list on any
    // line means at least one provider was found for this CID.
    let body = response.text().await?;
    let mut unique_providers = std::collections::HashSet::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let Some(responses) = value.get("Responses").and_then(|v| v.as_array()) else {
            continue;
        };
        for entry in responses {
            if let Some(peer_id) = entry.get("ID").and_then(|v| v.as_str()) {
                if !peer_id.is_empty() {
                    unique_providers.insert(peer_id.to_string());
                }
            }
        }
    }

    Ok(unique_providers.len())
}

async fn sync_now(State(state): State<AppState>) -> Result<Json<SyncNowResponse>, AppError> {
    let outcome = sync_all_watched_pins(&state, true)
        .await
        .map_err(AppError::internal)?;

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
        format!(
            r#"<ul class="plain" style="margin-top: 16px;">{}</ul>"#,
            detail_rows
        )
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
        format!(
            r#"<ul class="plain" style="margin-top: 16px;">{}</ul>"#,
            pin_rows
        )
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

async fn share_work_inner(
    state: &AppState,
    input: ShareWorkRequest,
) -> Result<ShareWorkResponse, AppError> {
    validate_session(state, &input.session_secret).await?;

    let pins = pin_work_payload(
        state,
        RelayShareWorkPayload {
            title: input.title.clone(),
            contract_address: input.contract_address.clone(),
            token_id: input.token_id.clone(),
            foundation_url: input.foundation_url.clone(),
            metadata_cid: input.metadata_cid.clone(),
            media_cid: input.media_cid.clone(),
            artist_username: input.artist_username.clone(),
        },
    )
    .await?;

    Ok(ShareWorkResponse {
        share_id: Uuid::new_v4().to_string(),
        title: input.title,
        contract_address: input.contract_address,
        token_id: input.token_id,
        foundation_url: input.foundation_url,
        artist_username: input.artist_username,
        pins,
        message: "Work share accepted. The bridge pinned the rescued roots and added them to the forever-watch list.",
    })
}

async fn pin_work_payload(
    state: &AppState,
    input: RelayShareWorkPayload,
) -> Result<Vec<PinCidResult>, AppError> {
    let mut pins = Vec::new();

    if let Some(cid) = input
        .metadata_cid
        .as_deref()
        .filter(|cid| !cid.trim().is_empty())
    {
        pins.push(
            pin_and_watch_cid(
                state,
                WatchPinInput {
                    cid: cid.to_string(),
                    label: Some("metadata".to_string()),
                    source_kind: "work".to_string(),
                    title: Some(input.title.clone()),
                    contract_address: Some(input.contract_address.clone()),
                    token_id: Some(input.token_id.clone()),
                    foundation_url: input.foundation_url.clone(),
                    artist_username: input.artist_username.clone(),
                    account_address: None,
                    username: None,
                },
            )
            .await?,
        );
    }

    if let Some(cid) = input
        .media_cid
        .as_deref()
        .filter(|cid| !cid.trim().is_empty())
    {
        pins.push(
            pin_and_watch_cid(
                state,
                WatchPinInput {
                    cid: cid.to_string(),
                    label: Some("media".to_string()),
                    source_kind: "work".to_string(),
                    title: Some(input.title.clone()),
                    contract_address: Some(input.contract_address.clone()),
                    token_id: Some(input.token_id.clone()),
                    foundation_url: input.foundation_url.clone(),
                    artist_username: input.artist_username.clone(),
                    account_address: None,
                    username: None,
                },
            )
            .await?,
        );
    }

    Ok(pins)
}

async fn share_profile_inner(
    state: &AppState,
    input: ShareProfileRequest,
) -> Result<ShareProfileResponse, AppError> {
    validate_session(state, &input.session_secret).await?;

    let mut seen = HashMap::<String, Option<String>>::new();
    for cid in input.cids {
        let trimmed = cid.trim();
        if !trimmed.is_empty() {
            seen.entry(trimmed.to_string())
                .or_insert_with(|| Some("profile".to_string()));
        }
    }

    let mut pins = Vec::new();
    for (cid, label) in seen {
        pins.push(
            pin_and_watch_cid(
                state,
                WatchPinInput {
                    cid,
                    label,
                    source_kind: "profile".to_string(),
                    title: input.label.clone(),
                    contract_address: None,
                    token_id: None,
                    foundation_url: None,
                    artist_username: None,
                    account_address: Some(input.account_address.clone()),
                    username: input.username.clone(),
                },
            )
            .await?,
        );
    }

    Ok(ShareProfileResponse {
        share_id: Uuid::new_v4().to_string(),
        account_address: input.account_address,
        username: input.username,
        label: input.label,
        pinned_count: pins.len(),
        pins,
        message: "Profile share accepted. The bridge pinned these CIDs and added them to the forever-watch list.",
    })
}

async fn validate_session(state: &AppState, session_secret: &str) -> Result<(), AppError> {
    let sessions = state.sessions.read().await;
    if sessions.contains_key(session_secret) {
        return Ok(());
    }

    Err(AppError::unauthorized(
        "Unknown session_secret. Connect the website before sending share or pin requests.",
    ))
}

async fn pin_and_watch_cid(
    state: &AppState,
    input: WatchPinInput,
) -> Result<PinCidResult, AppError> {
    let result = pin_single_cid(state, &input.cid, input.label.clone()).await?;
    remember_watched_pin(
        state,
        input.clone(),
        Some(result.pin_reference.clone()),
        None,
        true,
    )
    .await?;

    if let Err(error) = sync_cid_if_enabled(state, &input.cid).await {
        warn!("sync after pin failed for {}: {}", input.cid, error);
    }

    Ok(result)
}

async fn remember_watched_pin(
    state: &AppState,
    input: WatchPinInput,
    pin_reference: Option<String>,
    last_error: Option<String>,
    just_repaired: bool,
) -> Result<(), AppError> {
    {
        let mut persistent = state.persistent.write().await;
        let now = Utc::now();

        persistent.updated_at = Some(now);

        if let Some(existing) = persistent.watched_pins.get_mut(&input.cid) {
            existing.label = input.label.or(existing.label.clone());
            existing.title = input.title.or(existing.title.clone());
            existing.contract_address =
                input.contract_address.or(existing.contract_address.clone());
            existing.token_id = input.token_id.or(existing.token_id.clone());
            existing.foundation_url = input.foundation_url.or(existing.foundation_url.clone());
            existing.artist_username = input.artist_username.or(existing.artist_username.clone());
            existing.account_address = input.account_address.or(existing.account_address.clone());
            existing.username = input.username.or(existing.username.clone());
            existing.source_kind = input.source_kind;
            existing.last_verified_at = Some(now);
            existing.pin_reference = pin_reference.or(existing.pin_reference.clone());
            existing.last_error = last_error.clone();
            existing.verify_count += 1;

            if just_repaired {
                existing.last_repaired_at = Some(now);
                existing.repair_count += 1;
            }
        } else {
            persistent.watched_pins.insert(
                input.cid.clone(),
                WatchedPin {
                    cid: input.cid,
                    label: input.label,
                    source_kind: input.source_kind,
                    title: input.title,
                    contract_address: input.contract_address,
                    token_id: input.token_id,
                    foundation_url: input.foundation_url,
                    artist_username: input.artist_username,
                    account_address: input.account_address,
                    username: input.username,
                    added_at: now,
                    last_verified_at: Some(now),
                    last_repaired_at: if just_repaired { Some(now) } else { None },
                    last_error,
                    pin_reference,
                    verify_count: 1,
                    repair_count: if just_repaired { 1 } else { 0 },
                    sync_path: None,
                    local_gateway_url: None,
                    public_gateway_url: None,
                    last_synced_at: None,
                    last_sync_error: None,
                    sync_count: 0,
                },
            );
        }
    }

    persist_bridge_state(state)
        .await
        .map_err(AppError::internal)
}

async fn mark_pin_checked(
    state: &AppState,
    cid: &str,
    last_error: Option<String>,
) -> Result<(), AppError> {
    {
        let mut persistent = state.persistent.write().await;
        let now = Utc::now();

        if let Some(existing) = persistent.watched_pins.get_mut(cid) {
            existing.last_verified_at = Some(now);
            existing.last_error = last_error;
            existing.verify_count += 1;
        }

        persistent.updated_at = Some(now);
    }

    persist_bridge_state(state)
        .await
        .map_err(AppError::internal)
}

async fn mark_pin_synced(
    state: &AppState,
    cid: &str,
    sync_path: String,
    local_gateway_url: String,
    public_gateway_url: String,
) -> anyhow::Result<()> {
    {
        let mut persistent = state.persistent.write().await;
        let now = Utc::now();

        if let Some(existing) = persistent.watched_pins.get_mut(cid) {
            existing.sync_path = Some(sync_path);
            existing.local_gateway_url = Some(local_gateway_url);
            existing.public_gateway_url = Some(public_gateway_url);
            existing.last_synced_at = Some(now);
            existing.last_sync_error = None;
            existing.sync_count += 1;
        }

        persistent.updated_at = Some(now);
    }

    persist_bridge_state(state).await
}

async fn mark_pin_sync_failed(state: &AppState, cid: &str, message: String) -> anyhow::Result<()> {
    {
        let mut persistent = state.persistent.write().await;
        let now = Utc::now();

        if let Some(existing) = persistent.watched_pins.get_mut(cid) {
            existing.last_sync_error = Some(message);
        }

        persistent.updated_at = Some(now);
    }

    persist_bridge_state(state).await
}

fn trim_trailing_slash(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

fn build_gateway_url(base: &str, cid: &str) -> String {
    format!("{}/ipfs/{}", trim_trailing_slash(base), cid.trim())
}

fn encode_query_component(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            b' ' => vec!['+'],
            _ => format!("%{:02X}", byte).chars().collect(),
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct PinLsEntry {
    #[serde(rename = "Type")]
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PinLsResponse {
    #[serde(rename = "Keys")]
    keys: Option<HashMap<String, PinLsEntry>>,
}

async fn list_kubo_pinset(state: &AppState) -> anyhow::Result<HashMap<String, String>> {
    let endpoint = format!("{}/api/v0/pin/ls", state.ipfs_api_url.trim_end_matches('/'));

    let mut request = state.http.post(endpoint);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Unable to list the local IPFS pinset: {body}"));
    }

    let payload = response.json::<PinLsResponse>().await?;
    let mut pins = HashMap::new();
    for (cid, entry) in payload.keys.unwrap_or_default() {
        pins.insert(cid, entry.kind.unwrap_or_else(|| "recursive".to_string()));
    }

    Ok(pins)
}

async fn list_local_pin_inventory(state: &AppState) -> anyhow::Result<PinsResponse> {
    let pinset = list_kubo_pinset(state).await?;
    let persistent = state.persistent.read().await.clone();
    let config = state.config.read().await.clone();

    let mut items = Vec::new();

    for (cid, pin_type) in pinset.iter() {
        let is_top_level_pin = pin_type != "indirect";

        if !is_top_level_pin && !persistent.watched_pins.contains_key(cid) {
            continue;
        }

        if let Some(pin) = persistent.watched_pins.get(cid) {
            items.push(PinInventoryItem {
                cid: cid.clone(),
                pinned: true,
                pin_type: Some(pin_type.clone()),
                managed: true,
                label: pin.label.clone(),
                source_kind: Some(pin.source_kind.clone()),
                title: pin.title.clone(),
                contract_address: pin.contract_address.clone(),
                token_id: pin.token_id.clone(),
                foundation_url: pin.foundation_url.clone(),
                artist_username: pin.artist_username.clone(),
                account_address: pin.account_address.clone(),
                username: pin.username.clone(),
                added_at: Some(pin.added_at),
                last_verified_at: pin.last_verified_at,
                last_repaired_at: pin.last_repaired_at,
                last_error: pin.last_error.clone(),
                pin_reference: pin.pin_reference.clone(),
                verify_count: pin.verify_count,
                repair_count: pin.repair_count,
                sync_path: pin.sync_path.clone(),
                local_gateway_url: Some(build_gateway_url(&config.local_gateway_base_url, cid)),
                public_gateway_url: Some(build_gateway_url(&config.public_gateway_base_url, cid)),
                last_synced_at: pin.last_synced_at,
                last_sync_error: pin.last_sync_error.clone(),
                sync_count: pin.sync_count,
            });
        } else {
            items.push(PinInventoryItem {
                cid: cid.clone(),
                pinned: true,
                pin_type: Some(pin_type.clone()),
                managed: false,
                label: None,
                source_kind: None,
                title: None,
                contract_address: None,
                token_id: None,
                foundation_url: None,
                artist_username: None,
                account_address: None,
                username: None,
                added_at: None,
                last_verified_at: None,
                last_repaired_at: None,
                last_error: None,
                pin_reference: None,
                verify_count: 0,
                repair_count: 0,
                sync_path: None,
                local_gateway_url: Some(build_gateway_url(&config.local_gateway_base_url, cid)),
                public_gateway_url: Some(build_gateway_url(&config.public_gateway_base_url, cid)),
                last_synced_at: None,
                last_sync_error: None,
                sync_count: 0,
            });
        }
    }

    for (cid, pin) in persistent.watched_pins.iter() {
        if pinset.contains_key(cid) {
            continue;
        }

        items.push(PinInventoryItem {
            cid: cid.clone(),
            pinned: false,
            pin_type: None,
            managed: true,
            label: pin.label.clone(),
            source_kind: Some(pin.source_kind.clone()),
            title: pin.title.clone(),
            contract_address: pin.contract_address.clone(),
            token_id: pin.token_id.clone(),
            foundation_url: pin.foundation_url.clone(),
            artist_username: pin.artist_username.clone(),
            account_address: pin.account_address.clone(),
            username: pin.username.clone(),
            added_at: Some(pin.added_at),
            last_verified_at: pin.last_verified_at,
            last_repaired_at: pin.last_repaired_at,
            last_error: pin.last_error.clone(),
            pin_reference: pin.pin_reference.clone(),
            verify_count: pin.verify_count,
            repair_count: pin.repair_count,
            sync_path: pin.sync_path.clone(),
            local_gateway_url: Some(build_gateway_url(&config.local_gateway_base_url, cid)),
            public_gateway_url: Some(build_gateway_url(&config.public_gateway_base_url, cid)),
            last_synced_at: pin.last_synced_at,
            last_sync_error: pin.last_sync_error.clone(),
            sync_count: pin.sync_count,
        });
    }

    items.sort_by(|left, right| {
        let left_added = left.added_at.unwrap_or_else(Utc::now);
        let right_added = right.added_at.unwrap_or_else(Utc::now);
        right_added.cmp(&left_added)
    });

    Ok(PinsResponse {
        total: items.len(),
        pinned_count: items.iter().filter(|item| item.pinned).count(),
        managed_count: items.iter().filter(|item| item.managed).count(),
        last_repair_cycle_at: persistent.last_repair_cycle_at,
        items,
    })
}

async fn sync_cid_if_enabled(state: &AppState, cid: &str) -> anyhow::Result<bool> {
    let sync_enabled = { state.config.read().await.sync_enabled };
    if !sync_enabled {
        return Ok(false);
    }

    sync_cid_to_download_dir(state, cid).await?;
    Ok(true)
}

async fn sync_all_watched_pins(state: &AppState, force: bool) -> anyhow::Result<SyncOutcome> {
    let watched = {
        state
            .persistent
            .read()
            .await
            .watched_pins
            .values()
            .cloned()
            .collect::<Vec<_>>()
    };

    let sync_enabled = { state.config.read().await.sync_enabled };
    let mut outcome = SyncOutcome::default();

    for pin in watched {
        if !force && !sync_enabled {
            outcome.skipped += 1;
            continue;
        }

        if !force && pin.last_synced_at.is_some() && pin.last_sync_error.is_none() {
            outcome.skipped += 1;
            continue;
        }

        match sync_cid_to_download_dir(state, &pin.cid).await {
            Ok(_) => outcome.synced += 1,
            Err(error) => {
                warn!("sync failed for {}: {}", pin.cid, error);
                outcome.failed += 1;
            }
        }
    }

    Ok(outcome)
}

async fn sync_cid_to_download_dir(state: &AppState, cid: &str) -> anyhow::Result<PathBuf> {
    let config = { state.config.read().await.clone() };
    let root_dir = PathBuf::from(config.download_root_dir.clone()).join(cid.trim());

    let sync_result = async {
        if fs::try_exists(&root_dir).await.unwrap_or(false) {
            let _ = fs::remove_dir_all(&root_dir).await;
        }

        fs::create_dir_all(&root_dir)
            .await
            .with_context(|| format!("Unable to create sync directory {}", root_dir.display()))?;

        download_ipfs_path_recursive(state, &format!("/ipfs/{}", cid.trim()), &root_dir).await?;

        let local_gateway_url = build_gateway_url(&config.local_gateway_base_url, cid);
        let public_gateway_url = build_gateway_url(&config.public_gateway_base_url, cid);

        mark_pin_synced(
            state,
            cid,
            root_dir.display().to_string(),
            local_gateway_url,
            public_gateway_url,
        )
        .await?;

        Ok::<PathBuf, anyhow::Error>(root_dir.clone())
    }
    .await;

    if let Err(error) = &sync_result {
        let _ = mark_pin_sync_failed(state, cid, error.to_string()).await;
    }

    sync_result
}

#[async_recursion]
async fn download_ipfs_path_recursive(
    state: &AppState,
    ipfs_path: &str,
    destination_dir: &Path,
) -> anyhow::Result<()> {
    let links = match list_ipfs_links(state, ipfs_path).await {
        Ok(links) => links,
        Err(_) => {
            let target_file = destination_dir.join("content");
            download_ipfs_file(state, ipfs_path, &target_file).await?;
            return Ok(());
        }
    };

    if links.is_empty() {
        let target_file = destination_dir.join("content");
        download_ipfs_file(state, ipfs_path, &target_file).await?;
        return Ok(());
    }

    fs::create_dir_all(destination_dir).await.with_context(|| {
        format!(
            "Unable to create destination directory {}",
            destination_dir.display()
        )
    })?;

    for link in links {
        let name = link
            .get("Name")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim();
        if name.is_empty() {
            continue;
        }

        let child_destination = destination_dir.join(name);
        let child_ipfs_path = format!("{}/{}", ipfs_path.trim_end_matches('/'), name);
        let link_type = link
            .get("Type")
            .and_then(|value| value.as_i64())
            .unwrap_or(0);

        if matches!(link_type, 1 | 5) {
            download_ipfs_path_recursive(state, &child_ipfs_path, &child_destination).await?;
        } else {
            download_ipfs_file(state, &child_ipfs_path, &child_destination).await?;
        }
    }

    Ok(())
}

async fn list_ipfs_links(
    state: &AppState,
    ipfs_path: &str,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let endpoint = format!("{}/api/v0/ls", state.ipfs_api_url.trim_end_matches('/'));

    let mut request = state.http.post(endpoint).query(&[("arg", ipfs_path)]);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request.send().await?;
    let status = response.status();
    let payload = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("Unable to list IPFS path {ipfs_path}: {payload}"));
    }

    let json = serde_json::from_str::<serde_json::Value>(&payload)?;
    let links = json
        .get("Objects")
        .and_then(|value| value.as_array())
        .and_then(|objects| objects.first())
        .and_then(|object| object.get("Links"))
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(links)
}

async fn download_ipfs_file(
    state: &AppState,
    ipfs_path: &str,
    destination_file: &Path,
) -> anyhow::Result<()> {
    if let Some(parent) = destination_file.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Unable to create file directory {}", parent.display()))?;
    }

    let endpoint = format!("{}/api/v0/cat", state.ipfs_api_url.trim_end_matches('/'));

    let mut request = state.http.post(endpoint).query(&[("arg", ipfs_path)]);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request.send().await?;
    let status = response.status();

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Unable to download IPFS file {ipfs_path}: {body}"));
    }

    let bytes = response.bytes().await?;
    fs::write(destination_file, &bytes).await.with_context(|| {
        format!(
            "Unable to write synced IPFS file to {}",
            destination_file.display()
        )
    })?;

    Ok(())
}

fn build_relay_socket_url(relay_server_url: &str, device_token: &str) -> anyhow::Result<Url> {
    let mut url = Url::parse(relay_server_url)
        .with_context(|| format!("Unable to parse relay server URL {relay_server_url}"))?;

    let next_scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        other => {
            return Err(anyhow!(
                "Unsupported relay server scheme for websocket transport: {other}"
            ));
        }
    };

    url.set_scheme(next_scheme)
        .map_err(|_| anyhow!("Unable to convert relay server URL to websocket scheme"))?;
    url.set_path("/desktop-relay");
    url.query_pairs_mut()
        .clear()
        .append_pair("role", "device")
        .append_pair("deviceToken", device_token);

    Ok(url)
}

async fn clear_relay_link(state: &AppState) -> anyhow::Result<()> {
    {
        let mut config = state.config.write().await;
        config.relay_enabled = false;
        config.relay_device_id = None;
        config.relay_device_label = None;
        config.relay_device_token = None;
        config.relay_last_error = None;
    }

    persist_bridge_config(state).await
}

async fn perform_relay_unlink(state: &AppState, notify_server: bool) -> anyhow::Result<()> {
    let config = { state.config.read().await.clone() };

    if notify_server
        && !config.relay_server_url.trim().is_empty()
        && config
            .relay_device_token
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
            == false
    {
        let endpoint = format!(
            "{}/api/relay/bridge/unlink",
            trim_trailing_slash(&config.relay_server_url)
        );

        let _ = state
            .http
            .post(endpoint)
            .json(&serde_json::json!({
                "deviceToken": config.relay_device_token,
            }))
            .send()
            .await;
    }

    clear_relay_link(state).await
}

async fn send_relay_inventory(
    state: &AppState,
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> anyhow::Result<()> {
    let snapshot = list_local_pin_inventory(state).await?;
    let payload = RelayInventoryMessage {
        r#type: "device.inventory",
        items: snapshot.items,
    };

    socket
        .send(Message::Text(
            serde_json::to_string(&payload)
                .context("Unable to encode relay inventory")?
                .into(),
        ))
        .await
        .context("Unable to send relay inventory to the archive server")?;

    Ok(())
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

    info!(
        "relay websocket connected: {} ({})",
        response.status(),
        config.relay_server_url
    );

    send_relay_inventory(state, &mut socket).await?;

    while let Some(message) = socket.next().await {
        match message? {
            Message::Text(text) => {
                let value = serde_json::from_str::<serde_json::Value>(&text)
                    .context("Unable to parse relay websocket message")?;
                let message_type = value
                    .get("type")
                    .and_then(|item| item.as_str())
                    .unwrap_or("");

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
                                let pins = pin_work_payload(state, input)
                                    .await
                                    .map_err(|error| anyhow!(error.message))?;
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
                            payload
                                .reason
                                .unwrap_or_else(|| "connection closed".to_string())
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

async fn remember_relay_success(
    state: &AppState,
    device_id: Option<String>,
    device_label: Option<String>,
) -> anyhow::Result<()> {
    {
        let mut config = state.config.write().await;
        config.relay_last_connected_at = Some(Utc::now());
        config.relay_last_error = None;

        if let Some(value) = device_id {
            config.relay_device_id = Some(value);
        }

        if let Some(value) = device_label {
            config.relay_device_label = Some(value);
        }
    }

    persist_bridge_config(state).await
}

async fn remember_relay_error(state: &AppState, message: String) -> anyhow::Result<()> {
    {
        let mut config = state.config.write().await;
        config.relay_last_error = Some(message);
    }

    persist_bridge_config(state).await
}

async fn repair_watched_pins(state: &AppState) -> anyhow::Result<RepairCycleOutcome> {
    let watched = {
        state
            .persistent
            .read()
            .await
            .watched_pins
            .values()
            .cloned()
            .collect::<Vec<_>>()
    };

    let mut outcome = RepairCycleOutcome::default();

    for pin in watched {
        match is_cid_pinned(state, &pin.cid).await {
            Ok(true) => {
                mark_pin_checked(state, &pin.cid, None)
                    .await
                    .map_err(|error| anyhow!(error.message))?;
                outcome.healthy += 1;
            }
            Ok(false) => {
                warn!("cid {} missing from ipfs pinset, repairing", pin.cid);

                let label = pin.label.clone();
                match pin_single_cid(state, &pin.cid, label.clone()).await {
                    Ok(result) => {
                        remember_watched_pin(
                            state,
                            WatchPinInput {
                                cid: pin.cid.clone(),
                                label,
                                source_kind: pin.source_kind.clone(),
                                title: pin.title.clone(),
                                contract_address: pin.contract_address.clone(),
                                token_id: pin.token_id.clone(),
                                foundation_url: pin.foundation_url.clone(),
                                artist_username: pin.artist_username.clone(),
                                account_address: pin.account_address.clone(),
                                username: pin.username.clone(),
                            },
                            Some(result.pin_reference),
                            None,
                            true,
                        )
                        .await
                        .map_err(|error| anyhow!(error.message))?;
                        outcome.repaired += 1;
                    }
                    Err(error) => {
                        let message = error.message.clone();
                        mark_pin_checked(state, &pin.cid, Some(message.clone()))
                            .await
                            .map_err(|write_error| anyhow!(write_error.message))?;
                        outcome.failed += 1;
                    }
                }
            }
            Err(error) => {
                let message = error.to_string();
                let _ = mark_pin_checked(state, &pin.cid, Some(message.clone())).await;
                outcome.failed += 1;
            }
        }
    }

    {
        let mut persistent = state.persistent.write().await;
        persistent.last_repair_cycle_at = Some(Utc::now());
        persistent.repair_cycle_count += 1;
        persistent.updated_at = Some(Utc::now());
    }

    persist_bridge_state(state).await?;
    Ok(outcome)
}

async fn is_cid_pinned(state: &AppState, cid: &str) -> anyhow::Result<bool> {
    let endpoint = format!(
        "{}/api/v0/pin/ls?arg={}",
        state.ipfs_api_url.trim_end_matches('/'),
        cid.trim()
    );

    let mut request = state.http.post(endpoint);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request.send().await?;
    if response.status().is_success() {
        return Ok(true);
    }

    let body = response.text().await.unwrap_or_default();
    if body.to_lowercase().contains("not pinned") {
        return Ok(false);
    }

    Err(anyhow!("Unable to verify pin status for {cid}: {body}"))
}

async fn pin_single_cid(
    state: &AppState,
    cid: &str,
    label: Option<String>,
) -> Result<PinCidResult, AppError> {
    let trimmed = cid.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("CID is required"));
    }

    let endpoint = format!(
        "{}/api/v0/pin/add?arg={}",
        state.ipfs_api_url.trim_end_matches('/'),
        trimmed
    );

    let mut request = state.http.post(endpoint);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request
        .send()
        .await
        .map_err(|error| AppError::internal(anyhow!("Failed to reach IPFS API: {error}")))?;

    let status = response.status();

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::internal(anyhow!(
            "IPFS pin failed with status {}: {}",
            status,
            body
        )));
    }

    let payload = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| AppError::internal(anyhow!("Unable to decode IPFS response: {error}")))?;

    let pin_reference = payload
        .get("Pinned")
        .and_then(|value| value.as_str())
        .or_else(|| {
            payload
                .get("Pins")
                .and_then(|value| value.as_array())
                .and_then(|pins| pins.first())
                .and_then(|value| value.as_str())
        })
        .unwrap_or(trimmed)
        .to_string();

    Ok(PinCidResult {
        cid: trimmed.to_string(),
        label,
        pinned: true,
        provider: "kubo",
        pin_reference,
        requested_at: Utc::now(),
    })
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

const PAGE_STYLE: &str = r#"
:root {
  --bg: #fafaf7;
  --surface: #ffffff;
  --surface-alt: #f4f1ea;
  --surface-quiet: #fbf8f1;
  --ink: #111111;
  --body: #2a2a2a;
  --muted: #6a6a66;
  --subtle: #989892;
  --line: #e8e3db;
  --line-strong: #d7d1c6;
  --ok: #2e6f4a;
  --warn: #9a6a1e;
  --err: #9a2a2a;
  --tint-ok: #e8f1ea;
  --tint-warn: #f3efe4;
  --tint-err: #f3e5e5;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #0e0e0b;
    --surface: #161613;
    --surface-alt: #1c1b17;
    --surface-quiet: #1a1915;
    --ink: #f1ece0;
    --body: #d4cfc2;
    --muted: #8e897e;
    --subtle: #625e55;
    --line: #25241f;
    --line-strong: #34322c;
    --ok: #8cc69f;
    --warn: #d6b278;
    --err: #d69797;
    --tint-ok: #1c2a22;
    --tint-warn: #2a241a;
    --tint-err: #2c1e1e;
  }
}
* { box-sizing: border-box; }
html, body { margin: 0; }
body {
  background: var(--bg);
  color: var(--body);
  font-family: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
  font-size: 15px;
  line-height: 1.55;
  -webkit-font-smoothing: antialiased;
}
h1, h2, h3 {
  font-family: ui-serif, Georgia, "Times New Roman", serif;
  color: var(--ink);
  letter-spacing: -0.01em;
  margin: 0;
  font-weight: 500;
}
h1 { font-size: clamp(2rem, 4vw, 2.75rem); line-height: 1.08; }
h2 { font-size: 1.5rem; line-height: 1.2; }
h3 { font-size: 1.1rem; line-height: 1.3; }
p { margin: 0; }
a { color: var(--ink); text-decoration: underline; text-underline-offset: 3px; text-decoration-color: var(--line-strong); }
a:hover { text-decoration-color: var(--ink); }
code {
  font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  font-size: 0.82em;
  background: var(--surface-alt);
  border: 1px solid var(--line);
  border-radius: 4px;
  padding: 1px 6px;
  color: var(--body);
  word-break: break-all;
}
.eyebrow {
  font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  font-size: 0.68rem;
  text-transform: uppercase;
  letter-spacing: 0.3em;
  color: var(--muted);
  margin: 0;
}
.site-nav {
  border-bottom: 1px solid var(--line);
  background: var(--bg);
  position: sticky;
  top: 0;
  z-index: 10;
}
.site-nav-inner {
  max-width: 1100px;
  margin: 0 auto;
  padding: 14px 24px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
}
.brand {
  font-family: ui-serif, Georgia, serif;
  color: var(--ink);
  text-decoration: none;
  font-size: 1.05rem;
  letter-spacing: -0.01em;
}
.brand small {
  color: var(--muted);
  font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  font-size: 0.68rem;
  text-transform: uppercase;
  letter-spacing: 0.22em;
  margin-left: 10px;
}
.nav-links { display: flex; gap: 22px; align-items: center; }
.nav-links a {
  color: var(--body);
  text-decoration: none;
  font-size: 0.88rem;
}
.nav-links a:hover { color: var(--ink); }
main.shell {
  max-width: 1100px;
  margin: 0 auto;
  padding: 40px 24px 80px;
}
main.shell.narrow { max-width: 720px; }
.stack > * + * { margin-top: 36px; }
.section-head {
  border-bottom: 1px solid var(--line);
  padding-bottom: 24px;
}
.section-head h1 { margin-top: 10px; max-width: 30ch; }
.section-head .lead { margin-top: 14px; max-width: 62ch; color: var(--body); }
.stats {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
  gap: 14px;
}
.stat {
  background: var(--surface);
  border: 1px solid var(--line);
  border-radius: 6px;
  padding: 18px;
}
.stat .eyebrow { margin-bottom: 10px; }
.stat-value {
  font-family: ui-serif, Georgia, serif;
  color: var(--ink);
  font-size: 2rem;
  line-height: 1.05;
  margin: 0 0 6px;
  font-weight: 500;
}
.stat-body { color: var(--body); font-size: 0.86rem; }
.card {
  background: var(--surface);
  border: 1px solid var(--line);
  border-radius: 6px;
  padding: 22px;
}
.card .eyebrow + h2 { margin-top: 8px; }
.card h2 + p { margin-top: 10px; }
.two-col { display: grid; gap: 16px; grid-template-columns: 1fr; }
@media (min-width: 820px) {
  .two-col { grid-template-columns: 1fr 1fr; }
}
.field { display: block; margin-top: 14px; }
.field > span {
  display: block;
  font-size: 0.82rem;
  color: var(--muted);
  margin-bottom: 6px;
}
input[type="text"],
input:not([type]),
input[type="url"] {
  width: 100%;
  padding: 10px 12px;
  border-radius: 5px;
  border: 1px solid var(--line-strong);
  background: var(--surface);
  color: var(--ink);
  font: inherit;
  font-size: 0.9rem;
}
input:focus { outline: 2px solid var(--ink); outline-offset: 1px; border-color: var(--ink); }
.btn {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 10px 18px;
  border-radius: 999px;
  border: 1px solid var(--ink);
  background: var(--ink);
  color: var(--bg);
  font: inherit;
  font-size: 0.88rem;
  font-weight: 500;
  cursor: pointer;
  text-decoration: none;
}
.btn:hover { opacity: 0.88; }
.btn.ghost {
  background: transparent;
  color: var(--ink);
  border-color: var(--line-strong);
}
.btn.ghost:hover { border-color: var(--ink); opacity: 1; }
.btn-row { margin-top: 18px; display: flex; gap: 10px; flex-wrap: wrap; }
.flash {
  border-radius: 6px;
  padding: 12px 16px;
  border: 1px solid;
  font-size: 0.9rem;
}
.flash.ok   { background: var(--tint-ok);   border-color: rgba(46,111,74,0.25);  color: var(--ok); }
.flash.warn { background: var(--tint-warn); border-color: rgba(154,106,30,0.25); color: var(--warn); }
.flash.err  { background: var(--tint-err);  border-color: rgba(154,42,42,0.25);  color: var(--err); }
.pill {
  display: inline-flex;
  padding: 4px 10px;
  border-radius: 999px;
  font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  font-size: 0.66rem;
  text-transform: uppercase;
  letter-spacing: 0.22em;
  border: 1px solid var(--line-strong);
  color: var(--muted);
  background: var(--surface);
}
.pill.ok   { background: var(--tint-ok);   color: var(--ok);   border-color: transparent; }
.pill.warn { background: var(--tint-warn); color: var(--warn); border-color: transparent; }
.pill.err  { background: var(--tint-err);  color: var(--err);  border-color: transparent; }
.table-wrap {
  border: 1px solid var(--line);
  border-radius: 6px;
  overflow-x: auto;
  background: var(--surface);
}
table { width: 100%; border-collapse: collapse; }
th, td {
  padding: 12px 14px;
  text-align: left;
  border-bottom: 1px solid var(--line);
  vertical-align: top;
  font-size: 0.86rem;
  color: var(--body);
}
tbody tr:last-child td { border-bottom: 0; }
tbody tr:hover { background: var(--surface-quiet); }
th {
  font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  font-size: 0.64rem;
  text-transform: uppercase;
  letter-spacing: 0.24em;
  color: var(--muted);
  background: var(--surface-alt);
  font-weight: 500;
  white-space: nowrap;
}
.empty {
  border: 1px dashed var(--line-strong);
  border-radius: 6px;
  padding: 36px 20px;
  text-align: center;
  color: var(--muted);
  background: var(--surface);
}
.muted { color: var(--muted); }
.cid {
  font-family: ui-monospace, Menlo, Consolas, monospace;
  font-size: 0.78rem;
  color: var(--muted);
  word-break: break-all;
}
.footer {
  margin-top: 60px;
  padding-top: 24px;
  border-top: 1px solid var(--line);
  color: var(--muted);
  font-size: 0.82rem;
}
ul.plain { list-style: none; padding: 0; margin: 0; }
ul.plain li { padding: 8px 0; border-bottom: 1px solid var(--line); }
ul.plain li:last-child { border-bottom: 0; }
hr.sep { border: 0; border-top: 1px solid var(--line); margin: 36px 0; }
.kv { display: grid; grid-template-columns: max-content 1fr; gap: 6px 16px; font-size: 0.88rem; }
.kv dt {
  color: var(--muted);
  font-family: ui-monospace, Menlo, Consolas, monospace;
  font-size: 0.72rem;
  text-transform: uppercase;
  letter-spacing: 0.2em;
}
.kv dd { margin: 0; color: var(--body); }
"#;

fn render_page(title: &str, body_html: &str) -> String {
    let mut out = String::with_capacity(6144 + body_html.len());
    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n");
    out.push_str("  <meta charset=\"utf-8\" />\n");
    out.push_str("  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n");
    out.push_str("  <title>");
    out.push_str(&escape_html(title));
    out.push_str("</title>\n  <style>");
    out.push_str(PAGE_STYLE);
    out.push_str("</style>\n</head>\n<body>\n");
    out.push_str(
        "  <nav class=\"site-nav\"><div class=\"site-nav-inner\">\
         <a class=\"brand\" href=\"/\">Foundation Share Bridge<small>pin companion</small></a>\
         <div class=\"nav-links\">\
         <a href=\"/#status\">Status</a>\
         <a href=\"/#inventory\">Pins</a>\
         <a href=\"/#connection\">Connection</a>\
         </div>\
         </div></nav>\n",
    );
    out.push_str(body_html);
    out.push_str("\n</body>\n</html>");
    out
}

fn format_timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
