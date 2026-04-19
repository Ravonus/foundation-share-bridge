//! Pin DTOs — everything that describes a watched pin, its verification
//! status, inventory display, repair/sync outcomes, and the internal Kubo
//! `pin ls` response shape.
//!
//! The [`WatchedPin`] record is the persistence root for every CID the bridge
//! tracks; it is serialized into `bridge-state.json` via
//! [`crate::model::config::BridgePersistentState`]. Its field layout is a
//! migration boundary — add new fields with `#[serde(default)]`, never rename
//! or remove existing ones without a migration.

// `large_enum_variant`: the inventory enum is built transiently and consumed
// immediately; boxing would just add an allocation per pin without any
// material benefit.
#![allow(clippy::large_enum_variant)]
// `struct_excessive_bools`: persisted wire formats — see module-level note.
#![allow(clippy::struct_excessive_bools)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchedPin {
    pub cid: String,
    pub label: Option<String>,
    #[serde(default)]
    pub preferred_file_name: Option<String>,
    pub source_kind: String,
    pub title: Option<String>,
    pub contract_address: Option<String>,
    pub token_id: Option<String>,
    pub foundation_url: Option<String>,
    pub artist_username: Option<String>,
    pub account_address: Option<String>,
    pub username: Option<String>,
    pub added_at: DateTime<Utc>,
    pub last_verified_at: Option<DateTime<Utc>>,
    pub last_repaired_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub pin_reference: Option<String>,
    #[serde(default)]
    pub verify_count: u64,
    #[serde(default)]
    pub repair_count: u64,
    pub sync_path: Option<String>,
    pub local_gateway_url: Option<String>,
    pub public_gateway_url: Option<String>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub last_sync_error: Option<String>,
    #[serde(default)]
    pub sync_count: u64,
    #[serde(default)]
    pub retry_attempts: u32,
    #[serde(default)]
    pub next_retry_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub error_category: Option<String>,
    #[serde(default)]
    pub provider_count: Option<usize>,
    #[serde(default)]
    pub provider_checked_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub custom_tags: Vec<String>,
    #[serde(default)]
    pub remote_pinned: bool,
    #[serde(default)]
    pub remote_pin_service: Option<String>,
    #[serde(default)]
    pub remote_pin_last_attempt_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub remote_pin_last_error: Option<String>,
    #[serde(default)]
    pub final_failure_reported_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct PinCidRequest {
    pub session_secret: Option<String>,
    pub cid: String,
    pub label: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PinCidResult {
    pub cid: String,
    pub label: Option<String>,
    pub pinned: bool,
    pub provider: &'static str,
    pub pin_reference: String,
    pub requested_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Clone)]
