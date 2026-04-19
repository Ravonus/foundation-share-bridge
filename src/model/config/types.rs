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

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
};

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::pin::WatchedPin;

/// Default relay server URL used when nothing else is configured.
pub const DEFAULT_RELAY_SERVER_URL: &str = "https://foundation.agorix.io";

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
    pub tunnel_enabled: bool,
    #[serde(default)]
    pub tunnel_hostname: Option<String>,
    #[serde(default)]
    pub tunnel_subdomain: Option<String>,
    #[serde(default)]
    pub tunnel_token: Option<String>,
    #[serde(default)]
    pub tunnel_provisioned_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub tunnel_last_error: Option<String>,
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
    pub tunnel_enabled: bool,
    pub tunnel_hostname: Option<String>,
    pub tunnel_last_error: Option<String>,
    pub tunnel_provisioned_at: Option<DateTime<Utc>>,
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
    pub tunnel_enabled: Option<bool>,
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
    pub tunnel_enabled: Option<String>,
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

/// Default download directory — derived from the state file's parent so the
/// sync tree lives next to `bridge-state.json` by default.
pub fn default_download_root_dir(state_file: &Path) -> String {
    state_file
        .parent()
        .map_or_else(|| PathBuf::from("./synced-ipfs"), |parent| parent.join("synced-ipfs"))
        .display()
        .to_string()
}

/// Build a [`BridgeConfig`] seeded entirely from environment variables, with
/// hard-coded fallbacks for each field. Used on first boot before the on-disk
/// config file exists.
pub fn default_bridge_config(state_file: &Path) -> BridgeConfig {
    BridgeConfig {
        download_root_dir: env::var("BRIDGE_DOWNLOAD_ROOT_DIR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default_download_root_dir(state_file)),
        sync_enabled: env::var("BRIDGE_SYNC_ENABLED").ok().is_some_and(|value| {
            matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
        }),
        local_gateway_base_url: env::var("LOCAL_IPFS_GATEWAY_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "http://127.0.0.1:8080".to_string()),
        public_gateway_base_url: env::var("PUBLIC_IPFS_GATEWAY_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "https://ipfs.io".to_string()),
        relay_enabled: env::var("BRIDGE_RELAY_ENABLED").ok().is_some_and(|value| {
            matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
        }),
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
        tunnel_enabled: false,
        tunnel_hostname: None,
        tunnel_subdomain: None,
        tunnel_token: None,
        tunnel_provisioned_at: None,
        tunnel_last_error: None,
        storage_quota_gb: env::var("BRIDGE_STORAGE_QUOTA_GB")
            .ok()
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|value| *value > 0.0),
        max_retry_attempts: env::var("BRIDGE_MAX_RETRY_ATTEMPTS")
            .ok()
            .and_then(|value| value.trim().parse::<u32>().ok()),
        remote_pinning_enabled: env::var("BRIDGE_REMOTE_PINNING_ENABLED").ok().is_some_and(
            |value| {
                matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
            },
        ),
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

/// Detect whether the config file extension indicates YAML (so we parse YAML
/// first and fall back to JSON) vs. the other way round.
pub fn bridge_config_uses_yaml(path: &Path) -> bool {
    matches!(path.extension().and_then(|value| value.to_str()), Some("yaml" | "yml"))
}

/// Parse a bridge config string, trying YAML or JSON first depending on the
/// file extension, then falling back to the other format.
pub fn parse_bridge_config(contents: &str, path: &Path) -> anyhow::Result<BridgeConfig> {
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

/// If `path` is a YAML config, return the sibling `.json` path that older
/// installs may have used. Lets the loader migrate legacy state in place.
pub fn legacy_bridge_json_path(path: &Path) -> Option<PathBuf> {
    if !bridge_config_uses_yaml(path) {
        return None;
    }

    let file_stem = path.file_stem()?.to_str()?;
    let parent = path.parent()?;
    Some(parent.join(format!("{file_stem}.json")))
}
