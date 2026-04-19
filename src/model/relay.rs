//! Relay DTOs — both the HTTP `/relay/*` bodies and the typed WebSocket
//! protocol envelopes exchanged with the Foundation relay server, plus the
//! user-facing share-work / share-profile request bodies.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::pin::{PinCidResult, PinInventoryItem};

#[derive(Debug, Deserialize)]
pub struct RelayLinkRequest {
    pub relay_server_url: Option<String>,
    pub pairing_code: String,
    pub device_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RelayLinkResponse {
    pub relay_enabled: bool,
    pub relay_server_url: String,
    pub relay_device_name: String,
    pub relay_device_id: String,
    pub relay_device_label: String,
    pub linked_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct RelayLinkFormRequest {
    pub relay_server_url: String,
    pub pairing_code: String,
    pub device_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RelayUnlinkResponse {
    pub unlinked: bool,
}

#[derive(Debug, Serialize)]
pub struct RelayInventoryMessage {
    pub r#type: &'static str,
    pub items: Vec<PinInventoryItem>,
}

#[derive(Debug, Serialize)]
pub struct RelayJobResultMessage {
    pub r#type: &'static str,
    pub job_id: String,
    pub status: &'static str,
    pub result_payload: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RelayWelcomeMessage {
    #[serde(rename = "type")]
    pub _type: String,
    pub device_id: Option<String>,
    pub device_label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RelayRequestInventoryMessage {
    #[serde(rename = "type")]
    pub _type: String,
}

#[derive(Debug, Deserialize)]
pub struct RelayForceDisconnectMessage {
    #[serde(rename = "type")]
    pub _type: String,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RelayJobMessage {
    #[serde(rename = "type")]
    pub _type: String,
    pub job_id: String,
    pub kind: String,
    pub payload: String,
}

#[derive(Debug)]
pub struct PairingDeepLink {
    pub relay_server_url: String,
    pub pairing_code: String,
    pub device_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShareWorkRequest {
    pub session_secret: String,
    pub title: String,
    pub contract_address: String,
    pub token_id: String,
    pub foundation_url: Option<String>,
    pub metadata_cid: Option<String>,
    pub media_cid: Option<String>,
    pub artist_username: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayShareWorkPayload {
    pub title: String,
    pub contract_address: String,
    pub token_id: String,
    pub foundation_url: Option<String>,
    pub metadata_cid: Option<String>,
    pub media_cid: Option<String>,
    pub artist_username: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ShareWorkResponse {
    pub share_id: String,
    pub title: String,
    pub contract_address: String,
    pub token_id: String,
    pub foundation_url: Option<String>,
    pub artist_username: Option<String>,
    pub pins: Vec<PinCidResult>,
    pub message: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShareProfileRequest {
    pub session_secret: String,
    pub account_address: String,
    pub username: Option<String>,
    pub label: Option<String>,
    pub cids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ShareProfileResponse {
    pub share_id: String,
    pub account_address: String,
    pub username: Option<String>,
    pub label: Option<String>,
    pub pinned_count: usize,
    pub pins: Vec<PinCidResult>,
    pub message: &'static str,
}
