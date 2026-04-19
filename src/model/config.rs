//! Configuration DTOs — the persistent bridge state, the writable bridge
//! config, JSON/form bodies for `/config` updates, and HTML dashboard
//! query-string bindings.
//!
//! [`BridgePersistentState`] and [`BridgeConfig`] both back on-disk files.
//! Their serde layouts are migration boundaries; see the note in
//! [`crate::model::pin`].

// `struct_excessive_bools`: the wire format is fixed; see module-level note.
// `option_option`: `Option<Option<T>>` in the update request distinguishes
// "field absent from JSON" (outer None) from "field explicitly set to null"
// (outer Some(None)) — a deliberate three-valued encoding for PATCH-style
// updates.
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::option_option)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::model::pin::WatchedPin;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BridgePersistentState {
    pub watched_pins: HashMap<String, WatchedPin>,
    pub updated_at: Option<DateTime<Utc>>,
    pub last_repair_cycle_at: Option<DateTime<Utc>>,
    pub repair_cycle_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeConfig {
    pub download_root_dir: String,
    pub sync_enabled: bool,
    pub local_gateway_base_url: String,
    pub public_gateway_base_url: String,
    pub relay_enabled: bool,
    pub relay_server_url: String,
    pub relay_device_name: String,
    pub relay_device_id: Option<String>,
    pub relay_device_label: Option<String>,
    pub relay_device_token: Option<String>,
    pub relay_last_connected_at: Option<DateTime<Utc>>,
    pub relay_last_error: Option<String>,
    #[serde(default)]
    pub storage_quota_gb: Option<f64>,
    #[serde(default)]
    pub max_retry_attempts: Option<u32>,
    #[serde(default)]
    pub remote_pinning_enabled: bool,
    #[serde(default)]
    pub remote_pinning_service_name: Option<String>,
    #[serde(default)]
    pub remote_pinning_service_url: Option<String>,
    #[serde(default)]
    pub remote_pinning_access_token: Option<String>,
    #[serde(default)]
    pub onboarded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeConfigResponse {
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
    pub config_file: String,
    pub storage_quota_gb: Option<f64>,
    pub max_retry_attempts: Option<u32>,
    pub remote_pinning_enabled: bool,
    pub remote_pinning_service_name: Option<String>,
    pub remote_pinning_service_url: Option<String>,
    pub remote_pinning_access_token_configured: bool,
    pub onboarded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBridgeConfigRequest {
    pub download_root_dir: Option<String>,
    pub sync_enabled: Option<bool>,
    pub local_gateway_base_url: Option<String>,
    pub public_gateway_base_url: Option<String>,
    pub relay_enabled: Option<bool>,
    pub relay_server_url: Option<String>,
    pub relay_device_name: Option<String>,
    #[serde(default)]
    pub storage_quota_gb: Option<Option<f64>>,
    #[serde(default)]
    pub max_retry_attempts: Option<Option<u32>>,
    #[serde(default)]
    pub remote_pinning_enabled: Option<bool>,
    #[serde(default)]
    pub remote_pinning_service_name: Option<Option<String>>,
    #[serde(default)]
    pub remote_pinning_service_url: Option<Option<String>>,
    #[serde(default)]
    pub remote_pinning_access_token: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBridgeConfigFormRequest {
    pub download_root_dir: String,
    pub sync_enabled: Option<String>,
    pub local_gateway_base_url: String,
    pub public_gateway_base_url: String,
    pub relay_enabled: Option<String>,
    pub relay_server_url: String,
    pub relay_device_name: String,
    #[serde(default)]
    pub storage_quota_gb: Option<String>,
    #[serde(default)]
    pub max_retry_attempts: Option<String>,
    #[serde(default)]
    pub remote_pinning_enabled: Option<String>,
    #[serde(default)]
    pub remote_pinning_service_name: Option<String>,
    #[serde(default)]
    pub remote_pinning_service_url: Option<String>,
    #[serde(default)]
    pub remote_pinning_access_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RootPageQuery {
    pub session_id: Option<String>,
    pub relay_server_url: Option<String>,
    pub pairing_code: Option<String>,
    pub device_name: Option<String>,
    pub autolink: Option<String>,
    pub linked: Option<String>,
    pub unlinked: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SettingsPageQuery {
    pub saved: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ShareWorkViewQuery {
    pub session_secret: String,
    pub title: String,
    pub contract_address: String,
    pub token_id: String,
    pub foundation_url: Option<String>,
    pub metadata_cid: Option<String>,
    pub media_cid: Option<String>,
    pub artist_username: Option<String>,
}
