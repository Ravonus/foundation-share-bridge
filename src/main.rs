use std::{
    collections::{HashMap, HashSet, VecDeque},
    env,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, anyhow};
use async_recursion::async_recursion;
use axum::{
    Form, Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{
        StatusCode,
        header::{HeaderName, HeaderValue},
    },
    middleware,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt, stream};
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

const DEFAULT_RELAY_SERVER_URL: &str = "https://foundation.agorix.io";
const FOUNDATION_SITE_HOSTNAME: &str = "foundation.agorix.io";
const FOUNDATION_SOCKET_HOSTNAME: &str = "socket-foundation.agorix.io";
const INVENTORY_PAGE_SIZE: usize = 12;
const INVENTORY_MAX_PAGE_SIZE: usize = 24;
const PUBLIC_UTILITY_GATEWAY_BASE_URL: &str = "https://dweb.link";
const VERIFY_CONCURRENCY: usize = 6;
const MAX_DISCOVERY_TEXT_BYTES: usize = 512 * 1024;
const MAX_DEPENDENCY_DISCOVERY_DEPTH: usize = 2;
const MAX_DEPENDENCY_SCAN_CIDS: usize = 24;

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
    operation: Arc<RwLock<OperationStatus>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationStatus {
    phase: String,
    detail: Option<String>,
    progress_current: Option<usize>,
    progress_total: Option<usize>,
    started_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl OperationStatus {
    fn idle() -> Self {
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

    fn busy(phase: &str, detail: Option<String>, total: Option<usize>) -> Self {
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
    #[serde(default)]
    preferred_file_name: Option<String>,
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
    #[serde(default)]
    retry_attempts: u32,
    #[serde(default)]
    next_retry_at: Option<DateTime<Utc>>,
    #[serde(default)]
    error_category: Option<String>,
    #[serde(default)]
    provider_count: Option<usize>,
    #[serde(default)]
    provider_checked_at: Option<DateTime<Utc>>,
    #[serde(default)]
    custom_tags: Vec<String>,
    #[serde(default)]
    remote_pinned: bool,
    #[serde(default)]
    remote_pin_service: Option<String>,
    #[serde(default)]
    remote_pin_last_attempt_at: Option<DateTime<Utc>>,
    #[serde(default)]
    remote_pin_last_error: Option<String>,
    #[serde(default)]
    final_failure_reported_at: Option<DateTime<Utc>>,
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
    #[serde(default)]
    storage_quota_gb: Option<f64>,
    #[serde(default)]
    max_retry_attempts: Option<u32>,
    #[serde(default)]
    remote_pinning_enabled: bool,
    #[serde(default)]
    remote_pinning_service_name: Option<String>,
    #[serde(default)]
    remote_pinning_service_url: Option<String>,
    #[serde(default)]
    remote_pinning_access_token: Option<String>,
    #[serde(default)]
    onboarded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
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
    storage: StorageSnapshot,
    operation: OperationStatus,
    remote_pinning_enabled: bool,
    onboarded: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageSnapshot {
    repo_size_bytes: Option<u64>,
    storage_max_bytes: Option<u64>,
    num_objects: Option<u64>,
    synced_bytes_on_disk: u64,
    quota_gb: Option<f64>,
    quota_used_fraction: Option<f64>,
    ipfs_daemon_reachable: bool,
    checked_at: DateTime<Utc>,
}

async fn add_private_network_access_header(mut response: Response) -> Response {
    response.headers_mut().insert(
        HeaderName::from_static("access-control-allow-private-network"),
        HeaderValue::from_static("true"),
    );
    response
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

#[derive(Debug, Deserialize)]
struct PinsPageQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PinsPageResponse {
    total: usize,
    pinned_count: usize,
    managed_count: usize,
    next_cursor: Option<String>,
    items: Vec<PinInventoryItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinMetadataField {
    label: String,
    value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinMetadataView {
    description: Option<String>,
    fields: Vec<PinMetadataField>,
    attributes: Vec<PinMetadataField>,
    raw_json: String,
    raw_json_truncated: bool,
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
    preview_local_gateway_url: Option<String>,
    preview_public_gateway_url: Option<String>,
    media_kind: Option<String>,
    metadata_view: Option<PinMetadataView>,
    metadata_cid: Option<String>,
    media_cid: Option<String>,
    #[serde(default)]
    related_cids: Vec<String>,
    last_synced_at: Option<DateTime<Utc>>,
    last_sync_error: Option<String>,
    sync_count: u64,
    #[serde(default)]
    retry_attempts: u32,
    #[serde(default)]
    next_retry_at: Option<DateTime<Utc>>,
    #[serde(default)]
    error_category: Option<String>,
    #[serde(default)]
    provider_count: Option<usize>,
    #[serde(default)]
    provider_checked_at: Option<DateTime<Utc>>,
    #[serde(default)]
    custom_tags: Vec<String>,
    #[serde(default)]
    remote_pinned: bool,
    #[serde(default)]
    remote_pin_service: Option<String>,
    #[serde(default)]
    remote_pin_last_error: Option<String>,
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

#[derive(Debug, Deserialize)]
struct UnwatchPinsRequest {
    cids: Vec<String>,
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
struct UnwatchPinsResponse {
    removed: usize,
    missing: usize,
    message: &'static str,
}

#[derive(Debug, Serialize)]
struct SyncNowResponse {
    synced: usize,
    failed: usize,
    skipped: usize,
    message: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
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
    storage_quota_gb: Option<f64>,
    max_retry_attempts: Option<u32>,
    remote_pinning_enabled: bool,
    remote_pinning_service_name: Option<String>,
    remote_pinning_service_url: Option<String>,
    remote_pinning_access_token_configured: bool,
    onboarded_at: Option<DateTime<Utc>>,
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
    #[serde(default)]
    storage_quota_gb: Option<Option<f64>>,
    #[serde(default)]
    max_retry_attempts: Option<Option<u32>>,
    #[serde(default)]
    remote_pinning_enabled: Option<bool>,
    #[serde(default)]
    remote_pinning_service_name: Option<Option<String>>,
    #[serde(default)]
    remote_pinning_service_url: Option<Option<String>>,
    #[serde(default)]
    remote_pinning_access_token: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
struct UpdateBridgeConfigFormRequest {
    download_root_dir: String,
    sync_enabled: Option<String>,
    local_gateway_base_url: String,
    public_gateway_base_url: String,
    relay_enabled: Option<String>,
    relay_server_url: String,
    relay_device_name: String,
    #[serde(default)]
    storage_quota_gb: Option<String>,
    #[serde(default)]
    max_retry_attempts: Option<String>,
    #[serde(default)]
    remote_pinning_enabled: Option<String>,
    #[serde(default)]
    remote_pinning_service_name: Option<String>,
    #[serde(default)]
    remote_pinning_service_url: Option<String>,
    #[serde(default)]
    remote_pinning_access_token: Option<String>,
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
    autolink: Option<String>,
    linked: Option<String>,
    unlinked: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SettingsPageQuery {
    saved: Option<String>,
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
    preferred_file_name: Option<String>,
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
        .layer(middleware::map_response(add_private_network_access_header))
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
            .unwrap_or_else(|| DEFAULT_RELAY_SERVER_URL.to_string()),
        relay_device_name: env::var("BRIDGE_DEVICE_NAME")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Foundation desktop helper".to_string()),
        relay_device_id: None,
        relay_device_label: None,
        relay_device_token: None,
        relay_last_connected_at: None,
        relay_last_error: None,
        storage_quota_gb: env::var("BRIDGE_STORAGE_QUOTA_GB")
            .ok()
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|value| *value > 0.0),
        max_retry_attempts: env::var("BRIDGE_MAX_RETRY_ATTEMPTS")
            .ok()
            .and_then(|value| value.trim().parse::<u32>().ok()),
        remote_pinning_enabled: env::var("BRIDGE_REMOTE_PINNING_ENABLED")
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false),
        remote_pinning_service_name: env::var("BRIDGE_REMOTE_PINNING_SERVICE_NAME")
            .ok()
            .filter(|value| !value.trim().is_empty()),
        remote_pinning_service_url: env::var("BRIDGE_REMOTE_PINNING_SERVICE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty()),
        remote_pinning_access_token: env::var("BRIDGE_REMOTE_PINNING_ACCESS_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty()),
        onboarded_at: None,
    }
}

fn bridge_config_uses_yaml(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("yaml" | "yml")
    )
}

fn parse_bridge_config(contents: &str, path: &Path) -> anyhow::Result<BridgeConfig> {
    if bridge_config_uses_yaml(path) {
        serde_yaml::from_str::<BridgeConfig>(contents)
            .or_else(|_| serde_json::from_str::<BridgeConfig>(contents))
            .with_context(|| format!("Unable to parse bridge config from {}", path.display()))
    } else {
        serde_json::from_str::<BridgeConfig>(contents)
            .or_else(|_| serde_yaml::from_str::<BridgeConfig>(contents))
            .with_context(|| format!("Unable to parse bridge config from {}", path.display()))
    }
}

fn legacy_bridge_json_path(path: &Path) -> Option<PathBuf> {
    if !bridge_config_uses_yaml(path) {
        return None;
    }

    let file_stem = path.file_stem()?.to_str()?;
    let parent = path.parent()?;
    Some(parent.join(format!("{file_stem}.json")))
}

async fn load_bridge_config(path: &Path, state_file: &Path) -> anyhow::Result<BridgeConfig> {
    let defaults = default_bridge_config(state_file);

    match fs::read_to_string(path).await {
        Ok(contents) => {
            let mut config = parse_bridge_config(&contents, path)?;

            if config.download_root_dir.trim().is_empty() {
                config.download_root_dir = defaults.download_root_dir;
            }
            if config.local_gateway_base_url.trim().is_empty() {
                config.local_gateway_base_url = defaults.local_gateway_base_url;
            }
            if config.public_gateway_base_url.trim().is_empty() {
                config.public_gateway_base_url = defaults.public_gateway_base_url;
            }
            if config.relay_server_url.trim().is_empty() {
                config.relay_server_url = defaults.relay_server_url;
            }
            if config.relay_device_name.trim().is_empty() {
                config.relay_device_name = defaults.relay_device_name;
            }

            Ok(config)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(legacy_path) = legacy_bridge_json_path(path) {
                match fs::read_to_string(&legacy_path).await {
                    Ok(contents) => {
                        let mut config = parse_bridge_config(&contents, &legacy_path)?;

                        if config.download_root_dir.trim().is_empty() {
                            config.download_root_dir = defaults.download_root_dir;
                        }
                        if config.local_gateway_base_url.trim().is_empty() {
                            config.local_gateway_base_url = defaults.local_gateway_base_url;
                        }
                        if config.public_gateway_base_url.trim().is_empty() {
                            config.public_gateway_base_url = defaults.public_gateway_base_url;
                        }
                        if config.relay_server_url.trim().is_empty() {
                            config.relay_server_url = defaults.relay_server_url;
                        }
                        if config.relay_device_name.trim().is_empty() {
                            config.relay_device_name = defaults.relay_device_name;
                        }

                        return Ok(config);
                    }
                    Err(legacy_error) if legacy_error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(legacy_error) => {
                        return Err(legacy_error).with_context(|| {
                            format!(
                                "Unable to read legacy bridge config at {}",
                                legacy_path.display()
                            )
                        });
                    }
                }
            }

            Ok(defaults)
        }
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

    if bridge_config_uses_yaml(&state.config_file) {
        let yaml =
            serde_yaml::to_string(&snapshot).context("Unable to encode bridge config as YAML")?;
        fs::write(&state.config_file, yaml).await.with_context(|| {
            format!(
                "Unable to write bridge config to {}",
                state.config_file.display()
            )
        })?;
    } else {
        let json =
            serde_json::to_vec_pretty(&snapshot).context("Unable to encode bridge config")?;
        fs::write(&state.config_file, json).await.with_context(|| {
            format!(
                "Unable to write bridge config to {}",
                state.config_file.display()
            )
        })?;
    }

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

fn relay_is_connected(config: &BridgeConfig) -> bool {
    config.relay_enabled
        && config
            .relay_last_error
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        && !config
            .relay_device_token
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
}

fn build_config_response(state: &AppState, config: &BridgeConfig) -> BridgeConfigResponse {
    BridgeConfigResponse {
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
        storage_quota_gb: config.storage_quota_gb,
        max_retry_attempts: config.max_retry_attempts,
        remote_pinning_enabled: config.remote_pinning_enabled,
        remote_pinning_service_name: config.remote_pinning_service_name.clone(),
        remote_pinning_service_url: config.remote_pinning_service_url.clone(),
        remote_pinning_access_token_configured: config
            .remote_pinning_access_token
            .as_deref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false),
        onboarded_at: config.onboarded_at,
    }
}

fn render_inventory_table_rows(items: &[PinInventoryItem], limit: usize) -> String {
    items.iter()
        .take(limit)
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
                        r#"<a href="{}" target="_blank" rel="noreferrer">pinned</a>"#,
                        escape_html(url)
                    ));
                }
                parts.push(format!(
                    r#"<a href="{}" target="_blank" rel="noreferrer">public</a>"#,
                    escape_html(&build_public_utility_gateway_url(&pin.cid))
                ));
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
}

fn render_inventory_fallback_table(items: &[PinInventoryItem]) -> String {
    let rows = render_inventory_table_rows(items, INVENTORY_PAGE_SIZE);
    if rows.is_empty() {
        return r#"<div class="empty">No pins yet. Once the archive site hands you something to rescue, it will appear here.</div>"#.to_string();
    }

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

    let relay_connected = relay_is_connected(&config);
    let relay_server_value = query
        .relay_server_url
        .as_deref()
        .unwrap_or(config.relay_server_url.as_str());
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

    let connection_status = if relay_connected {
        "Live"
    } else {
        "Not linked"
    };
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
        <p class="lead">Foundation-linked roots keep their rescue context. Other IPFS items show up here too. "Open pinned" uses your external gateway base URL, and every card also includes a separate public IPFS link for quick sharing.</p>
      </div>
      <div style="margin-top: 20px;">{inventory_body}</div>
    </section>

    <p class="footer">Agorix share bridge · local-only · {repair_interval}s repair interval · last cycle {last_repair}</p>
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
            format!(
                r#"<div class="flash warn">Relay note: {}</div>"#,
                escape_html(message)
            )
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
        let hostname_help = match detected_public_ipv4.as_deref() {
            Some(ip) => format!(
                "If you own a hostname, point an A record like <code>ipfs.example.com</code> at <code>{}</code>, then click \"Use hostname\".",
                escape_html(ip)
            ),
            None => "Type a DDNS or custom hostname here, then click \"Use hostname\" to fill the gateway URL below.".to_string(),
        };

        let ip_action = detected_public_ipv4
            .as_deref()
            .map(|ip| {
                format!(
                    r#"<button type="button" class="btn ghost" id="gateway_fill_ip" data-gateway-url="{}">Use detected IP</button>"#,
                    escape_html(&build_direct_ip_gateway_base_url(ip))
                )
            })
            .unwrap_or_else(|| {
                r#"<span class="muted gateway-helper-note">Public IPv4 detection is unavailable right now. You can still type a hostname or edit the full URL manually.</span>"#.to_string()
            });

        format!(
            r#"<div class="gateway-helper">
  <p class="eyebrow">Quick fill</p>
  <h3>Hostname first, IP if needed</h3>
  <p class="muted settings-copy">Use a hostname if you have one. If not, you can fill the external gateway with your detected public IP and adjust the full URL below if your gateway uses a different port or protocol.</p>
  <label class="field">
    <span>Gateway hostname</span>
    <input type="text" id="gateway_hostname_input" placeholder="ipfs.example.com or studio.ddns.net" />
    <small class="field-help">{hostname_help}</small>
  </label>
  <div class="btn-row gateway-helper-actions">
    <button type="button" class="btn ghost" id="gateway_fill_hostname">Use hostname</button>
    {ip_action}
  </div>
  <p class="muted gateway-helper-preview">Next pinned gateway base: <code id="gateway_helper_preview_value">{current_external_gateway}</code></p>
</div>"#,
            hostname_help = hostname_help,
            ip_action = ip_action,
            current_external_gateway = current_external_gateway,
        )
    };

    let gateway_dns_card = match detected_public_ipv4.as_deref() {
        Some(ip) => {
            let direct_ip_gateway = build_direct_ip_gateway_base_url(ip);
            format!(
                r#"<section class="card">
          <p class="eyebrow">Gateway DNS</p>
          <h2>Point a hostname at this helper</h2>
          <p class="muted settings-copy">If you want cleaner pinned links, create an A record for something like <code>ipfs.example.com</code> and point it at your public IP. If you do not have a hostname yet, the detected IP button above fills a direct gateway URL for you.</p>
          <dl class="kv" style="margin-top: 16px;">
            <dt>Detected public IP</dt><dd><code>{ip}</code></dd>
            <dt>Example A record</dt><dd><code>ipfs.example.com → {ip}</code></dd>
            <dt>Direct IP fallback</dt><dd><code>{direct_ip_gateway}</code></dd>
          </dl>
        </section>"#,
                ip = escape_html(ip),
                direct_ip_gateway = escape_html(&direct_ip_gateway),
            )
        }
        None => r#"<section class="card">
          <p class="eyebrow">Gateway DNS</p>
          <h2>Hostname or direct IP</h2>
          <p class="muted settings-copy">We could not detect a public IPv4 address right now, but the quick-fill controls still help: type a hostname you control to build the external gateway URL automatically, or edit the full URL manually if you already know your public IP.</p>
        </section>"#
            .to_string(),
    };

    let body = format!(
        r#"<main class="shell">
  <div class="stack">
    <section class="section-head">
      <p class="eyebrow">Bridge settings</p>
      <h1>Configure how this helper saves, tests, and opens pinned media.</h1>
      <p class="lead">These inputs edit the bridge&apos;s YAML config file behind the scenes. People should use this page, not hand-edit YAML, unless they want the advanced path.</p>
      <div class="btn-row">
        <a class="btn ghost" href="/">Back to dashboard</a>
        <span class="{relay_class}">{relay_label}</span>
      </div>
    </section>

    {flash}
    {relay_note}

    <section class="settings-layout">
      <form action="/settings/form" method="post" class="card settings-form">
        <div class="settings-block">
          <p class="eyebrow">Storage</p>
          <h2>Saved copies on disk</h2>
          <p class="muted settings-copy">Choose where synced copies live and whether the helper should maintain an on-disk mirror in addition to the IPFS pin.</p>
          <label class="field">
            <span>Download folder</span>
            <input type="text" name="download_root_dir" value="{download_root_dir}" placeholder="/Users/you/Archive Pins" />
            <small class="field-help">When sync is enabled, each watched CID is mirrored into this folder.</small>
          </label>
          <label class="checkbox-row">
            <input type="checkbox" name="sync_enabled" value="1" {sync_checked} />
            <span>
              <strong>Keep synced copies on disk</strong>
              <small>Turn this on if you want the helper to write rescued media into your download folder too.</small>
            </span>
          </label>
        </div>

        <div class="settings-block">
          <p class="eyebrow">Gateways</p>
          <h2>Preview and share links</h2>
          <p class="muted settings-copy">The helper uses the local gateway for on-machine previews and the external gateway for the "Open pinned" button. Inventory cards also show a separate public IPFS link for quick sharing.</p>
          <label class="field">
            <span>Local gateway base URL</span>
            <input type="url" name="local_gateway_base_url" value="{local_gateway_base_url}" placeholder="http://127.0.0.1:8080" />
            <small class="field-help">Used by the local browser UI when it can reach your own gateway.</small>
          </label>
          <label class="field">
            <span>External pinned gateway URL</span>
            <input type="url" id="public_gateway_base_url" name="public_gateway_base_url" value="{public_gateway_base_url}" placeholder="https://ipfs.io" />
            <small class="field-help">Point this at your own hostname, DDNS name, reverse proxy, or direct public IP gateway so "Open pinned" uses your route.</small>
          </label>
          {gateway_helper}
        </div>

        <div class="settings-block">
          <p class="eyebrow">Relay</p>
          <h2>Archive connection</h2>
          <p class="muted settings-copy">These settings control how this helper pairs with the archive site and how it identifies itself when linked.</p>
          <label class="checkbox-row">
            <input type="checkbox" name="relay_enabled" value="1" {relay_checked} />
            <span>
              <strong>Enable archive relay link</strong>
              <small>Leave this on for normal use so archive pages can hand work to this helper.</small>
            </span>
          </label>
          <label class="field">
            <span>Archive server URL</span>
            <input type="url" name="relay_server_url" value="{relay_server_url}" placeholder="https://foundation.agorix.io" />
            <small class="field-help">Changing this resets the current relay pairing so the helper can link to the new server cleanly.</small>
          </label>
          <label class="field">
            <span>Desktop name</span>
            <input type="text" name="relay_device_name" value="{relay_device_name}" placeholder="Studio MacBook" />
            <small class="field-help">This is what the archive site shows when choosing where to send saved works.</small>
          </label>
        </div>

        <div class="btn-row settings-actions">
          <button type="submit" class="btn">Save settings</button>
          <a class="btn ghost" href="/">Cancel</a>
        </div>
      </form>

      <aside class="settings-side">
        <section class="card">
          <p class="eyebrow">Saved backend</p>
          <h2>Still YAML underneath</h2>
          <p class="muted settings-copy">The helper still stores everything in <code>bridge-config.yaml</code>. This page simply edits that file for the user.</p>
          <dl class="kv" style="margin-top: 16px;">
            <dt>Config file</dt><dd><code>{yaml_path}</code></dd>
            <dt>Relay status</dt><dd>{relay_label}</dd>
            <dt>Linked device</dt><dd>{linked_device}</dd>
            <dt>Last linked</dt><dd>{linked_at}</dd>
          </dl>
        </section>

        <section class="card">
          <p class="eyebrow">External URLs</p>
          <h2>Friendly pinned links</h2>
          <p class="muted settings-copy">If you want the helper to open media through your own hostname, point DNS at this machine and use that hostname for the external gateway. If you do not have a hostname yet, the helper can fill a direct external-IP URL instead. The inventory UI keeps that route separate from the public IPFS fallback link.</p>
        </section>
        {gateway_dns_card}
      </aside>
    </section>
  </div>
</main>
<script>{settings_gateway_script}</script>"#,
        relay_class = relay_status_class,
        relay_label = escape_html(relay_status_label),
        flash = flash_block,
        relay_note = relay_note,
        download_root_dir = escape_html(&config.download_root_dir),
        sync_checked = sync_checked,
        local_gateway_base_url = escape_html(&config.local_gateway_base_url),
        public_gateway_base_url = escape_html(&config.public_gateway_base_url),
        relay_checked = relay_checked,
        relay_server_url = escape_html(&config.relay_server_url),
        relay_device_name = escape_html(&config.relay_device_name),
        yaml_path = escape_html(&yaml_path),
        linked_device = escape_html(linked_device),
        linked_at = escape_html(&linked_at),
        gateway_helper = gateway_helper,
        gateway_dns_card = gateway_dns_card,
        settings_gateway_script = SETTINGS_GATEWAY_HELPER_SCRIPT,
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
        if trimmed.is_empty() {
            None
        } else {
            trimmed.parse::<u32>().ok()
        }
    });

    let name = input.remote_pinning_service_name.as_deref().map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let url = input.remote_pinning_service_url.as_deref().map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let token = input.remote_pinning_access_token.as_deref().map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
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
        Err(error) => Ok(Redirect::to(&format!(
            "/settings?error={}",
            encode_query_component(&error.message)
        ))),
    }
}

async fn apply_config_update(
    state: &AppState,
    input: UpdateBridgeConfigRequest,
) -> Result<BridgeConfigResponse, AppError> {
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
            let trimmed = relay_server_url.trim();
            if trimmed.is_empty() {
                return Err(AppError::bad_request("relay_server_url cannot be empty"));
            }
            if trimmed != config.relay_server_url.trim() {
                config.relay_enabled = false;
                config.relay_device_id = None;
                config.relay_device_label = None;
                config.relay_device_token = None;
                config.relay_last_connected_at = None;
                config.relay_last_error =
                    Some("Relay server changed. Pair this desktop app again.".to_string());
            }
            config.relay_server_url = trimmed.to_string();
        }

        if let Some(relay_device_name) = input.relay_device_name {
            let trimmed = relay_device_name.trim();
            if trimmed.is_empty() {
                return Err(AppError::bad_request("relay_device_name cannot be empty"));
            }
            config.relay_device_name = trimmed.to_string();
        }

        if let Some(quota) = input.storage_quota_gb {
            config.storage_quota_gb = quota;
        }

        if let Some(retries) = input.max_retry_attempts {
            config.max_retry_attempts = retries;
        }

        if let Some(enabled) = input.remote_pinning_enabled {
            config.remote_pinning_enabled = enabled;
        }

        if let Some(name) = input.remote_pinning_service_name {
            config.remote_pinning_service_name = name.filter(|value| !value.trim().is_empty());
        }

        if let Some(url) = input.remote_pinning_service_url {
            config.remote_pinning_service_url = url.filter(|value| !value.trim().is_empty());
        }

        if let Some(token) = input.remote_pinning_access_token {
            config.remote_pinning_access_token = token.filter(|value| !value.trim().is_empty());
        }

        if config.onboarded_at.is_none() {
            config.onboarded_at = Some(Utc::now());
        }
    }

    persist_bridge_config(state)
        .await
        .map_err(AppError::internal)?;

    let config = state.config.read().await;
    Ok(build_config_response(state, &config))
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

async fn list_pins_page(
    State(state): State<AppState>,
    Query(query): Query<PinsPageQuery>,
) -> Result<Json<PinsPageResponse>, AppError> {
    let cursor = parse_inventory_cursor(query.cursor.as_deref());
    let limit = resolve_inventory_page_size(query.limit);
    let response = list_local_pin_inventory_page(&state, cursor, limit)
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

    Ok(Json(VerifyPinsResponse {
        checked_at: Utc::now(),
        results: ordered_results,
    }))
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

    persist_bridge_state(&state)
        .await
        .map_err(AppError::internal)?;

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

fn parse_inventory_cursor(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0)
}

fn resolve_inventory_page_size(raw: Option<usize>) -> usize {
    raw.unwrap_or(INVENTORY_PAGE_SIZE)
        .clamp(1, INVENTORY_MAX_PAGE_SIZE)
}

fn unique_trimmed_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && seen.insert(value.clone()))
        .collect()
}

fn build_pins_page_response(
    response: PinsResponse,
    cursor: usize,
    limit: usize,
) -> PinsPageResponse {
    let total = response.total;
    let start = cursor.min(total);
    let end = start.saturating_add(limit).min(total);
    let items = response.items[start..end].to_vec();

    PinsPageResponse {
        total,
        pinned_count: response.pinned_count,
        managed_count: response.managed_count,
        next_cursor: (end < total).then(|| end.to_string()),
        items,
    }
}

fn collect_inventory_descriptors(
    pinset: &HashMap<String, String>,
    persistent: &BridgePersistentState,
) -> Vec<InventoryEntryDescriptor> {
    let mut grouped_work_members = HashMap::<String, Vec<InventorySourcePin>>::new();
    let mut descriptors = Vec::new();

    for watched in persistent.watched_pins.values() {
        let source = InventorySourcePin {
            cid: watched.cid.clone(),
            pinned: pinset.contains_key(&watched.cid),
            pin_type: pinset.get(&watched.cid).cloned(),
            watched: watched.clone(),
        };

        if let Some(group_key) = inventory_work_group_key(&source.watched) {
            grouped_work_members.entry(group_key).or_default().push(source);
        } else {
            descriptors.push(InventoryEntryDescriptor::Single(source));
        }
    }

    for members in grouped_work_members.into_values() {
        descriptors.push(InventoryEntryDescriptor::Work(members));
    }

    descriptors.sort_by(|left, right| right.added_at().cmp(&left.added_at()));
    descriptors
}

async fn build_inventory_item_from_descriptor(
    state: &AppState,
    config: &BridgeConfig,
    descriptor: &InventoryEntryDescriptor,
) -> PinInventoryItem {
    match descriptor {
        InventoryEntryDescriptor::Single(source) => {
            build_single_inventory_item(config, source.clone())
        }
        InventoryEntryDescriptor::Work(members) => {
            build_work_inventory_item(state, config, members).await
        }
    }
}

async fn remember_pin_verification(
    state: &AppState,
    result: &PinVerification,
) -> Result<(), AppError> {
    mark_pin_checked(state, &result.cid, result.error.clone()).await
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

    let mut response = request.send().await?;
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
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => body.extend_from_slice(&chunk),
            Ok(None) => break,
            Err(error) => {
                if body.is_empty() {
                    return Err(anyhow!(
                        "Unable to read IPFS {endpoint} response body: {error}"
                    ));
                }
                break;
            }
        }
    }

    let body = String::from_utf8_lossy(&body);
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
                    preferred_file_name: None,
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
                    preferred_file_name: None,
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
                    preferred_file_name: None,
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
            existing.preferred_file_name =
                input.preferred_file_name.or(existing.preferred_file_name.clone());
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
                existing.retry_attempts = 0;
                existing.next_retry_at = None;
                existing.error_category = None;
                existing.final_failure_reported_at = None;
            }
        } else {
            persistent.watched_pins.insert(
                input.cid.clone(),
                WatchedPin {
                    cid: input.cid,
                    label: input.label,
                    preferred_file_name: input.preferred_file_name,
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
                    retry_attempts: 0,
                    next_retry_at: None,
                    error_category: None,
                    provider_count: None,
                    provider_checked_at: None,
                    custom_tags: Vec::new(),
                    remote_pinned: false,
                    remote_pin_service: None,
                    remote_pin_last_attempt_at: None,
                    remote_pin_last_error: None,
                    final_failure_reported_at: None,
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

fn build_public_utility_gateway_url(cid: &str) -> String {
    build_gateway_url(PUBLIC_UTILITY_GATEWAY_BASE_URL, cid)
}

fn build_direct_ip_gateway_base_url(ip: &str) -> String {
    format!("http://{}:8080", ip.trim())
}

async fn detect_public_ipv4(state: &AppState) -> Option<String> {
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

#[derive(Debug, Clone)]
struct InventorySourcePin {
    cid: String,
    pinned: bool,
    pin_type: Option<String>,
    watched: WatchedPin,
}

#[derive(Debug, Clone)]
enum InventoryEntryDescriptor {
    Single(InventorySourcePin),
    Work(Vec<InventorySourcePin>),
}

impl InventoryEntryDescriptor {
    fn added_at(&self) -> DateTime<Utc> {
        match self {
            Self::Single(source) => source.watched.added_at,
            Self::Work(members) => members
                .iter()
                .map(|member| member.watched.added_at)
                .max()
                .unwrap_or_else(Utc::now),
        }
    }

    fn pinned(&self) -> bool {
        match self {
            Self::Single(source) => source.pinned,
            Self::Work(members) => members.iter().all(|member| member.pinned),
        }
    }
}

#[derive(Debug, Default)]
struct ResolvedWorkDisplay {
    local_open_url: Option<String>,
    public_open_url: Option<String>,
    preview_local_url: Option<String>,
    preview_public_url: Option<String>,
    media_kind: Option<String>,
    metadata_view: Option<PinMetadataView>,
}

fn inventory_work_group_key(pin: &WatchedPin) -> Option<String> {
    if pin.source_kind != "work" {
        return None;
    }

    if let (Some(contract_address), Some(token_id)) =
        (pin.contract_address.as_deref(), pin.token_id.as_deref())
    {
        let contract = contract_address.trim().to_ascii_lowercase();
        let token = token_id.trim();
        if !contract.is_empty() && !token.is_empty() {
            return Some(format!("work:{contract}:{token}"));
        }
    }

    if let Some(foundation_url) = pin.foundation_url.as_deref() {
        let trimmed = foundation_url.trim();
        if !trimmed.is_empty() {
            return Some(format!("work-url:{trimmed}"));
        }
    }

    if let Some(title) = pin.title.as_deref() {
        let trimmed = title.trim();
        if !trimmed.is_empty() {
            return Some(format!(
                "work-title:{}:{}",
                trimmed.to_ascii_lowercase(),
                pin.artist_username
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase()
            ));
        }
    }

    None
}

fn first_present_string<I>(values: I) -> Option<String>
where
    I: IntoIterator<Item = Option<String>>,
{
    values
        .into_iter()
        .flatten()
        .find(|value| !value.trim().is_empty())
}

fn max_timestamp_by<F>(members: &[InventorySourcePin], accessor: F) -> Option<DateTime<Utc>>
where
    F: Fn(&InventorySourcePin) -> Option<DateTime<Utc>>,
{
    members.iter().filter_map(accessor).max()
}

fn first_present_error<F>(members: &[InventorySourcePin], accessor: F) -> Option<String>
where
    F: Fn(&InventorySourcePin) -> Option<&String>,
{
    members
        .iter()
        .filter_map(accessor)
        .find(|value| !value.trim().is_empty())
        .cloned()
}

fn related_cids_from_members(members: &[InventorySourcePin]) -> Vec<String> {
    let mut seen = HashSet::new();
    members
        .iter()
        .map(|member| member.cid.clone())
        .filter(|cid| seen.insert(cid.clone()))
        .collect()
}

fn parse_ipfs_path(raw: &str) -> Option<(String, String)> {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed.strip_prefix("ipfs/").unwrap_or(trimmed);
    let mut parts = normalized.splitn(2, '/');
    let cid = parts.next()?.trim();
    if cid.is_empty() {
        return None;
    }

    let relative_path = parts.next().unwrap_or("").trim_matches('/').to_string();
    Some((cid.to_string(), relative_path))
}

fn parse_ipfs_reference(raw: &str) -> Option<(String, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix("ipfs://") {
        return parse_ipfs_path(rest);
    }

    if let Some(rest) = trimmed.strip_prefix("/ipfs/") {
        return parse_ipfs_path(rest);
    }

    let url = Url::parse(trimmed).ok()?;
    if let Some(host) = url.host_str() {
        if let Some((cid, _)) = host.split_once(".ipfs.") {
            return Some((cid.to_string(), url.path().trim_matches('/').to_string()));
        }
    }

    let path = url.path();
    let index = path.find("/ipfs/")?;
    parse_ipfs_path(&path[(index + "/ipfs/".len())..])
}

fn build_gateway_asset_url(base: &str, cid: &str, relative_path: &str) -> String {
    let cleaned = relative_path.trim().trim_matches('/');
    if cleaned.is_empty() {
        return build_gateway_url(base, cid);
    }

    format!(
        "{}/ipfs/{}/{}",
        trim_trailing_slash(base),
        cid.trim(),
        cleaned
    )
}

fn normalize_asset_url_for_gateway(raw: &str, gateway_base: &str) -> String {
    if let Some((cid, relative_path)) = parse_ipfs_reference(raw) {
        return build_gateway_asset_url(gateway_base, &cid, &relative_path);
    }

    raw.to_string()
}

fn json_string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(|entry| entry.as_str())
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToOwned::to_owned)
}

fn nested_json_value<'a>(
    value: &'a serde_json::Value,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path {
        current = current.as_object()?.get(*segment)?;
    }
    Some(current)
}