pub struct AddedFileEntry {
    pub name: String,
    pub cid: String,
    pub size: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct AddFilesResult {
    pub root_cid: String,
    pub label: Option<String>,
    pub pinned: bool,
    pub provider: &'static str,
    pub pin_reference: String,
    pub requested_at: DateTime<Utc>,
    pub file_count: usize,
    pub total_bytes: u64,
    pub wrapped: bool,
    pub entries: Vec<AddedFileEntry>,
}

#[derive(Debug, Serialize)]
pub struct PinsResponse {
    pub total: usize,
    #[serde(rename = "pinnedCount")]
    pub pinned_count: usize,
    #[serde(rename = "managedCount")]
    pub managed_count: usize,
    pub last_repair_cycle_at: Option<DateTime<Utc>>,
    pub items: Vec<PinInventoryItem>,
}

#[derive(Debug, Deserialize)]
pub struct PinsPageQuery {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinsPageResponse {
    pub total: usize,
    pub pinned_count: usize,
    pub managed_count: usize,
    pub next_cursor: Option<String>,
    pub items: Vec<PinInventoryItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinMetadataField {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinMetadataView {
    pub description: Option<String>,
    pub fields: Vec<PinMetadataField>,
    pub attributes: Vec<PinMetadataField>,
    pub raw_json: String,
    pub raw_json_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinInventoryItem {
    pub cid: String,
    pub pinned: bool,
    pub pin_type: Option<String>,
    pub managed: bool,
    pub label: Option<String>,
    pub source_kind: Option<String>,
    pub title: Option<String>,
    pub contract_address: Option<String>,
    pub token_id: Option<String>,
    pub foundation_url: Option<String>,
    pub artist_username: Option<String>,
    pub account_address: Option<String>,
    pub username: Option<String>,
    pub added_at: Option<DateTime<Utc>>,
    pub last_verified_at: Option<DateTime<Utc>>,
    pub last_repaired_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub pin_reference: Option<String>,
    pub verify_count: u64,
    pub repair_count: u64,
    pub sync_path: Option<String>,
    pub local_gateway_url: Option<String>,
    pub public_gateway_url: Option<String>,
    pub preview_local_gateway_url: Option<String>,
    pub preview_public_gateway_url: Option<String>,
    pub media_kind: Option<String>,
    pub metadata_view: Option<PinMetadataView>,
    pub metadata_cid: Option<String>,
    pub media_cid: Option<String>,
    #[serde(default)]
    pub related_cids: Vec<String>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub last_sync_error: Option<String>,
    pub sync_count: u64,
    #[serde(default)]
    pub retry_attempts: u32,
    #[serde(default)]
    pub next_retry_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub error_category: Option<String>,
    #[serde(default)]
    pub provider_count: Option<usize>,
    #[serde(default)]
    pub provider_checked_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub custom_tags: Vec<String>,
    #[serde(default)]
    pub remote_pinned: bool,
    #[serde(default)]
    pub remote_pin_service: Option<String>,
    #[serde(default)]
    pub remote_pin_last_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RepairNowResponse {
    pub repaired: usize,
    pub healthy: usize,
    pub failed: usize,
    pub message: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct VerifyPinsRequest {
    #[serde(default)]
    pub cids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UnwatchPinsRequest {
    pub cids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinVerification {
    pub cid: String,
    pub reachable: bool,
    pub provider_count: usize,
    pub checked_at: DateTime<Utc>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyPinsResponse {
    pub checked_at: DateTime<Utc>,
    pub results: Vec<PinVerification>,
}

#[derive(Debug, Serialize)]
pub struct UnwatchPinsResponse {
    pub removed: usize,
    pub missing: usize,
    pub message: &'static str,
}

#[derive(Debug, Serialize)]
pub struct SyncNowResponse {
    pub synced: usize,
    pub failed: usize,
    pub skipped: usize,
    pub message: &'static str,
}

#[derive(Debug, Default)]
pub struct RepairCycleOutcome {
    pub repaired: usize,
    pub healthy: usize,
    pub failed: usize,
}

#[derive(Debug, Default)]
pub struct SyncOutcome {
    pub synced: usize,
    pub failed: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone)]
pub struct WatchPinInput {
    pub cid: String,
    pub label: Option<String>,
    pub preferred_file_name: Option<String>,
    pub source_kind: String,
    pub title: Option<String>,
    pub contract_address: Option<String>,
    pub token_id: Option<String>,
    pub foundation_url: Option<String>,
    pub artist_username: Option<String>,
    pub account_address: Option<String>,
    pub username: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryPinResponse {
    pub cid: String,
    pub pinned: bool,
    pub used_remote_service: Option<String>,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrySyncResponse {
    pub cid: String,
    pub synced: bool,
    pub path: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPinTagsRequest {
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPinTagsResponse {
    pub cid: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    pub format: Option<String>,
    pub session_secret: String,
}

#[derive(Debug, Deserialize)]
pub struct PinLsEntry {
    #[serde(rename = "Type")]
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PinLsResponse {
    #[serde(rename = "Keys")]
    pub keys: Option<HashMap<String, PinLsEntry>>,
}

#[derive(Debug, Clone)]
pub struct InventorySourcePin {
    pub cid: String,
    pub pinned: bool,
    pub pin_type: Option<String>,
    pub watched: WatchedPin,
}

#[derive(Debug, Clone)]
pub enum InventoryEntryDescriptor {
    Single(InventorySourcePin),
    Work(Vec<InventorySourcePin>),
}

impl InventoryEntryDescriptor {
    pub fn added_at(&self) -> DateTime<Utc> {
        match self {
            Self::Single(source) => source.watched.added_at,
            Self::Work(members) => {
                members.iter().map(|member| member.watched.added_at).max().unwrap_or_else(Utc::now)
            }
        }
    }

    pub fn pinned(&self) -> bool {
        match self {
            Self::Single(source) => source.pinned,
            Self::Work(members) => members.iter().all(|member| member.pinned),
        }
    }
}

#[derive(Debug, Default)]
pub struct ResolvedWorkDisplay {
    pub local_open_url: Option<String>,
    pub public_open_url: Option<String>,
    pub preview_local_url: Option<String>,
    pub preview_public_url: Option<String>,
    pub media_kind: Option<String>,
    pub metadata_view: Option<PinMetadataView>,
}

#[derive(Debug, Clone)]
pub struct DiscoveredDependency {
    pub cid: String,
    pub preferred_file_name: Option<String>,
}
