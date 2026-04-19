//! Relay service layer — link/unlink with the archive server, websocket
//! inventory broadcast, and the share-work / share-profile job handlers that
//! flow through the WebSocket channel.

use std::collections::HashMap;

use anyhow::{Context, anyhow};
use chrono::Utc;
use futures_util::SinkExt;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use super::types::{
    RelayInventoryMessage, RelayLinkRequest, RelayLinkResponse, RelayShareWorkPayload,
    ShareProfileRequest, ShareProfileResponse, ShareWorkRequest, ShareWorkResponse,
};
use crate::{
    AppError, AppState,
    model::{
        config::service::persist_bridge_config,
        pin::{
            service::{list_local_pin_inventory, pin_and_watch_cid, pin_work_payload},
            types::WatchPinInput,
        },
        session::service::validate_session,
        system::service::notify_work_share_success,
    },
    util::url::trim_trailing_slash,
};

pub async fn clear_relay_link(state: &AppState) -> anyhow::Result<()> {
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

pub async fn perform_relay_unlink(state: &AppState, notify_server: bool) -> anyhow::Result<()> {
    let config = { state.config.read().await.clone() };

    if notify_server
        && !config.relay_server_url.trim().is_empty()
        && !config.relay_device_token.as_deref().map_or("", str::trim).is_empty()
    {
        let endpoint =
            format!("{}/api/relay/bridge/unlink", trim_trailing_slash(&config.relay_server_url));

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

// End-to-end device pairing: validate, POST to relay, persist token,
// trigger socket reconnect. Sequential flow — splitting threads half-linked
// state back up. `or_fun_call` silenced on the config-fallback branches
// where the `.trim()` on the guarded config is cheap and local.
#[allow(clippy::too_many_lines, clippy::or_fun_call)]
pub async fn perform_relay_link(
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

    let endpoint = format!("{}/api/relay/bridge/claim", trim_trailing_slash(&relay_server_url));

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
        return Err(AppError::internal(anyhow!("Relay pairing claim failed: {body}")));
    }

    let payload = response.json::<serde_json::Value>().await.map_err(|error| {
        AppError::internal(anyhow!("Unable to parse relay pairing response: {error}"))
    })?;

    let device_id = payload
        .get("deviceId")
        .and_then(|value| value.as_str())
        .ok_or_else(|| AppError::internal(anyhow!("Relay response did not include a deviceId")))?;
    let device_label =
        payload.get("deviceLabel").and_then(|value| value.as_str()).ok_or_else(|| {
            AppError::internal(anyhow!("Relay response did not include a deviceLabel"))
        })?;
    let device_token =
        payload.get("deviceToken").and_then(|value| value.as_str()).ok_or_else(|| {
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

    persist_bridge_config(state).await.map_err(AppError::internal)?;

    let (relay_server_url, relay_device_name) = {
        let config = state.config.read().await;
        (config.relay_server_url.clone(), config.relay_device_name.clone())
    };

    Ok(RelayLinkResponse {
        relay_enabled: true,
        relay_server_url,
        relay_device_name,
        relay_device_id: device_id.to_string(),
        relay_device_label: device_label.to_string(),
        linked_at,
    })
}

pub async fn send_relay_inventory(
    state: &AppState,
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> anyhow::Result<()> {
    let snapshot = list_local_pin_inventory(state).await?;
    let payload = RelayInventoryMessage { r#type: "device.inventory", items: snapshot.items };

    socket
        .send(Message::Text(
            serde_json::to_string(&payload).context("Unable to encode relay inventory")?.into(),
        ))
        .await
        .context("Unable to send relay inventory to the archive server")?;

    Ok(())
}

pub async fn remember_relay_success(
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

pub async fn remember_relay_error(state: &AppState, message: String) -> anyhow::Result<()> {
    {
        let mut config = state.config.write().await;
        config.relay_last_error = Some(message);
    }

    persist_bridge_config(state).await
}

pub async fn send_relay_pin_failure(
    state: &AppState,
    pin: &crate::model::pin::types::WatchedPin,
    message: &str,
) -> anyhow::Result<bool> {
    let (relay_enabled, relay_server_url, device_token) = {
        let config = state.config.read().await;
        (config.relay_enabled, config.relay_server_url.clone(), config.relay_device_token.clone())
    };
    if !relay_enabled {
        return Ok(false);
    }
    let Some(token) = device_token.filter(|value| !value.trim().is_empty()) else {
        return Ok(false);
    };
    if relay_server_url.trim().is_empty() {
        return Ok(false);
    }
    let endpoint =
        format!("{}/api/relay/bridge/pin-failure", trim_trailing_slash(&relay_server_url));
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
    let response = state
        .http
        .post(endpoint)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await;
    match response {
        Ok(resp) if resp.status().is_success() => Ok(true),
        Ok(resp) => Err(anyhow!("Relay pin-failure callback returned {}", resp.status())),
        Err(error) => Err(anyhow!(error)),
    }
}

pub async fn share_work_inner(
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

    notify_work_share_success(&input.title, pins.len());

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

pub async fn share_profile_inner(
    state: &AppState,
    input: ShareProfileRequest,
) -> Result<ShareProfileResponse, AppError> {
    validate_session(state, &input.session_secret).await?;

    let mut seen = HashMap::<String, Option<String>>::new();
    for cid in input.cids {
        let trimmed = cid.trim();
        if !trimmed.is_empty() {
            seen.entry(trimmed.to_string()).or_insert_with(|| Some("profile".to_string()));
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