fn collect_url_candidates(value: Option<&serde_json::Value>) -> Vec<String> {
    let mut candidates = Vec::new();
    let entries = match value {
        Some(serde_json::Value::Array(items)) => items.iter().collect::<Vec<_>>(),
        Some(other) => vec![other],
        None => Vec::new(),
    };

    for entry in entries {
        let Some(record) = entry.as_object() else {
            continue;
        };

        for key in [
            "uri",
            "url",
            "src",
            "href",
            "animation_url",
            "animation",
            "image",
            "image_url",
        ] {
            let candidate = json_string(record.get(key));
            if let Some(candidate) = candidate.filter(|value| !candidates.contains(value)) {
                candidates.push(candidate);
            }
        }
    }

    candidates
}

fn metadata_image_url(metadata: &serde_json::Value) -> Option<String> {
    first_present_string([
        json_string(metadata.get("image")),
        json_string(metadata.get("image_url")),
        json_string(nested_json_value(metadata, &["properties", "image"])),
        json_string(nested_json_value(metadata, &["properties", "image_url"])),
        json_string(nested_json_value(metadata, &["displayUri"])),
        json_string(nested_json_value(metadata, &["display_uri"])),
        json_string(nested_json_value(metadata, &["thumbnailUri"])),
        json_string(nested_json_value(metadata, &["thumbnail_uri"])),
    ])
}

