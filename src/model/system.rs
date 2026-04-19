//! System-health DTOs — `/health`, `/storage/stats`, `/gateway/health`,
//! `/diagnose/:cid`, the underlying Kubo `repo/stat` shape, and the artist
//! summary surfaced on the dashboard.

// `struct_excessive_bools`: the `/health` wire format is fixed; the response
// intentionally exposes every relevant boolean state.
#![allow(clippy::struct_excessive_bools)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::OperationStatus;

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
