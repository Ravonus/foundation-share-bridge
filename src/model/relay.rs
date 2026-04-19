//! Relay DTOs — both the HTTP `/relay/*` bodies and the typed WebSocket
//! protocol envelopes exchanged with the Foundation relay server, plus the
//! user-facing share-work / share-profile request bodies.

use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::model::{
    config::BridgeConfig,
    pin::{PinCidResult, PinInventoryItem},
};

const FOUNDATION_SITE_HOSTNAME: &str = "foundation.agorix.io";
const FOUNDATION_SOCKET_HOSTNAME: &str = "socket-foundation.agorix.io";

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

/// Translate the configured relay HTTP URL into the WebSocket URL used to
/// connect the device side of the desktop-relay channel. Swaps scheme
/// (`https` → `wss`), rewrites the Foundation site host to the socket host,
/// and appends the `role=device` and `deviceToken` query params.
pub fn build_relay_socket_url(relay_server_url: &str, device_token: &str) -> anyhow::Result<Url> {
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
        .map_err(|()| anyhow!("Unable to convert relay server URL to websocket scheme"))?;

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

/// True only when the bridge has a working relay link: the feature is
/// enabled, we hold a device token, and the last attempt did not record an
/// error string.
pub fn relay_is_connected(config: &BridgeConfig) -> bool {
    config.relay_enabled
        && config.relay_last_error.as_deref().map_or("", str::trim).is_empty()
        && !config.relay_device_token.as_deref().map_or("", str::trim).is_empty()
}