fn metadata_file_url(metadata: &serde_json::Value) -> Option<String> {
    first_present_string(
        collect_url_candidates(nested_json_value(metadata, &["media", "files"]))
            .into_iter()
            .map(Some)
            .chain(
                collect_url_candidates(nested_json_value(metadata, &["properties", "files"]))
                    .into_iter()
                    .map(Some),
            )
            .chain(
                collect_url_candidates(nested_json_value(metadata, &["files"]))
                    .into_iter()
                    .map(Some),
            )
            .chain(
                collect_url_candidates(nested_json_value(metadata, &["formats"]))
                    .into_iter()
                    .map(Some),
            ),
    )
}

fn metadata_primary_media_url(metadata: &serde_json::Value) -> Option<String> {
    first_present_string([
        json_string(metadata.get("animation_url")),
        json_string(metadata.get("animation")),
        json_string(nested_json_value(metadata, &["media", "uri"])),
        json_string(nested_json_value(metadata, &["media", "url"])),
        json_string(nested_json_value(
            metadata,
            &["properties", "animation_url"],
        )),
        json_string(nested_json_value(metadata, &["properties", "animation"])),
        json_string(nested_json_value(metadata, &["artifactUri"])),
        json_string(nested_json_value(metadata, &["artifact_uri"])),
        json_string(nested_json_value(metadata, &["content", "uri"])),
        json_string(nested_json_value(metadata, &["content", "url"])),
    ])
}

fn json_display_value(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(boolean) => Some(boolean.to_string()),
        serde_json::Value::Array(values) => {
            let joined = values
                .iter()
                .filter_map(|entry| json_display_value(Some(entry)))
                .collect::<Vec<_>>();
            (!joined.is_empty()).then(|| joined.join(", "))
        }
        serde_json::Value::Object(record) => serde_json::to_string(record)
            .ok()
            .filter(|value| !value.is_empty()),
    }
}

fn metadata_description(metadata: &serde_json::Value) -> Option<String> {
    first_present_string([
        json_string(metadata.get("description")),
        json_string(nested_json_value(metadata, &["properties", "description"])),
        json_string(nested_json_value(metadata, &["content", "description"])),
    ])
}

fn metadata_external_url(metadata: &serde_json::Value) -> Option<String> {
    first_present_string([
        json_string(metadata.get("external_url")),
        json_string(metadata.get("externalUrl")),
        json_string(nested_json_value(metadata, &["properties", "external_url"])),
        json_string(nested_json_value(metadata, &["properties", "externalUrl"])),
    ])
}

fn metadata_mime_type(metadata: &serde_json::Value) -> Option<String> {
    first_present_string([
        json_string(metadata.get("mimeType")),
        json_string(metadata.get("mime_type")),
        json_string(nested_json_value(metadata, &["content", "mimeType"])),
        json_string(nested_json_value(metadata, &["content", "mime_type"])),
        json_string(nested_json_value(metadata, &["properties", "mimeType"])),
        json_string(nested_json_value(metadata, &["properties", "mime_type"])),
    ])
}

fn build_metadata_summary_fields(
    metadata: &serde_json::Value,
    image_raw: Option<&str>,
    media_raw: Option<&str>,
) -> Vec<PinMetadataField> {
    let mut fields = Vec::new();
    let mut seen = HashSet::new();

    let mut push_field = |label: &str, value: Option<String>| {
        let Some(value) = value.filter(|entry| !entry.trim().is_empty()) else {
            return;
        };
        let dedupe_key = format!("{}:{}", label.to_ascii_lowercase(), value);
        if seen.insert(dedupe_key) {
            fields.push(PinMetadataField {
                label: label.to_string(),
                value,
            });
        }
    };

    push_field("Metadata title", json_string(metadata.get("name")));
    push_field("External URL", metadata_external_url(metadata));
    push_field("Preview image", image_raw.map(ToOwned::to_owned));
    push_field(
        "Primary media",
        media_raw
            .filter(|entry| Some(*entry) != image_raw)
            .map(ToOwned::to_owned),
    );
    push_field("Mime type", metadata_mime_type(metadata));

    fields
}

fn build_metadata_attribute_fields(metadata: &serde_json::Value) -> Vec<PinMetadataField> {
    let mut attributes = Vec::new();
    let mut seen = HashSet::new();

    for candidate in [
        metadata.get("attributes"),
        nested_json_value(metadata, &["properties", "attributes"]),
        metadata.get("traits"),
    ]
    .into_iter()
    .flatten()
    {
        let Some(entries) = candidate.as_array() else {
            continue;
        };

        for (index, entry) in entries.iter().enumerate() {
            let Some(record) = entry.as_object() else {
                continue;
            };

            let label = first_present_string([
                json_string(record.get("trait_type")),
                json_string(record.get("type")),
                json_string(record.get("name")),
                json_string(record.get("key")),
            ])
            .unwrap_or_else(|| format!("Trait {}", index + 1));

            let value = first_present_string([
                json_display_value(record.get("value")),
                json_display_value(record.get("display_value")),
                json_display_value(record.get("trait_value")),
            ]);
            let Some(value) = value.filter(|entry| !entry.trim().is_empty()) else {
                continue;
            };

            let dedupe_key = format!("{}:{}", label.to_ascii_lowercase(), value);
            if seen.insert(dedupe_key) {
                attributes.push(PinMetadataField { label, value });
            }
        }
    }

    attributes
}

fn build_metadata_view(
    metadata: &serde_json::Value,
    image_raw: Option<&str>,
    media_raw: Option<&str>,
) -> Option<PinMetadataView> {
    const MAX_METADATA_JSON_CHARS: usize = 24_000;

    let mut raw_json = serde_json::to_string_pretty(metadata).ok()?;
    let mut raw_json_truncated = false;
    if raw_json.chars().count() > MAX_METADATA_JSON_CHARS {
        raw_json = raw_json.chars().take(MAX_METADATA_JSON_CHARS).collect();
        raw_json.push_str("\n…");
        raw_json_truncated = true;
    }

    let description = metadata_description(metadata);
    let fields = build_metadata_summary_fields(metadata, image_raw, media_raw);
    let attributes = build_metadata_attribute_fields(metadata);

    if description.is_none() && fields.is_empty() && attributes.is_empty() && raw_json.is_empty() {
        return None;
    }

    Some(PinMetadataView {
        description,
        fields,
        attributes,
        raw_json,
        raw_json_truncated,
    })
}

fn detect_media_kind_from_text(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let markers: [(&str, &[&str]); 6] = [
        ("VIDEO", &[".mp4", ".mov", ".webm", "video"]),
        ("AUDIO", &[".mp3", ".wav", ".ogg", ".aac", "audio"]),
        (
            "IMAGE",
            &[".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", "image"],
        ),
        ("HTML", &[".html", "text/html"]),
        (
            "MODEL",
            &[
                ".glb",
                ".gltf",
                ".usdz",
                "model",
                "model/gltf",
                "model/vnd.usdz",
                "3d",
            ],
        ),
        ("JSON", &[".json", "application/json", "text/json"]),
    ];

    markers.iter().find_map(|(kind, entries)| {
        entries
            .iter()
            .any(|marker| lower.contains(marker))
            .then(|| (*kind).to_string())
    })
}

async fn detect_media_kind_for_url(
    state: &AppState,
    local_url: Option<&str>,
    hints: &[Option<String>],
) -> Option<String> {
    for value in hints.iter().flatten() {
        if let Some(kind) = detect_media_kind_from_text(value) {
            return Some(kind);
        }
    }

    let url = local_url?.trim();
    if url.is_empty() {
        return None;
    }

    let response = state
        .http
        .head(url)
        .timeout(Duration::from_secs(6))
        .send()
        .await
        .ok()?;

    if let Some(content_type) = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(detect_media_kind_from_text)
    {
        return Some(content_type);
    }

    detect_media_kind_from_text(response.url().as_str())
}

async fn fetch_ipfs_json(
    state: &AppState,
    ipfs_path: &str,
) -> anyhow::Result<Option<serde_json::Value>> {
    let endpoint = format!("{}/api/v0/cat", state.ipfs_api_url.trim_end_matches('/'));
    let mut request = state.http.post(endpoint).query(&[("arg", ipfs_path)]);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request.send().await?;
    if !response.status().is_success() {
        return Ok(None);
    }

    let body = response.bytes().await?;
    let parsed = serde_json::from_slice::<serde_json::Value>(&body).ok();
    Ok(parsed)
}

async fn resolve_single_child_path(
    state: &AppState,
    cid: &str,
    required_suffixes: &[&str],
) -> Option<String> {
    let links = list_ipfs_links(state, &format!("/ipfs/{}", cid.trim()))
        .await
        .ok()?;
    if links.is_empty() {
        return None;
    }

    let mut names = links
        .iter()
        .filter_map(|link| link.get("Name").and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .filter(|name| {
            required_suffixes.is_empty()
                || required_suffixes
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    names.dedup();
    (names.len() == 1).then(|| names.remove(0))
}

async fn load_work_metadata_record(
    state: &AppState,
    metadata_cid: &str,
    token_id: Option<&str>,
) -> Option<serde_json::Value> {
    let mut candidates = Vec::new();
    let cid = metadata_cid.trim();

    if let Some(token_id) = token_id.map(str::trim).filter(|value| !value.is_empty()) {
        candidates.push(format!("/ipfs/{cid}/{token_id}.json"));
        candidates.push(format!("/ipfs/{cid}/{token_id}"));
    }

    candidates.push(format!("/ipfs/{cid}/metadata.json"));
    if let Some(single_json_child) = resolve_single_child_path(state, cid, &[".json"]).await {
        candidates.push(format!("/ipfs/{cid}/{single_json_child}"));
    }
    candidates.push(format!("/ipfs/{cid}"));

    let mut seen = HashSet::new();
    for candidate in candidates {
        if !seen.insert(candidate.clone()) {
            continue;
        }
        if let Ok(Some(metadata)) = fetch_ipfs_json(state, &candidate).await {
            return Some(metadata);
        }
    }

    None
}

async fn resolve_work_display(
    state: &AppState,
    config: &BridgeConfig,
    metadata_cid: Option<&str>,
    media_cid: Option<&str>,
    token_id: Option<&str>,
) -> ResolvedWorkDisplay {
    let mut display = ResolvedWorkDisplay::default();

    let metadata = if let Some(metadata_cid) = metadata_cid.filter(|value| !value.trim().is_empty())
    {
        load_work_metadata_record(state, metadata_cid, token_id).await
    } else {
        None
    };

    let image_raw = metadata.as_ref().and_then(metadata_image_url);
    let media_raw = metadata.as_ref().and_then(|record| {
        let image = image_raw.clone();
        metadata_primary_media_url(record)
            .or_else(|| metadata_file_url(record))
            .or(image)
    });
    display.metadata_view = metadata
        .as_ref()
        .and_then(|record| build_metadata_view(record, image_raw.as_deref(), media_raw.as_deref()));

    if let Some(raw) = media_raw
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        display.local_open_url = Some(normalize_asset_url_for_gateway(
            raw,
            &config.local_gateway_base_url,
        ));
        display.public_open_url = Some(normalize_asset_url_for_gateway(
            raw,
            &config.public_gateway_base_url,
        ));
    } else if let Some(media_cid) = media_cid.filter(|value| !value.trim().is_empty()) {
        if let Some(child) = resolve_single_child_path(state, media_cid, &[]).await {
            display.local_open_url = Some(build_gateway_asset_url(
                &config.local_gateway_base_url,
                media_cid,
                &child,
            ));
            display.public_open_url = Some(build_gateway_asset_url(
                &config.public_gateway_base_url,
                media_cid,
                &child,
            ));
        } else {
            display.local_open_url =
                Some(build_gateway_url(&config.local_gateway_base_url, media_cid));
            display.public_open_url = Some(build_gateway_url(
                &config.public_gateway_base_url,
                media_cid,
            ));
        }
    } else if let Some(metadata_cid) = metadata_cid.filter(|value| !value.trim().is_empty()) {
        display.local_open_url = Some(build_gateway_url(
            &config.local_gateway_base_url,
            metadata_cid,
        ));
        display.public_open_url = Some(build_gateway_url(
            &config.public_gateway_base_url,
            metadata_cid,
        ));
    }

    if let Some(raw) = image_raw
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        display.preview_local_url = Some(normalize_asset_url_for_gateway(
            raw,
            &config.local_gateway_base_url,
        ));
        display.preview_public_url = Some(normalize_asset_url_for_gateway(
            raw,
            &config.public_gateway_base_url,
        ));
    }

    display.media_kind = detect_media_kind_for_url(
        state,
        display
            .local_open_url
            .as_deref()
            .or(display.preview_local_url.as_deref()),
        &[
            media_raw.clone(),
            image_raw.clone(),
            display.local_open_url.clone(),
            display.preview_local_url.clone(),
        ],
    )
    .await;

    if display.preview_local_url.is_none()
        && matches!(
            display.media_kind.as_deref(),
            Some("IMAGE") | Some("VIDEO") | Some("HTML") | Some("MODEL")
        )
    {
        display.preview_local_url = display.local_open_url.clone();
        display.preview_public_url = display.public_open_url.clone();
    }

    display
}

fn build_single_inventory_item(
    config: &BridgeConfig,
    source: InventorySourcePin,
) -> PinInventoryItem {
    let cid = source.cid.clone();

    PinInventoryItem {
        cid: cid.clone(),
        pinned: source.pinned,
        pin_type: source.pin_type.clone(),
        managed: true,
        label: source.watched.label.clone(),
        source_kind: Some(source.watched.source_kind.clone()),
        title: source.watched.title.clone(),
        contract_address: source.watched.contract_address.clone(),
        token_id: source.watched.token_id.clone(),
        foundation_url: source.watched.foundation_url.clone(),
        artist_username: source.watched.artist_username.clone(),
        account_address: source.watched.account_address.clone(),
        username: source.watched.username.clone(),
        added_at: Some(source.watched.added_at),
        last_verified_at: source.watched.last_verified_at,
        last_repaired_at: source.watched.last_repaired_at,
        last_error: source.watched.last_error.clone(),
        pin_reference: source.watched.pin_reference.clone(),
        verify_count: source.watched.verify_count,
        repair_count: source.watched.repair_count,
        sync_path: source.watched.sync_path.clone(),
        local_gateway_url: Some(build_gateway_url(&config.local_gateway_base_url, &cid)),
        public_gateway_url: Some(build_gateway_url(&config.public_gateway_base_url, &cid)),
        preview_local_gateway_url: Some(build_gateway_url(&config.local_gateway_base_url, &cid)),
        preview_public_gateway_url: Some(build_gateway_url(&config.public_gateway_base_url, &cid)),
        media_kind: None,
        metadata_view: None,
        metadata_cid: None,
        media_cid: None,
        related_cids: vec![cid],
        last_synced_at: source.watched.last_synced_at,
        last_sync_error: source.watched.last_sync_error.clone(),
        sync_count: source.watched.sync_count,
        retry_attempts: source.watched.retry_attempts,
        next_retry_at: source.watched.next_retry_at,
        error_category: source.watched.error_category.clone(),
        provider_count: source.watched.provider_count,
        provider_checked_at: source.watched.provider_checked_at,
        custom_tags: source.watched.custom_tags.clone(),
        remote_pinned: source.watched.remote_pinned,
        remote_pin_service: source.watched.remote_pin_service.clone(),
        remote_pin_last_error: source.watched.remote_pin_last_error.clone(),
    }
}

async fn build_work_inventory_item(
    state: &AppState,
    config: &BridgeConfig,
    members: &[InventorySourcePin],
) -> PinInventoryItem {
    let metadata_member = members
        .iter()
        .find(|member| matches!(member.watched.label.as_deref(), Some("metadata")));
    let media_member = members
        .iter()
        .find(|member| matches!(member.watched.label.as_deref(), Some("media")));
    let primary_member = media_member
        .or(metadata_member)
        .or_else(|| members.first())
        .expect("work groups always contain at least one member");

    let metadata_cid = metadata_member.map(|member| member.cid.clone());
    let media_cid = media_member.map(|member| member.cid.clone());
    let display = resolve_work_display(
        state,
        config,
        metadata_cid.as_deref(),
        media_cid.as_deref(),
        primary_member.watched.token_id.as_deref(),
    )
    .await;

    let primary_cid = media_cid
        .clone()
        .or(metadata_cid.clone())
        .unwrap_or_else(|| primary_member.cid.clone());

    PinInventoryItem {
        cid: primary_cid.clone(),
        pinned: members.iter().all(|member| member.pinned),
        pin_type: members
            .iter()
            .find_map(|member| member.pin_type.clone())
            .or_else(|| Some("watched".to_string())),
        managed: true,
        label: None,
        source_kind: Some("work".to_string()),
        title: first_present_string(members.iter().map(|member| member.watched.title.clone())),
        contract_address: first_present_string(
            members
                .iter()
                .map(|member| member.watched.contract_address.clone()),
        ),
        token_id: first_present_string(
            members.iter().map(|member| member.watched.token_id.clone()),
        ),
        foundation_url: first_present_string(
            members
                .iter()
                .map(|member| member.watched.foundation_url.clone()),
        ),
        artist_username: first_present_string(
            members
                .iter()
                .map(|member| member.watched.artist_username.clone()),
        ),
        account_address: first_present_string(
            members
                .iter()
                .map(|member| member.watched.account_address.clone()),
        ),
        username: first_present_string(
            members.iter().map(|member| member.watched.username.clone()),
        ),
        added_at: max_timestamp_by(members, |member| Some(member.watched.added_at)),
        last_verified_at: max_timestamp_by(members, |member| member.watched.last_verified_at),
        last_repaired_at: max_timestamp_by(members, |member| member.watched.last_repaired_at),
        last_error: first_present_error(members, |member| member.watched.last_error.as_ref()),
        pin_reference: primary_member.watched.pin_reference.clone(),
        verify_count: members
            .iter()
            .map(|member| member.watched.verify_count)
            .sum(),
        repair_count: members
            .iter()
            .map(|member| member.watched.repair_count)
            .sum(),
        sync_path: media_member
            .and_then(|member| member.watched.sync_path.clone())
            .or_else(|| metadata_member.and_then(|member| member.watched.sync_path.clone()))
            .or_else(|| primary_member.watched.sync_path.clone()),
        local_gateway_url: display.local_open_url.clone().or_else(|| {
            Some(build_gateway_url(
                &config.local_gateway_base_url,
                &primary_cid,
            ))
        }),
        public_gateway_url: display.public_open_url.clone().or_else(|| {
            Some(build_gateway_url(
                &config.public_gateway_base_url,
                &primary_cid,
            ))
        }),
        preview_local_gateway_url: display.preview_local_url.clone(),
        preview_public_gateway_url: display.preview_public_url.clone(),
        media_kind: display.media_kind.clone(),
        metadata_view: display.metadata_view.clone(),
        metadata_cid,
        media_cid,
        related_cids: related_cids_from_members(members),
        last_synced_at: max_timestamp_by(members, |member| member.watched.last_synced_at),
        last_sync_error: first_present_error(members, |member| {
            member.watched.last_sync_error.as_ref()
        }),
        sync_count: members.iter().map(|member| member.watched.sync_count).sum(),
        retry_attempts: members
            .iter()
            .map(|member| member.watched.retry_attempts)
            .max()
            .unwrap_or(0),
        next_retry_at: members
            .iter()
            .filter_map(|member| member.watched.next_retry_at)
            .min(),
        error_category: first_present_error(members, |member| {
            member.watched.error_category.as_ref()
        }),
        provider_count: members
            .iter()
            .filter_map(|member| member.watched.provider_count)
            .min(),
        provider_checked_at: max_timestamp_by(members, |member| {
            member.watched.provider_checked_at
        }),
        custom_tags: {
            let mut tags = Vec::new();
            let mut seen = HashSet::new();
            for member in members {
                for tag in &member.watched.custom_tags {
                    if seen.insert(tag.clone()) {
                        tags.push(tag.clone());
                    }
                }
            }
            tags
        },
        remote_pinned: members.iter().any(|member| member.watched.remote_pinned),
        remote_pin_service: first_present_string(
            members
                .iter()
                .map(|member| member.watched.remote_pin_service.clone()),
        ),
        remote_pin_last_error: first_present_error(members, |member| {
            member.watched.remote_pin_last_error.as_ref()
        }),
    }
}

async fn list_local_pin_inventory(state: &AppState) -> anyhow::Result<PinsResponse> {
    let pinset = list_kubo_pinset(state).await?;
    let persistent = state.persistent.read().await.clone();
    let config = state.config.read().await.clone();
    let descriptors = collect_inventory_descriptors(&pinset, &persistent);
    let mut items = Vec::with_capacity(descriptors.len());
    for descriptor in &descriptors {
        items.push(build_inventory_item_from_descriptor(state, &config, descriptor).await);
    }

    Ok(PinsResponse {
        total: descriptors.len(),
        pinned_count: descriptors.iter().filter(|descriptor| descriptor.pinned()).count(),
        managed_count: descriptors.len(),
        last_repair_cycle_at: persistent.last_repair_cycle_at,
        items,
    })
}

async fn list_local_pin_inventory_page(
    state: &AppState,
    cursor: usize,
    limit: usize,
) -> anyhow::Result<PinsPageResponse> {
    let pinset = list_kubo_pinset(state).await?;
    let persistent = state.persistent.read().await.clone();
    let config = state.config.read().await.clone();
    let descriptors = collect_inventory_descriptors(&pinset, &persistent);

    let total = descriptors.len();
    let start = cursor.min(total);
    let end = start.saturating_add(limit).min(total);
    let pinned_count = descriptors.iter().filter(|descriptor| descriptor.pinned()).count();

    let mut items = Vec::with_capacity(end.saturating_sub(start));
    for descriptor in &descriptors[start..end] {
        items.push(build_inventory_item_from_descriptor(state, &config, descriptor).await);
    }

    Ok(PinsPageResponse {
        total,
        pinned_count,
        managed_count: total,
        next_cursor: (end < total).then(|| end.to_string()),
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

    if matches!(url.host_str(), Some(FOUNDATION_SITE_HOSTNAME)) {
        url.set_host(Some(FOUNDATION_SOCKET_HOSTNAME))
            .map_err(|_| anyhow!("Unable to route relay websocket to the socket host"))?;
    }

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
        config.relay_last_connected_at = None;
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

    let total = watched.len();
    set_current_operation(
        state,
        OperationStatus::busy(
            "repairing",
            Some(format!("Checking {total} watched pin{}", if total == 1 { "" } else { "s" })),
            Some(total),
        ),
    )
    .await;

    let max_attempts = {
        state
            .config
            .read()
            .await
            .max_retry_attempts
            .unwrap_or(10)
    };

    let mut outcome = RepairCycleOutcome::default();
    let now = Utc::now();

    for (index, pin) in watched.into_iter().enumerate() {
        update_current_operation(
            state,
            Some(format!(
                "Checking {} ({} of {total})",
                pin.title.clone().unwrap_or_else(|| pin.cid.clone()),
                index + 1
            )),
            Some(index),
        )
        .await;

        if let Some(next_retry_at) = pin.next_retry_at {
            if next_retry_at > now {
                outcome.healthy += 1;
                continue;
            }
        }

        match is_cid_pinned(state, &pin.cid).await {
            Ok(true) => {
                record_pin_repaired(state, &pin).await?;
                outcome.healthy += 1;
            }
            Ok(false) => {
                warn!("cid {} missing from ipfs pinset, repairing", pin.cid);

                match pin_single_cid(state, &pin.cid, pin.label.clone()).await {
                    Ok(result) => {
                        remember_watched_pin(
                            state,
                            WatchPinInput {
                                cid: pin.cid.clone(),
                                label: pin.label.clone(),
                                preferred_file_name: pin.preferred_file_name.clone(),
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
                        record_pin_failure(state, &pin, &message, max_attempts).await?;
                        outcome.failed += 1;
                    }
                }
            }
            Err(error) => {
                let message = error.to_string();
                record_pin_failure(state, &pin, &message, max_attempts).await?;
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
    clear_current_operation(state).await;
    Ok(outcome)
}

async fn record_pin_repaired(state: &AppState, pin: &WatchedPin) -> anyhow::Result<()> {
    let cid = pin.cid.clone();
    {
        let mut persistent = state.persistent.write().await;
        let now = Utc::now();
        if let Some(existing) = persistent.watched_pins.get_mut(&cid) {
            existing.last_verified_at = Some(now);
            existing.last_error = None;
            existing.error_category = None;
            existing.retry_attempts = 0;
            existing.next_retry_at = None;
            existing.final_failure_reported_at = None;
            existing.verify_count += 1;
        }
        persistent.updated_at = Some(now);
    }
    persist_bridge_state(state).await
}

async fn record_pin_failure(
    state: &AppState,
    pin: &WatchedPin,
    message: &str,
    max_attempts: u32,
) -> anyhow::Result<()> {
    let (category_label, _hint) = categorize_pin_error(message);
    let next_attempt = pin.retry_attempts.saturating_add(1);
    let next_retry_at = compute_next_retry_at(state, next_attempt).await;
    let should_try_remote = next_attempt >= max_attempts
        && category_label != "invalid_cid"
        && category_label != "unauthorized";

    let mut remote_service: Option<String> = None;
    let mut remote_error: Option<String> = None;

    if should_try_remote {
        let hint = pin.title.clone().or_else(|| Some(pin.cid.clone()));
        match submit_to_remote_pinning_service(state, &pin.cid, hint.as_deref()).await {
            Ok(Some(service)) => {
                info!("remote pin service accepted {} via {}", pin.cid, service);
                remote_service = Some(service);
            }
            Ok(None) => {}
            Err(error) => {
                warn!("remote pin service rejected {}: {}", pin.cid, error);
                remote_error = Some(error.to_string());
            }
        }
    }

    let mut should_notify_relay = false;
    {
        let mut persistent = state.persistent.write().await;
        let now = Utc::now();
        if let Some(existing) = persistent.watched_pins.get_mut(&pin.cid) {
            existing.last_verified_at = Some(now);
            existing.last_error = Some(message.to_string());
            existing.error_category = Some(category_label.to_string());
            existing.retry_attempts = next_attempt;
            existing.next_retry_at = Some(next_retry_at);
            existing.verify_count += 1;

            if let Some(service) = &remote_service {
                existing.remote_pinned = true;
                existing.remote_pin_service = Some(service.clone());
                existing.remote_pin_last_attempt_at = Some(now);
                existing.remote_pin_last_error = None;
            } else if let Some(err) = &remote_error {
                existing.remote_pin_last_error = Some(err.clone());
                existing.remote_pin_last_attempt_at = Some(now);
            }

            if next_attempt >= max_attempts && existing.final_failure_reported_at.is_none() {
                existing.final_failure_reported_at = Some(now);
                should_notify_relay = true;
            }
        }
        persistent.updated_at = Some(now);
    }

    persist_bridge_state(state).await?;

    if should_notify_relay {
        if let Some(latest) = state.persistent.read().await.watched_pins.get(&pin.cid).cloned() {
            if let Err(error) = send_relay_pin_failure(state, &latest, message).await {
                warn!("relay pin-failure callback failed for {}: {error}", pin.cid);
            }
        }
    }

    Ok(())
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct KuboRepoStat {
    #[serde(rename = "RepoSize")]
    repo_size: Option<u64>,
    #[serde(rename = "StorageMax")]
    storage_max: Option<u64>,
    #[serde(rename = "NumObjects")]
    num_objects: Option<u64>,
    #[serde(rename = "RepoPath")]
    repo_path: Option<String>,
}

async fn fetch_kubo_repo_stat(state: &AppState) -> anyhow::Result<KuboRepoStat> {
    let endpoint = format!("{}/api/v0/repo/stat", state.ipfs_api_url.trim_end_matches('/'));
    let mut request = state.http.post(endpoint).timeout(Duration::from_secs(8));
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }
    let response = request.send().await?;
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Unable to read IPFS repo/stat: {body}"));
    }
    Ok(response.json::<KuboRepoStat>().await?)
}

#[async_recursion]
async fn sum_dir_size(path: &Path) -> u64 {
    let Ok(metadata) = fs::metadata(path).await else { return 0; };
    if metadata.is_file() {
        return metadata.len();
    }
    if !metadata.is_dir() {
        return 0;
    }
    let Ok(mut entries) = fs::read_dir(path).await else { return 0; };
    let mut total = 0u64;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let child = entry.path();
        total = total.saturating_add(sum_dir_size(&child).await);
    }
    total
}

async fn measure_synced_bytes_on_disk(state: &AppState) -> u64 {
    let paths = {
        state
            .persistent
            .read()
            .await
            .watched_pins
            .values()
            .filter_map(|pin| pin.sync_path.clone())
            .collect::<Vec<_>>()
    };
    let mut total = 0u64;
    for path in paths {
        total = total.saturating_add(sum_dir_size(&PathBuf::from(path)).await);
    }
    total
}

async fn build_storage_snapshot(state: &AppState) -> StorageSnapshot {
    let (repo_size, storage_max, num_objects, ipfs_daemon_reachable) =
        match fetch_kubo_repo_stat(state).await {
            Ok(stat) => (stat.repo_size, stat.storage_max, stat.num_objects, true),
            Err(_) => (None, None, None, false),
        };
    let synced_bytes = measure_synced_bytes_on_disk(state).await;
    let quota_gb = { state.config.read().await.storage_quota_gb };
    let quota_used_fraction = match (quota_gb, repo_size) {
        (Some(gb), Some(bytes)) if gb > 0.0 => {
            let max_bytes = gb * 1_000_000_000.0;
            if max_bytes > 0.0 { Some((bytes as f64) / max_bytes) } else { None }
        }
        _ => None,
    };
    StorageSnapshot {
        repo_size_bytes: repo_size,
        storage_max_bytes: storage_max,
        num_objects,
        synced_bytes_on_disk: synced_bytes,
        quota_gb,
        quota_used_fraction,
        ipfs_daemon_reachable,
        checked_at: Utc::now(),
    }
}

fn categorize_pin_error(message: &str) -> (&'static str, &'static str) {
    let lower = message.to_ascii_lowercase();
    if lower.contains("connection refused")
        || lower.contains("failed to reach ipfs")
        || lower.contains("failed to connect")
        || lower.contains("connection reset")
        || lower.contains("dial tcp")
    {
        return (
            "daemon_unreachable",
            "The local IPFS daemon is not responding. Start Kubo and retry.",
        );
    }
    if lower.contains("deadline exceeded")
        || lower.contains("timeout")
        || lower.contains("timed out")
    {
        return (
            "timeout",
            "The IPFS network took too long to answer. Try again in a minute.",
        );
    }
    if lower.contains("no providers")
        || lower.contains("could not find provider")
        || lower.contains("no route to host")
    {
        return (
            "no_providers",
            "No peers know about this CID yet. A remote pinning service can keep a copy.",
        );
    }
    if lower.contains("not pinned") {
        return ("not_pinned", "The CID is not pinned locally. The next cycle will pin it.");
    }
    if lower.contains("invalid") || lower.contains("not a valid cid") {
        return ("invalid_cid", "The CID looks malformed. Re-request the share.");
    }
    if lower.contains("unauthorized") || lower.contains("forbidden") || lower.contains("401") || lower.contains("403") {
        return ("unauthorized", "The IPFS API rejected the request. Verify IPFS_API_AUTH_HEADER.");
    }
    if lower.contains("disk") || lower.contains("no space") || lower.contains("quota") {
        return ("disk_full", "The IPFS datastore cannot accept more data. Free space or raise the quota.");
    }
    ("unknown", "Cause not recognized. Check the detail for the raw message.")
}

async fn compute_next_retry_at(state: &AppState, attempt: u32) -> DateTime<Utc> {
    let cap_attempts = {
        state.config.read().await.max_retry_attempts.unwrap_or(10)
    };
    let effective = attempt.min(cap_attempts).min(14);
    let base = 30u64.saturating_mul(1u64 << effective.min(10));
    let capped = base.min(60 * 60 * 6);
    Utc::now() + chrono::Duration::seconds(capped as i64)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnoseResponse {
    cid: String,
    pinned_locally: bool,
    provider_count: usize,
    reachable_on_dht: bool,
    error_category: Option<String>,
    error_hint: Option<String>,
    last_error: Option<String>,
    raw_error: Option<String>,
    checked_at: DateTime<Utc>,
    gateway_local_ok: Option<bool>,
    gateway_public_ok: Option<bool>,
}

async fn probe_gateway(client: &Client, url: &str) -> Option<bool> {
    let response = client
        .head(url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    Some(response.status().is_success() || response.status().is_redirection())
}

async fn check_gateway_reachability(state: &AppState, cid: &str) -> (Option<bool>, Option<bool>) {
    let (local_base, public_base) = {
        let config = state.config.read().await;
        (config.local_gateway_base_url.clone(), config.public_gateway_base_url.clone())
    };
    let local = probe_gateway(&state.http, &build_gateway_url(&local_base, cid)).await;
    let public = probe_gateway(&state.http, &build_gateway_url(&public_base, cid)).await;
    (local, public)
}

async fn diagnose_pin(state: &AppState, cid: &str) -> DiagnoseResponse {
    let checked_at = Utc::now();
    let pinned_locally = matches!(is_cid_pinned(state, cid).await, Ok(true));
    let provider_result = check_cid_network_providers(state, cid).await;
    let _ = remember_pin_verification(state, &provider_result).await;

    let (last_error, stored_category) = {
        let persistent = state.persistent.read().await;
        persistent
            .watched_pins
            .get(cid)
            .map(|pin| (pin.last_error.clone(), pin.error_category.clone()))
            .unwrap_or((None, None))
    };

    let combined_error = provider_result.error.clone().or(last_error.clone());
    let (category, hint) = combined_error
        .as_deref()
        .map(categorize_pin_error)
        .map(|(cat, hint)| (Some(cat.to_string()), Some(hint.to_string())))
        .unwrap_or_else(|| (stored_category.clone(), None));

    let (gateway_local_ok, gateway_public_ok) = check_gateway_reachability(state, cid).await;

    DiagnoseResponse {
        cid: cid.to_string(),
        pinned_locally,
        provider_count: provider_result.provider_count,
        reachable_on_dht: provider_result.reachable,
        error_category: category,
        error_hint: hint,
        last_error,
        raw_error: provider_result.error,
        checked_at,
        gateway_local_ok,
        gateway_public_ok,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GatewayHealthResponse {
    local_gateway_base_url: String,
    public_gateway_base_url: String,
    utility_gateway_base_url: &'static str,
    local_ok: Option<bool>,
    public_ok: Option<bool>,
    utility_ok: Option<bool>,
    checked_at: DateTime<Utc>,
}

async fn gateway_health_probe(state: &AppState) -> GatewayHealthResponse {
    let (local_base, public_base) = {
        let config = state.config.read().await;
        (config.local_gateway_base_url.clone(), config.public_gateway_base_url.clone())
    };
    const PROBE_CID: &str = "bafkqaaa";
    let local_ok = probe_gateway(&state.http, &build_gateway_url(&local_base, PROBE_CID)).await;
    let public_ok = probe_gateway(&state.http, &build_gateway_url(&public_base, PROBE_CID)).await;
    let utility_ok = probe_gateway(
        &state.http,
        &build_gateway_url(PUBLIC_UTILITY_GATEWAY_BASE_URL, PROBE_CID),
    ).await;
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

async fn submit_to_remote_pinning_service(
    state: &AppState,
    cid: &str,
    name_hint: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let (enabled, service_name, service_url, token) = {
        let config = state.config.read().await;
        (
            config.remote_pinning_enabled,
            config.remote_pinning_service_name.clone(),
            config.remote_pinning_service_url.clone(),
            config.remote_pinning_access_token.clone(),
        )
    };
    if !enabled { return Ok(None); }
    let service_url = service_url
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("Remote pinning is enabled but service URL is empty"))?;
    let token = token
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("Remote pinning is enabled but access token is empty"))?;
    let endpoint = format!("{}/pins", trim_trailing_slash(&service_url));
    let mut body = serde_json::json!({"cid": cid.trim()});
    if let Some(name) = name_hint.map(str::trim).filter(|value| !value.is_empty()) {
        body["name"] = serde_json::Value::String(name.to_string());
    }
    let response = state
        .http
        .post(endpoint)
        .bearer_auth(token.trim())
        .json(&body)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .context("Unable to reach remote pinning service")?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(anyhow!("Remote pin failed ({}): {}", status, text.chars().take(300).collect::<String>()));
    }
    let _ = response.bytes().await;
    Ok(Some(service_name.unwrap_or_else(|| "remote".to_string())))
}

async fn set_current_operation(state: &AppState, status: OperationStatus) {
    *state.operation.write().await = status;
}

async fn update_current_operation(
    state: &AppState,
    detail: Option<String>,
    progress_current: Option<usize>,
) {
    let mut guard = state.operation.write().await;
    guard.updated_at = Utc::now();
    if let Some(d) = detail { guard.detail = Some(d); }
    if let Some(p) = progress_current { guard.progress_current = Some(p); }
}

async fn clear_current_operation(state: &AppState) {
    *state.operation.write().await = OperationStatus::idle();
}

async fn send_relay_pin_failure(
    state: &AppState,
    pin: &WatchedPin,
    message: &str,
) -> anyhow::Result<bool> {
    let (relay_enabled, relay_server_url, device_token) = {
        let config = state.config.read().await;
        (config.relay_enabled, config.relay_server_url.clone(), config.relay_device_token.clone())
    };
    if !relay_enabled { return Ok(false); }
    let Some(token) = device_token.filter(|value| !value.trim().is_empty()) else { return Ok(false); };
    if relay_server_url.trim().is_empty() { return Ok(false); }
    let endpoint = format!(
        "{}/api/relay/bridge/pin-failure",
        trim_trailing_slash(&relay_server_url)
    );
    let payload = serde_json::json!({
        "deviceToken": token,
        "cid": pin.cid,
        "title": pin.title,
        "contractAddress": pin.contract_address,
        "tokenId": pin.token_id,
        "artistUsername": pin.artist_username,
        "errorCategory": pin.error_category,
        "errorMessage": message,
        "retryAttempts": pin.retry_attempts,
        "reportedAt": Utc::now(),
    });
    let response = state.http.post(endpoint).json(&payload).timeout(Duration::from_secs(8)).send().await;
    match response {
        Ok(resp) if resp.status().is_success() => Ok(true),
        Ok(resp) => Err(anyhow!("Relay pin-failure callback returned {}", resp.status())),
        Err(error) => Err(anyhow!(error)),
    }
}

fn sanitize_custom_tag(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 48 { return None; }
    let cleaned: String = trimmed.chars().filter(|c| !c.is_control()).collect();
    if cleaned.is_empty() { None } else { Some(cleaned) }
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn format_bytes_human(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;
    let v = bytes as f64;
    if v >= TB { format!("{:.2} TB", v / TB) }
    else if v >= GB { format!("{:.2} GB", v / GB) }
    else if v >= MB { format!("{:.1} MB", v / MB) }
    else if v >= KB { format!("{:.1} KB", v / KB) }
    else { format!("{} B", bytes) }
}

async fn diagnose_single_pin(
    AxumPath(cid): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<DiagnoseResponse>, AppError> {
    let trimmed = cid.trim();
    if trimmed.is_empty() { return Err(AppError::bad_request("CID is required")); }
    Ok(Json(diagnose_pin(&state, trimmed).await))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RetryPinResponse {
    cid: String,
    pinned: bool,
    used_remote_service: Option<String>,
    message: String,
}

async fn retry_pin_now(
    AxumPath(cid): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<RetryPinResponse>, AppError> {
    let trimmed = cid.trim().to_string();
    if trimmed.is_empty() { return Err(AppError::bad_request("CID is required")); }

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
        state.persistent.read().await.watched_pins.get(&trimmed).cloned()
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
            ).await?;
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
            let remote_result = submit_to_remote_pinning_service(&state, &trimmed, hint_name.as_deref()).await;
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
                format!("Local pin failed ({hint}), but the remote pinning service {service} accepted it.")
            } else {
                format!("Local pin failed. {hint} Detail: {message}")
            };
            Ok(Json(RetryPinResponse {
                cid: trimmed, pinned: false, used_remote_service: used_remote, message: reply,
            }))
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RetrySyncResponse {
    cid: String,
    synced: bool,
    path: Option<String>,
    error: Option<String>,
}

async fn retry_sync_single(
    AxumPath(cid): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<RetrySyncResponse>, AppError> {
    let trimmed = cid.trim().to_string();
    if trimmed.is_empty() { return Err(AppError::bad_request("CID is required")); }
    let exists = state.persistent.read().await.watched_pins.contains_key(&trimmed);
    if !exists { return Err(AppError::bad_request("CID is not watched by this bridge")); }
    match sync_cid_to_download_dir(&state, &trimmed).await {
        Ok(path) => Ok(Json(RetrySyncResponse {
            cid: trimmed, synced: true, path: Some(path.display().to_string()), error: None,
        })),
        Err(error) => Ok(Json(RetrySyncResponse {
            cid: trimmed, synced: false, path: None, error: Some(error.to_string()),
        })),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetPinTagsRequest {
    tags: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SetPinTagsResponse {
    cid: String,
    tags: Vec<String>,
}

async fn set_pin_tags(
    AxumPath(cid): AxumPath<String>,
    State(state): State<AppState>,
    Json(input): Json<SetPinTagsRequest>,
) -> Result<Json<SetPinTagsResponse>, AppError> {
    let trimmed = cid.trim().to_string();
    if trimmed.is_empty() { return Err(AppError::bad_request("CID is required")); }
    let cleaned: Vec<String> = {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for raw in input.tags {
            if let Some(tag) = sanitize_custom_tag(&raw) {
                let key = tag.to_ascii_lowercase();
                if seen.insert(key) { out.push(tag); }
            }
        }
        out
    };
    {
        let mut persistent = state.persistent.write().await;
        let existing = persistent.watched_pins.get_mut(&trimmed)
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

#[derive(Debug, Deserialize)]
struct ExportQuery {
    format: Option<String>,
}

async fn export_pins_handler(
    State(state): State<AppState>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, AppError> {
    let snapshot = state.persistent.read().await.clone();
    let format = query.format.as_deref().map(|v| v.trim().to_ascii_lowercase()).unwrap_or_else(|| "json".to_string());
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
                    pin.verify_count, pin.repair_count, pin.sync_count,
                    csv_escape(pin.last_error.as_deref().unwrap_or("")),
                    csv_escape(pin.error_category.as_deref().unwrap_or("")),
                    pin.retry_attempts, pin.remote_pinned,
                    csv_escape(pin.remote_pin_service.as_deref().unwrap_or("")),
                    csv_escape(&pin.custom_tags.join(";")),
                    csv_escape(pin.sync_path.as_deref().unwrap_or("")),
                ));
            }
            Ok((
                StatusCode::OK,
                [
                    ("content-type", "text/csv; charset=utf-8"),
                    ("content-disposition", "attachment; filename=\"foundation-share-bridge-pins.csv\""),
                ],
                body,
            ).into_response())
        }
        _ => {
            let json = serde_json::to_vec_pretty(&snapshot)
                .map_err(|err| AppError::internal(anyhow!("Unable to encode pins: {err}")))?;
            Ok((
                StatusCode::OK,
                [
                    ("content-type", "application/json"),
                    ("content-disposition", "attachment; filename=\"foundation-share-bridge-pins.json\""),
                ],
                json,
            ).into_response())
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtistSummary {
    total_works_managed: usize,
    works_by_you: usize,
    artists_tracked: usize,
    top_artists: Vec<ArtistEntry>,
    total_copies_pinned: usize,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ArtistEntry {
    artist_username: String,
    works: usize,
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
            if let Some(me) = current_username.as_deref() {
                if pin.artist_username.as_deref().map(|v| v.eq_ignore_ascii_case(me)).unwrap_or(false) {
                    works_by_you += 1;
                }
            }
        }
    }
    let artists_tracked = artist_counts.len();
    let mut top_artists: Vec<ArtistEntry> = artist_counts.into_iter()
        .map(|(username, works)| ArtistEntry { artist_username: username, works: works.len() })
        .collect();
    top_artists.sort_by(|a, b| b.works.cmp(&a.works).then_with(|| a.artist_username.cmp(&b.artist_username)));
    top_artists.truncate(5);
    Json(ArtistSummary {
        total_works_managed: works_by_group.len(),
        works_by_you,
        artists_tracked,
        top_artists,
        total_copies_pinned: total_copies,
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
  --brand-green: #2e6f4a;
  --brand-green-bright: #3d8c5d;
  --brand-green-soft: #e4efe7;
  --noise-blend: multiply;
  --noise-opacity: 0.5;
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
    --brand-green: #7fbf97;
    --brand-green-bright: #a6e0ba;
    --brand-green-soft: #1c2a22;
    --noise-blend: screen;
    --noise-opacity: 0.35;
  }
}
* { box-sizing: border-box; }
html, body { margin: 0; }
html { overflow-x: clip; }
body {
  position: relative;
  min-height: 100vh;
  background: var(--bg);
  color: var(--body);
  font-family: var(--font-inter), "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
  font-size: 15px;
  line-height: 1.55;
  font-feature-settings: "ss01", "cv11";
  -webkit-font-smoothing: antialiased;
  overflow-x: clip;
}
/* Paper grain (multiply on light, screen on dark) */
body::before {
  content: "";
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: 0;
  background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='140' height='140'><filter id='n'><feTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='2' stitchTiles='stitch'/><feColorMatrix values='0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 0.06 0'/></filter><rect width='100%25' height='100%25' filter='url(%23n)'/></svg>");
  opacity: var(--noise-opacity);
  mix-blend-mode: var(--noise-blend);
}
/* Ambient brand-green wash */
body::after {
  content: "";
  position: fixed;
  inset: -20vmax;
  pointer-events: none;
  z-index: 0;
  background:
    radial-gradient(38vmax 38vmax at 22% 28%, color-mix(in oklab, var(--brand-green) 10%, transparent), transparent 70%),
    radial-gradient(42vmax 42vmax at 78% 72%, color-mix(in oklab, var(--ink) 6%, transparent), transparent 70%);
  filter: blur(40px);
  opacity: 0.6;
  animation: ambient-drift 90s ease-in-out infinite alternate;
  will-change: transform;
}
@media (prefers-color-scheme: dark) {
  body::after { opacity: 0.45; }
}
@keyframes ambient-drift {
  0%   { transform: translate3d(-2%, -1%, 0) scale(1); }
  50%  { transform: translate3d(1.5%, 2%, 0) scale(1.04); }
  100% { transform: translate3d(2%, -1.5%, 0) scale(1); }
}
.page-wrap { position: relative; z-index: 1; min-height: 100vh; display: flex; flex-direction: column; }
::selection { background: var(--ink); color: var(--bg); }
h1, h2, h3 {
  font-family: var(--font-fraunces), ui-serif, Georgia, "Times New Roman", serif;
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
  background: color-mix(in oklab, var(--bg) 85%, transparent);
  -webkit-backdrop-filter: blur(8px);
  backdrop-filter: blur(8px);
  position: sticky;
  top: 0;
  z-index: 40;
}
.site-nav-inner {
  max-width: 1100px;
  margin: 0 auto;
  padding: 16px 24px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
}
.brand {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  color: var(--ink);
  text-decoration: none;
  letter-spacing: -0.01em;
  min-width: 0;
}
.brand-mark {
  display: inline-flex;
  flex: 0 0 auto;
  transition: transform 220ms cubic-bezier(0.22, 1, 0.36, 1);
}
.brand:hover .brand-mark { transform: rotate(-12deg); }
.brand-word {
  font-family: var(--font-fraunces), ui-serif, Georgia, serif;
  font-size: 1.15rem;
  line-height: 1.05;
  color: var(--ink);
  font-weight: 500;
}
.brand-eyebrow {
  display: inline-block;
  margin-left: 10px;
  color: var(--muted);
  font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  font-size: 0.62rem;
  text-transform: uppercase;
  letter-spacing: 0.26em;
  border-left: 1px solid var(--line);
  padding-left: 10px;
}
.nav-links { display: flex; gap: 22px; align-items: center; }
.nav-links a {
  position: relative;
  color: var(--muted);
  text-decoration: none;
  font-size: 0.88rem;
}
.nav-links a::after {
  content: "";
  position: absolute;
  left: 0; right: 0; bottom: -3px;
  height: 1px;
  background: currentColor;
  transform: scaleX(0);
  transform-origin: right center;
  transition: transform 320ms cubic-bezier(0.65, 0, 0.35, 1);
}
.nav-links a:hover { color: var(--ink); }
.nav-links a:hover::after {
  transform: scaleX(1);
  transform-origin: left center;
}
@media (max-width: 640px) {
  .brand-eyebrow { display: none; }
  .nav-links { gap: 14px; }
  .nav-links a { font-size: 0.82rem; }
}

/* Site footer (Agorix) */
.site-footer {
  margin-top: auto;
  border-top: 1px solid var(--line);
  background: var(--surface-quiet);
  position: relative;
  z-index: 1;
}
.site-footer-inner {
  max-width: 1100px;
  margin: 0 auto;
  padding: 56px 24px 28px;
  display: grid;
  gap: 32px;
}
@media (min-width: 720px) {
  .site-footer-inner { grid-template-columns: minmax(0, 1fr) auto; align-items: start; }
}
.site-footer .brand-row { display: flex; align-items: center; gap: 10px; }
.site-footer .brand-row .brand-word { font-size: 1.05rem; }
.site-footer p.about {
  margin-top: 14px;
  color: var(--muted);
  font-size: 0.9rem;
  line-height: 1.55;
  max-width: 52ch;
}
.site-footer p.tagline {
  margin-top: 12px;
  font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  font-size: 0.6rem;
  letter-spacing: 0.28em;
  text-transform: uppercase;
  color: var(--subtle);
}
.site-footer .footer-meta {
  border-top: 1px solid var(--line);
  margin-top: 12px;
}
.site-footer .footer-meta-inner {
  max-width: 1100px;
  margin: 0 auto;
  padding: 18px 24px;
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  justify-content: space-between;
  font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  font-size: 0.66rem;
  text-transform: uppercase;
  letter-spacing: 0.22em;
  color: var(--subtle);
}
.site-footer ul.foot-links { list-style: none; padding: 0; margin: 0; display: grid; gap: 10px; }
.site-footer ul.foot-links a {
  color: var(--body);
  text-decoration: none;
  font-size: 0.88rem;
}
.site-footer ul.foot-links a:hover { color: var(--brand-green); }
.site-footer .foot-col-label {
  font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  font-size: 0.6rem;
  letter-spacing: 0.24em;
  text-transform: uppercase;
  color: var(--subtle);
  margin: 0 0 14px;
}
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
  font-family: var(--font-fraunces), ui-serif, Georgia, serif;
  color: var(--brand-green);
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
.field-help {
  display: block;
  margin-top: 6px;
  color: var(--muted);
  font-size: 0.78rem;
  line-height: 1.45;
}
.settings-layout {
  display: grid;
  gap: 18px;
  grid-template-columns: 1fr;
}
.settings-form {
  display: grid;
  gap: 22px;
}
.settings-side {
  display: grid;
  gap: 18px;
}
.settings-block {
  padding-bottom: 22px;
  border-bottom: 1px solid var(--line);
}
.settings-block:last-of-type {
  padding-bottom: 0;
  border-bottom: 0;
}
.settings-copy {
  margin-top: 10px;
  max-width: 60ch;
}
.checkbox-row {
  display: grid;
  grid-template-columns: 18px minmax(0, 1fr);
  gap: 12px;
  align-items: start;
  margin-top: 14px;
  padding: 14px 16px;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--surface-quiet);
}
.checkbox-row input {
  margin-top: 3px;
}
.checkbox-row strong {
  display: block;
  color: var(--ink);
  font-size: 0.92rem;
}
.checkbox-row small {
  display: block;
  margin-top: 4px;
  color: var(--muted);
  font-size: 0.8rem;
  line-height: 1.45;
}
.settings-actions {
  margin-top: 6px;
}
.gateway-helper {
  margin-top: 16px;
  padding: 18px;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: var(--surface-quiet);
}
.gateway-helper h3 {
  margin-top: 8px;
  font-size: 1.05rem;
}
.gateway-helper-actions {
  align-items: center;
}
.gateway-helper-preview {
  margin-top: 14px;
  font-size: 0.82rem;
}
.gateway-helper-preview code {
  word-break: break-all;
}
.gateway-helper-note {
  font-size: 0.82rem;
}
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
.inventory-browser-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}
.inventory-browser {
  margin-top: 20px;
}
.pin-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  gap: 16px;
}
.pin-card {
  background: var(--surface);
  border: 1px solid var(--line);
  border-radius: 10px;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  min-height: 100%;
}
.pin-preview {
  aspect-ratio: 16 / 10;
  background: var(--surface-alt);
  border-bottom: 1px solid var(--line);
  position: relative;
  overflow: hidden;
}
.pin-preview-frame {
  width: 100%;
  height: 100%;
  border: 0;
  display: block;
  background: var(--surface-quiet);
}
.pin-preview-media {
  width: 100%;
  height: 100%;
  display: block;
  object-fit: cover;
  background: var(--surface-quiet);
}
.pin-preview-model {
  width: 100%;
  height: 100%;
  display: block;
  background: var(--surface-quiet);
  --poster-color: transparent;
}
.pin-preview-ar {
  display: block;
  width: 100%;
  height: 100%;
  background: var(--surface-quiet);
}
.pin-preview-ar img {
  width: 100%;
  height: 100%;
  display: block;
  object-fit: contain;
}
.pin-preview-audio {
  height: 100%;
  display: grid;
  place-items: center;
  padding: 18px;
  background: linear-gradient(180deg, rgba(18, 67, 79, 0.26), rgba(18, 67, 79, 0.05));
}
.pin-preview-audio audio {
  width: min(100%, 280px);
}
.pin-preview-empty {
  height: 100%;
  display: grid;
  place-items: center;
  padding: 18px;
  text-align: center;
  color: var(--muted);
  font-size: 0.86rem;
}
.pin-card-body {
  padding: 18px;
  display: grid;
  gap: 14px;
}
.pin-card-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}
.pin-title {
  font-family: var(--font-fraunces), ui-serif, Georgia, serif;
  color: var(--ink);
  font-size: 1.15rem;
  line-height: 1.15;
}
.pin-context {
  color: var(--muted);
  font-size: 0.84rem;
}
.pin-meta {
  display: grid;
  gap: 8px;
}
.pin-meta-line {
  display: grid;
  grid-template-columns: 112px minmax(0, 1fr);
  gap: 10px;
  align-items: start;
  font-size: 0.84rem;
}
.pin-meta-line strong {
  font-size: 0.7rem;
  text-transform: uppercase;
  letter-spacing: 0.18em;
  color: var(--muted);
  font-weight: 500;
}
.pin-note {
  border-radius: 6px;
  padding: 10px 12px;
  font-size: 0.82rem;
  border: 1px solid var(--line);
  background: var(--surface-quiet);
}
.pin-note.err {
  border-color: rgba(154,42,42,0.25);
  background: var(--tint-err);
  color: var(--err);
}
.pin-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
}
.pin-meta-toggle {
  justify-content: space-between;
  min-width: min(100%, 220px);
}
.pin-metadata-inline {
  flex: 1 1 100%;
}
.pin-meta-toggle-copy {
  color: var(--muted);
  font-size: 0.74rem;
  font-weight: 400;
}
.pin-metadata-viewer {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows 260ms cubic-bezier(0.22, 1, 0.36, 1);
}
.pin-metadata-viewer.is-open {
  grid-template-rows: 1fr;
}
.pin-metadata-viewer-inner {
  overflow: hidden;
}
.pin-metadata-panel {
  margin-top: 12px;
  padding: 16px;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: var(--surface-quiet);
  opacity: 0;
  transform: translateY(-6px);
  transition: opacity 220ms ease, transform 220ms ease;
}
.pin-metadata-viewer.is-open .pin-metadata-panel {
  opacity: 1;
  transform: translateY(0);
}
.pin-metadata-description {
  margin: 0 0 14px;
  color: var(--body);
  font-size: 0.84rem;
  line-height: 1.55;
  white-space: pre-wrap;
}
.pin-metadata-lines {
  display: grid;
  gap: 8px;
}
.pin-metadata-line {
  display: grid;
  grid-template-columns: 112px minmax(0, 1fr);
  gap: 10px;
  align-items: start;
  font-size: 0.82rem;
}
.pin-metadata-line strong,
.pin-metadata-json-head {
  font-size: 0.7rem;
  text-transform: uppercase;
  letter-spacing: 0.18em;
  color: var(--muted);
  font-weight: 500;
}
.pin-metadata-value {
  color: var(--body);
  word-break: break-word;
}
.pin-metadata-traits {
  margin-top: 14px;
}
.pin-metadata-trait-grid {
  margin-top: 10px;
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.pin-metadata-trait {
  display: inline-flex;
  flex-direction: column;
  gap: 3px;
  padding: 10px 12px;
  border-radius: 999px;
  border: 1px solid var(--line);
  background: var(--surface);
  max-width: 100%;
}
.pin-metadata-trait strong {
  font-size: 0.66rem;
  text-transform: uppercase;
  letter-spacing: 0.16em;
  color: var(--muted);
  font-weight: 500;
}
.pin-metadata-trait span {
  color: var(--ink);
  font-size: 0.82rem;
  word-break: break-word;
}
.pin-metadata-json-wrap {
  margin-top: 14px;
}
.pin-metadata-json-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}
.pin-metadata-json-note {
  color: var(--muted);
  font-size: 0.76rem;
  letter-spacing: normal;
  text-transform: none;
}
.pin-metadata-json {
  margin: 10px 0 0;
  padding: 14px;
  max-height: 280px;
  overflow: auto;
  border-radius: 8px;
  border: 1px solid var(--line);
  background: var(--surface);
  color: var(--body);
  font-size: 0.74rem;
  line-height: 1.55;
  font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  white-space: pre-wrap;
  word-break: break-word;
}
.pin-test-status {
  font-size: 0.82rem;
  color: var(--muted);
}
.inventory-load-row {
  margin-top: 18px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}
.inventory-status {
  font-size: 0.84rem;
}
.inventory-sentinel {
  height: 1px;
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
@media (max-width: 640px) {
  .pin-meta-line {
    grid-template-columns: 1fr;
    gap: 4px;
  }
  .pin-metadata-line {
    grid-template-columns: 1fr;
    gap: 4px;
  }
  .checkbox-row {
    grid-template-columns: 1fr;
  }
}
@media (min-width: 980px) {
  .settings-layout {
    grid-template-columns: minmax(0, 1.15fr) minmax(280px, 0.85fr);
    align-items: start;
  }
  .settings-side {
    position: sticky;
    top: 96px;
  }
}
"#;

const INVENTORY_BROWSER_SCRIPT: &str = r####"
(() => {
  const browser = document.getElementById("inventory-browser");
  if (!browser) return;

  const grid = document.getElementById("inventory-grid");
  const emptyState = document.getElementById("inventory-empty");
  const loadMoreButton = document.getElementById("inventory-load-more");
  const statusNode = document.getElementById("inventory-status");
  const sentinel = document.getElementById("inventory-sentinel");
  const pageSize = Number(browser.getAttribute("data-page-size") || "12");
  const state = {
    loading: false,
    nextCursor: null,
    done: false,
    loadedAny: false,
    error: false,
  };

  const previewObserver = "IntersectionObserver" in window
    ? new IntersectionObserver((entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          const node = entry.target;
          if (!node.getAttribute("src")) {
            loadPreviewCandidate(node, 0);
          }
          previewObserver.unobserve(node);
        }
      }, { rootMargin: "220px 0px" })
    : null;

  const paginationObserver = sentinel && "IntersectionObserver" in window
    ? new IntersectionObserver((entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            void loadNextPage();
          }
        }
      }, { rootMargin: "320px 0px" })
    : null;

  if (paginationObserver && sentinel) {
    paginationObserver.observe(sentinel);
  }

  const escapeHtml = (value) =>
    String(value ?? "").replace(/[&<>"']/g, (char) => ({
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      "\"": "&quot;",
      "'": "&#39;",
    }[char] || char));

  const formatTimestamp = (value) => {
    if (!value) return "Never";
    const parsed = new Date(value);
    if (Number.isNaN(parsed.getTime())) return String(value);
    return parsed.toLocaleString(undefined, {
      dateStyle: "medium",
      timeStyle: "short",
    });
  };

  const shortAddress = (value) => {
    const text = String(value ?? "").trim();
    if (text.length <= 12) return text;
    return `${text.slice(0, 6)}…${text.slice(-4)}`;
  };

  const uniqueStrings = (values) =>
    Array.from(
      new Set(
        values
          .map((value) => String(value ?? "").trim())
          .filter((value) => value.length > 0),
      ),
    );

  const guessKindFromUrl = (value) => {
    const text = String(value ?? "").toLowerCase();
    if (!text) return "UNKNOWN";
    if (text.includes(".mp4") || text.includes(".mov") || text.includes(".webm") || text.includes("video")) {
      return "VIDEO";
    }
    if (text.includes(".mp3") || text.includes(".wav") || text.includes(".ogg") || text.includes(".aac") || text.includes("audio")) {
      return "AUDIO";
    }
    if (
      text.includes(".png") ||
      text.includes(".jpg") ||
      text.includes(".jpeg") ||
      text.includes(".gif") ||
      text.includes(".svg") ||
      text.includes(".webp") ||
      text.includes("image")
    ) {
      return "IMAGE";
    }
    if (text.includes(".html") || text.includes("text/html")) {
      return "HTML";
    }
    if (
      text.includes(".glb") ||
      text.includes(".gltf") ||
      text.includes(".usdz") ||
      text.includes("model/gltf") ||
      text.includes("model/vnd.usdz") ||
      text.includes("model")
    ) {
      return "MODEL";
    }
    return "UNKNOWN";
  };

  const normalizeKind = (value) => {
    const text = String(value ?? "").trim().toUpperCase();
    if (text === "IMAGE" || text === "VIDEO" || text === "AUDIO" || text === "HTML" || text === "MODEL") {
      return text;
    }
    return "UNKNOWN";
  };

  const stripQueryString = (value) => {
    const raw = String(value ?? "");
    const cut = raw.indexOf("?");
    return cut === -1 ? raw : raw.slice(0, cut);
  };

  const isUsdzUrl = (value) =>
    stripQueryString(value).toLowerCase().endsWith(".usdz");

  const supportsInlineModelPreview = (value) => !isUsdzUrl(value);

  const previewKindForPreviewUrl = (url, fallbackKind, openUrl) => {
    if (openUrl && url !== openUrl) return "IMAGE";

    const fromUrl = guessKindFromUrl(url);
    if (fromUrl !== "UNKNOWN") return fromUrl;
    return fallbackKind;
  };

  const getRelatedCids = (item) =>
    uniqueStrings([
      item?.cid,
      item?.metadataCid,
      item?.mediaCid,
      ...(Array.isArray(item?.relatedCids) ? item.relatedCids : []),
    ]);

  const buildPublicGatewayUrl = (item) =>
    item?.cid ? `https://dweb.link/ipfs/${encodeURIComponent(String(item.cid).trim())}` : "";

  const buildPreviewCandidates = (item) => {
    const previewEntries = [
      item?.previewLocalGatewayUrl
        ? {
            url: item.previewLocalGatewayUrl,
            kind: previewKindForPreviewUrl(
              item.previewLocalGatewayUrl,
              normalizeKind(item.mediaKind),
              item.localGatewayUrl,
            ),
          }
        : null,
      item?.previewPublicGatewayUrl
        ? {
            url: item.previewPublicGatewayUrl,
            kind: previewKindForPreviewUrl(
              item.previewPublicGatewayUrl,
              normalizeKind(item.mediaKind),
              item.publicGatewayUrl,
            ),
          }
        : null,
      item?.localGatewayUrl
        ? { url: item.localGatewayUrl, kind: normalizeKind(item.mediaKind) }
        : null,
      item?.publicGatewayUrl
        ? { url: item.publicGatewayUrl, kind: normalizeKind(item.mediaKind) }
        : null,
      buildPublicGatewayUrl(item)
        ? { url: buildPublicGatewayUrl(item), kind: normalizeKind(item.mediaKind) }
        : null,
    ].filter((value) => value && value.url);

    const seen = new Set();
    return previewEntries.filter((entry) => {
      if (seen.has(entry.url)) return false;
      seen.add(entry.url);
      return true;
    });
  };

  const choosePinnedUrl = (item) =>
    item.publicGatewayUrl || item.localGatewayUrl || buildPublicGatewayUrl(item) || "";

  const choosePublicUrl = (item) => buildPublicGatewayUrl(item);

  const buildContextHtml = (item) => {
    if (item.foundationUrl) {
      return `<a href="${escapeHtml(item.foundationUrl)}" target="_blank" rel="noreferrer">Open work page</a>`;
    }
    if (item.contractAddress && item.tokenId) {
      return `${escapeHtml(shortAddress(item.contractAddress))} #${escapeHtml(item.tokenId)}`;
    }
    if (item.username) return `@${escapeHtml(item.username)}`;
    if (item.artistUsername) return `@${escapeHtml(item.artistUsername)}`;
    if (item.sourceKind) return escapeHtml(item.sourceKind);
    return "Pinned on this computer";
  };

  const buildPreviewHtml = (item, title) => {
    const candidates = buildPreviewCandidates(item);
    const primary = candidates[0] ?? null;
    if (!primary) {
      return `<div class="pin-preview-empty">No preview URL yet for this CID.</div>`;
    }

    const encodedCandidates = escapeHtml(
      candidates.map((entry) => `${entry.kind}|${entry.url}`).join("\n"),
    );

    if (primary.kind === "IMAGE") {
      return `<img class="pin-preview-media pin-preview-loadable" alt="${escapeHtml(title)}" data-preview-candidates="${encodedCandidates}" loading="lazy" />`;
    }

    if (primary.kind === "VIDEO") {
      return `<video class="pin-preview-media pin-preview-loadable" aria-label="Preview for ${escapeHtml(title)}" data-preview-candidates="${encodedCandidates}" muted playsinline controls preload="metadata"></video>`;
    }

    if (primary.kind === "AUDIO") {
      return `<div class="pin-preview-audio"><audio class="pin-preview-loadable" aria-label="Preview for ${escapeHtml(title)}" data-preview-candidates="${encodedCandidates}" controls preload="metadata"></audio></div>`;
    }

    if (primary.kind === "MODEL") {
      const modelCandidates = candidates.filter(
        (entry) => entry.kind === "MODEL" && supportsInlineModelPreview(entry.url),
      );
      const usdzCandidate = candidates.find(
        (entry) => entry.kind === "MODEL" && isUsdzUrl(entry.url),
      );
      const posterCandidate = candidates.find((entry) => entry.kind === "IMAGE");

      if (modelCandidates.length === 0 && usdzCandidate) {
        const posterSrc = posterCandidate ? posterCandidate.url : usdzCandidate.url;
        return `<a class="pin-preview-ar" rel="ar" href="${escapeHtml(usdzCandidate.url)}"><img alt="${escapeHtml(title)}" src="${escapeHtml(posterSrc)}" /></a>`;
      }

      const inlineCandidatesEncoded = escapeHtml(
        modelCandidates.map((entry) => `${entry.kind}|${entry.url}`).join("\n"),
      );
      const iosSrcAttr = usdzCandidate
        ? ` ios-src="${escapeHtml(usdzCandidate.url)}"`
        : "";
      const posterAttr = posterCandidate
        ? ` poster="${escapeHtml(posterCandidate.url)}"`
        : "";
      return `<model-viewer class="pin-preview-model pin-preview-loadable" alt="${escapeHtml(title)}" data-preview-candidates="${inlineCandidatesEncoded}"${iosSrcAttr}${posterAttr} ar ar-modes="webxr scene-viewer quick-look" camera-controls touch-action="pan-y" interaction-prompt="none" shadow-intensity="0.85" exposure="1" environment-image="neutral"><div class="pin-preview-empty">Loading 3D preview…</div></model-viewer>`;
    }

    return `<iframe class="pin-preview-frame pin-preview-loadable" title="Preview for ${escapeHtml(title)}" data-preview-candidates="${encodedCandidates}" referrerpolicy="no-referrer" allowfullscreen></iframe>`;
  };

  const buildVerificationSummary = (item) => {
    if (!item.lastVerifiedAt) {
      return "Network visibility has not been checked yet.";
    }
    const detail = item.lastError
      ? ` · ${escapeHtml(item.lastError)}`
      : "";
    return `Last checked ${escapeHtml(formatTimestamp(item.lastVerifiedAt))}${detail}`;
  };

  const buildNoteHtml = (item) => {
    if (item.lastError) {
      return `<p class="pin-note err">${escapeHtml(item.lastError)}</p>`;
    }
    if (item.lastSyncError) {
      return `<p class="pin-note err">${escapeHtml(item.lastSyncError)}</p>`;
    }
    return "";
  };

  const formatRootsSummary = (item) => {
    const totalRoots = getRelatedCids(item).length;
    if (totalRoots <= 1) return "1 linked root";
    return `${totalRoots} linked roots`;
  };

  const buildMetadataViewerId = (item) =>
    `pin-metadata-${encodeURIComponent(String(item?.cid ?? "").trim()).replace(/[^a-zA-Z0-9_-]+/g, "")}`;

  const metadataToggleCopy = (metadataView) => {
    if (!metadataView) return "";
    const fieldCount = Array.isArray(metadataView.fields) ? metadataView.fields.length : 0;
    const attributeCount = Array.isArray(metadataView.attributes) ? metadataView.attributes.length : 0;
    const pieces = [];
    if (fieldCount > 0) pieces.push(`${fieldCount} detail${fieldCount === 1 ? "" : "s"}`);
    if (attributeCount > 0) pieces.push(`${attributeCount} trait${attributeCount === 1 ? "" : "s"}`);
    if (pieces.length === 0) return "raw JSON";
    return pieces.join(" · ");
  };

  const renderMetadataLines = (entries) =>
    entries
      .filter((entry) => entry && entry.label && entry.value)
      .map((entry) => `
        <div class="pin-metadata-line">
          <strong>${escapeHtml(entry.label)}</strong>
          <span class="pin-metadata-value">${escapeHtml(entry.value)}</span>
        </div>
      `)
      .join("");

  const renderMetadataTraits = (entries) => {
    if (!Array.isArray(entries) || entries.length === 0) return "";

    return `
      <div class="pin-metadata-traits">
        <div class="pin-metadata-json-head">
          <strong>Traits</strong>
        </div>
        <div class="pin-metadata-trait-grid">
          ${entries
            .filter((entry) => entry && entry.label && entry.value)
            .map((entry) => `
              <div class="pin-metadata-trait">
                <strong>${escapeHtml(entry.label)}</strong>
                <span>${escapeHtml(entry.value)}</span>
              </div>
            `)
            .join("")}
        </div>
      </div>
    `;
  };

  const renderMetadataViewer = (item) => {
    const metadataView = item?.metadataView;
    if (!metadataView) return "";

    const viewerId = buildMetadataViewerId(item);
    const detailEntries = [
      item?.metadataCid ? { label: "Metadata CID", value: item.metadataCid } : null,
      item?.mediaCid ? { label: "Media CID", value: item.mediaCid } : null,
      ...(Array.isArray(metadataView.fields) ? metadataView.fields : []),
    ].filter(Boolean);

    const description = metadataView.description
      ? `<p class="pin-metadata-description">${escapeHtml(metadataView.description)}</p>`
      : "";
    const detailLines = detailEntries.length
      ? `<div class="pin-metadata-lines">${renderMetadataLines(detailEntries)}</div>`
      : "";
    const traits = renderMetadataTraits(metadataView.attributes);
    const rawJson = metadataView.rawJson
      ? `
        <div class="pin-metadata-json-wrap">
          <div class="pin-metadata-json-head">
            <strong>Raw JSON</strong>
            ${
              metadataView.rawJsonTruncated
                ? '<span class="pin-metadata-json-note">trimmed for speed</span>'
                : ""
            }
          </div>
          <pre class="pin-metadata-json"><code>${escapeHtml(metadataView.rawJson)}</code></pre>
        </div>
      `
      : "";

    return `
      <div class="pin-metadata-inline">
        <button
          type="button"
          class="btn ghost pin-meta-toggle"
          data-toggle-metadata
          data-metadata-target="${escapeHtml(viewerId)}"
          data-open-label="Hide metadata"
          data-closed-label="Show metadata"
          aria-expanded="false"
          aria-controls="${escapeHtml(viewerId)}"
        >
          <span data-toggle-label>Show metadata</span>
          <span class="pin-meta-toggle-copy">${escapeHtml(metadataToggleCopy(metadataView))}</span>
        </button>
        <div class="pin-metadata-viewer" id="${escapeHtml(viewerId)}" aria-hidden="true">
          <div class="pin-metadata-viewer-inner">
            <div class="pin-metadata-panel">
              ${description}
              ${detailLines}
              ${traits}
              ${rawJson}
            </div>
          </div>
        </div>
      </div>
    `;
  };

  const renderCard = (item) => {
    const title = item.title || item.label || "Local IPFS pin";
    const statusLabel = item.pinned ? (item.pinType || "pinned") : "repair needed";
    const statusClass = item.pinned ? "ok" : "warn";
    const pinnedUrl = choosePinnedUrl(item);
    const publicUrl = choosePublicUrl(item);
    const localUrl = item.localGatewayUrl || "";
    const relatedCids = getRelatedCids(item);
    const syncedValue = item.syncPath
      ? escapeHtml(item.syncPath)
      : "Not synced to disk";

    return `
      <article class="pin-card" data-cid="${escapeHtml(item.cid)}" data-related-cids="${escapeHtml(relatedCids.join(","))}">
        <div class="pin-preview">
          ${buildPreviewHtml(item, title)}
        </div>
        <div class="pin-card-body">
          <div class="pin-card-head">
            <div>
              <p class="pin-title">${escapeHtml(title)}</p>
              <p class="cid">${escapeHtml(item.cid)}</p>
            </div>
            <span class="pill ${statusClass}">${escapeHtml(statusLabel)}</span>
          </div>

          <p class="pin-context">${buildContextHtml(item)}</p>

          <div class="pin-meta">
            <div class="pin-meta-line">
              <strong>Source</strong>
              <span>${escapeHtml(item.label || item.sourceKind || "watched pin")}</span>
            </div>
            <div class="pin-meta-line">
              <strong>Verified</strong>
              <span>${escapeHtml(formatTimestamp(item.lastVerifiedAt))}</span>
            </div>
            <div class="pin-meta-line">
              <strong>Synced</strong>
              <span>${syncedValue}</span>
            </div>
            <div class="pin-meta-line">
              <strong>Roots</strong>
              <span>${escapeHtml(formatRootsSummary(item))}</span>
            </div>
          </div>

          ${buildNoteHtml(item)}

          <div class="pin-actions">
            <button type="button" class="btn ghost" data-verify-cids="${escapeHtml(relatedCids.join(","))}">Test on network</button>
            ${pinnedUrl ? `<a class="btn" href="${escapeHtml(pinnedUrl)}" target="_blank" rel="noreferrer">Open pinned</a>` : ""}
            ${publicUrl ? `<a class="btn ghost" href="${escapeHtml(publicUrl)}" target="_blank" rel="noreferrer">Open public</a>` : ""}
            ${localUrl ? `<a class="btn ghost" href="${escapeHtml(localUrl)}" target="_blank" rel="noreferrer">Open local</a>` : ""}
            ${renderMetadataViewer(item)}
          </div>

          <p class="pin-test-status">${buildVerificationSummary(item)}</p>
        </div>
      </article>
    `;
  };

  const readPreviewCandidates = (node) =>
    String(node.getAttribute("data-preview-candidates") || "")
      .split("\n")
      .map((entry) => entry.trim())
      .filter(Boolean)
      .map((entry) => {
        const divider = entry.indexOf("|");
        if (divider === -1) return { kind: "UNKNOWN", url: entry };
        return {
          kind: entry.slice(0, divider) || "UNKNOWN",
          url: entry.slice(divider + 1),
        };
      })
      .filter((entry) => entry.url);

  const loadPreviewCandidate = (node, index) => {
    const candidates = readPreviewCandidates(node);
    const next = candidates[index] ?? null;
    if (!next) return false;

    if (
      node.tagName === "IMG" ||
      node.tagName === "IFRAME" ||
      node.tagName === "VIDEO" ||
      node.tagName === "AUDIO" ||
      node.tagName === "MODEL-VIEWER"
    ) {
      node.setAttribute("src", next.url);
    }
    if ((node.tagName === "VIDEO" || node.tagName === "AUDIO") && typeof node.load === "function") {
      node.load();
    }

    node.setAttribute("data-preview-index", String(index));
    return true;
  };

  const advancePreviewCandidate = (node) => {
    const currentIndex = Number(node.getAttribute("data-preview-index") || "0");
    return loadPreviewCandidate(node, currentIndex + 1);
  };

  const hydratePreviewMedia = () => {
    const nodes = grid.querySelectorAll(".pin-preview-loadable[data-preview-candidates]");
    for (const node of nodes) {
      if (node.getAttribute("src")) continue;

      if (!node.hasAttribute("data-preview-error-bound")) {
        node.setAttribute("data-preview-error-bound", "true");
        node.addEventListener("error", () => {
          const advanced = advancePreviewCandidate(node);
          if (!advanced) {
            const container = node.closest(".pin-preview");
            if (container) {
              container.innerHTML = `<div class="pin-preview-empty">Preview unavailable right now.</div>`;
            }
          }
        });
      }

      if (!previewObserver) {
        loadPreviewCandidate(node, 0);
        continue;
      }
      previewObserver.observe(node);
    }
  };

  const setStatus = (message) => {
    if (statusNode) {
      statusNode.textContent = message;
    }
  };

  const syncControls = () => {
    if (!loadMoreButton) return;
    loadMoreButton.disabled = state.loading;
    loadMoreButton.hidden = !state.nextCursor && !state.error;
    loadMoreButton.textContent = state.error ? "Retry load" : "Load more works";
  };

  const loadNextPage = async () => {
    if (state.loading || (state.done && !state.error)) return;

    state.loading = true;
    state.error = false;
    syncControls();
    setStatus(state.loadedAny ? "Loading more works…" : "Loading saved works…");

    try {
      const url = new URL("/pins/page", window.location.origin);
      url.searchParams.set("limit", String(pageSize));
      if (state.nextCursor) {
        url.searchParams.set("cursor", state.nextCursor);
      }

      const response = await fetch(url.toString(), {
        headers: { Accept: "application/json" },
      });
      if (!response.ok) {
        throw new Error(`Inventory request failed (${response.status})`);
      }

      const payload = await response.json();
      const items = Array.isArray(payload.items) ? payload.items : [];

      if (items.length > 0) {
        grid.insertAdjacentHTML("beforeend", items.map(renderCard).join(""));
        hydratePreviewMedia();
        state.loadedAny = true;
        if (emptyState) emptyState.hidden = true;
      } else if (!state.loadedAny && emptyState) {
        emptyState.hidden = false;
      }

      state.nextCursor = payload.nextCursor || null;
      state.done = !state.nextCursor;
      state.error = false;
      syncControls();

      if (state.done) {
        setStatus(state.loadedAny ? `Showing ${grid.children.length} works.` : "No saved works available.");
      } else {
        setStatus(`Showing ${grid.children.length} of ${payload.total} works.`);
      }
    } catch (error) {
      state.done = true;
      state.error = true;
      syncControls();
      setStatus(error instanceof Error ? error.message : "Unable to load saved works.");
    } finally {
      state.loading = false;
      syncControls();
    }
  };

  const toggleMetadataViewer = (button) => {
    const targetId = String(button.getAttribute("data-metadata-target") || "").trim();
    if (!targetId) return;

    const viewer = document.getElementById(targetId);
    if (!viewer) return;

    const isOpen = !viewer.classList.contains("is-open");
    viewer.classList.toggle("is-open", isOpen);
    viewer.setAttribute("aria-hidden", String(!isOpen));
    button.setAttribute("aria-expanded", String(isOpen));

    const labelNode = button.querySelector("[data-toggle-label]");
    const nextLabel = isOpen
      ? button.getAttribute("data-open-label")
      : button.getAttribute("data-closed-label");
    if (labelNode && nextLabel) {
      labelNode.textContent = nextLabel;
    }
  };

  browser.addEventListener("click", async (event) => {
    const metadataButton = event.target.closest("[data-toggle-metadata]");
    if (metadataButton) {
      toggleMetadataViewer(metadataButton);
      return;
    }

    const button = event.target.closest("[data-verify-cids]");
    if (!button) return;

    const cids = uniqueStrings(
      String(button.getAttribute("data-verify-cids") || "").split(","),
    );
    const card = button.closest(".pin-card");
    const resultNode = card ? card.querySelector(".pin-test-status") : null;
    if (cids.length === 0 || !resultNode) return;

    button.setAttribute("disabled", "disabled");
    resultNode.textContent = `Checking ${cids.length} linked root${cids.length === 1 ? "" : "s"} on the network…`;

    try {
      const response = await fetch("/pins/verify", {
        method: "POST",
        headers: {
          Accept: "application/json",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ cids }),
      });

      if (!response.ok) {
        throw new Error(`Verification failed (${response.status})`);
      }

      const payload = await response.json();
      const results = Array.isArray(payload.results) ? payload.results : [];
      const visible = results.filter(
        (entry) => entry && entry.reachable && entry.providerCount > 0,
      );
      const checkedAt =
        payload.checkedAt ||
        results
          .map((entry) => entry?.checkedAt)
          .filter(Boolean)
          .sort()
          .at(-1) ||
        null;
      const firstError =
        results.find((entry) => entry?.error)?.error ||
        null;

      if (visible.length === results.length && visible.length > 0) {
        const providerCount = Math.min(
          ...visible.map((entry) => Number(entry.providerCount) || 0),
        );
        resultNode.textContent = `Visible on the network for all ${visible.length} linked root${visible.length === 1 ? "" : "s"} via at least ${providerCount} provider${providerCount === 1 ? "" : "s"} · checked ${formatTimestamp(checkedAt)}`;
      } else if (visible.length > 0) {
        resultNode.textContent = `Only ${visible.length} of ${results.length} linked roots are visible on the network yet${firstError ? ` · ${firstError}` : ""}${checkedAt ? ` · checked ${formatTimestamp(checkedAt)}` : ""}`;
      } else if (firstError) {
        resultNode.textContent = firstError;
      } else {
        resultNode.textContent = `No linked roots are visible on the network yet${checkedAt ? ` · checked ${formatTimestamp(checkedAt)}` : ""}`;
      }
    } catch (error) {
      resultNode.textContent = error instanceof Error ? error.message : "Unable to verify this pin right now.";
    } finally {
      button.removeAttribute("disabled");
    }
  });

  if (loadMoreButton) {
    loadMoreButton.addEventListener("click", () => {
      void loadNextPage();
    });
  }

  void loadNextPage();
})();
"####;

const ROOT_AUTOLINK_SCRIPT: &str = r####"
(() => {
  const form = document.getElementById("autolink-form");
  const status = document.getElementById("autolink-status");
  if (!form) return;

  window.setTimeout(() => {
    if (status) {
      status.textContent = "Confirming with the archive site now…";
    }

    if (typeof form.requestSubmit === "function") {
      form.requestSubmit();
      return;
    }

    form.submit();
  }, 400);
})();
"####;

const SETTINGS_GATEWAY_HELPER_SCRIPT: &str = r####"
(() => {
  const target = document.getElementById("public_gateway_base_url");
  if (!target) return;

  const hostnameInput = document.getElementById("gateway_hostname_input");
  const hostnameButton = document.getElementById("gateway_fill_hostname");
  const ipButton = document.getElementById("gateway_fill_ip");
  const previewValue = document.getElementById("gateway_helper_preview_value");

  const escapeHtml = (value) =>
    String(value ?? "").replace(/[&<>"']/g, (char) => ({
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      "\"": "&quot;",
      "'": "&#39;",
    }[char] || char));

  const updatePreview = (value) => {
    if (!previewValue) return;
    previewValue.innerHTML = escapeHtml(value || "");
  };

  const normalizeHost = (value) => {
    const trimmed = String(value ?? "").trim();
    if (!trimmed) return "";
    const withoutScheme = trimmed.replace(/^https?:\/\//i, "");
    return withoutScheme.replace(/\/+.*$/, "").replace(/\/+$/g, "");
  };

  if (hostnameButton) {
    hostnameButton.addEventListener("click", () => {
      const host = normalizeHost(hostnameInput ? hostnameInput.value : "");
      if (!host) {
        if (hostnameInput) hostnameInput.focus();
        return;
      }
      target.value = `https://${host}`;
      updatePreview(target.value);
      target.focus();
    });
  }

  if (ipButton) {
    ipButton.addEventListener("click", () => {
      const gatewayUrl = ipButton.getAttribute("data-gateway-url");
      if (!gatewayUrl) return;
      target.value = gatewayUrl;
      updatePreview(target.value);
      target.focus();
    });
  }

  target.addEventListener("input", () => {
    updatePreview(target.value);
  });

  updatePreview(target.value);
})();
"####;

const LOGO_MARK_SVG: &str = r##"<svg class="brand-mark" role="img" aria-label="Agorix mark" width="28" height="28" viewBox="0 0 64 64"><g fill="none" stroke="currentColor" stroke-width="3.2" stroke-linecap="square" style="color: var(--ink); opacity: 0.78"><path d="M6 18V6h12"/><path d="M58 18V6H46"/><path d="M6 46v12h12"/><path d="M58 46v12H46"/></g><path d="M32 16 C 32 24, 40 32, 48 32 C 40 32, 32 40, 32 48 C 32 40, 24 32, 16 32 C 24 32, 32 24, 32 16 Z" fill="var(--brand-green)"/></svg>"##;

fn render_page(title: &str, body_html: &str) -> String {
    let year = Utc::now().format("%Y").to_string();
    let mut out = String::with_capacity(8192 + body_html.len());
    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n");
    out.push_str("  <meta charset=\"utf-8\" />\n");
    out.push_str("  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n");
    out.push_str("  <title>");
    out.push_str(&escape_html(title));
    out.push_str(" · Agorix Share Bridge</title>\n");
    out.push_str("  <link rel=\"preconnect\" href=\"https://fonts.googleapis.com\" />\n");
    out.push_str("  <link rel=\"preconnect\" href=\"https://fonts.gstatic.com\" crossorigin />\n");
    out.push_str("  <link rel=\"stylesheet\" href=\"https://fonts.googleapis.com/css2?family=Fraunces:opsz,wght@9..144,400;9..144,500&family=Inter:wght@400;500;600&display=swap\" />\n");
    out.push_str("  <script type=\"module\" src=\"https://cdn.jsdelivr.net/npm/@google/model-viewer/dist/model-viewer.min.js\"></script>\n");
    out.push_str("  <style>:root{--font-inter:'Inter';--font-fraunces:'Fraunces';}");
    out.push_str(PAGE_STYLE);
    out.push_str("</style>\n</head>\n<body>\n");
    out.push_str("<div class=\"page-wrap\">\n");
    out.push_str("  <nav class=\"site-nav\"><div class=\"site-nav-inner\">");
    out.push_str("<a class=\"brand\" href=\"/\" aria-label=\"Agorix home\">");
    out.push_str(LOGO_MARK_SVG);
    out.push_str("<span class=\"brand-word\">Agorix</span>");
    out.push_str("<span class=\"brand-eyebrow\">share bridge</span>");
    out.push_str("</a>");
    out.push_str(
        "<div class=\"nav-links\">\
         <a href=\"/#status\">Status</a>\
         <a href=\"/#inventory\">Pins</a>\
         <a href=\"/#connection\">Connection</a>\
         <a href=\"/settings\">Settings</a>\
         </div>",
    );
    out.push_str("</div></nav>\n");
    out.push_str(body_html);
    out.push_str(
        "\n  <footer class=\"site-footer\"><div class=\"site-footer-inner\">\
        <div>\
          <div class=\"brand-row\">",
    );
    out.push_str(LOGO_MARK_SVG);
    out.push_str(
        "<span class=\"brand-word\">Agorix</span>\
          </div>\
          <p class=\"about\">Agorix is the broader preservation project. This local companion app keeps rescued Foundation roots pinned on your IPFS node and self-repairs anything that drops. Not affiliated with Foundation.</p>\
          <p class=\"tagline\">Local pin companion · Forever repair · Artist-aligned</p>\
        </div>\
        <div>\
          <p class=\"foot-col-label\">Bridge</p>\
          <ul class=\"foot-links\">\
            <li><a href=\"/#status\">Status</a></li>\
            <li><a href=\"/#inventory\">Local pins</a></li>\
            <li><a href=\"/#connection\">Connection</a></li>\
            <li><a href=\"/settings\">Settings</a></li>\
          </ul>\
        </div>\
      </div>\
      <div class=\"footer-meta\"><div class=\"footer-meta-inner\">\
        <p>© ",
    );
    out.push_str(&year);
    out.push_str(
        " Agorix</p>\
        <p>Independent · Decentralized · Artist-aligned</p>\
      </div></div>\
    </footer>\n",
    );
    out.push_str("</div>\n</body>\n</html>");
    out
}

fn format_timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
