//! Background task that maintains the relay WebSocket connection and
//! dispatches incoming share-work jobs to the pin domain.
#![allow(clippy::too_many_lines, clippy::cognitive_complexity, clippy::pedantic, clippy::nursery)]

use anyhow::{Context, anyhow};
use futures_util::{SinkExt, StreamExt};
use tokio::time::{Duration, sleep};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use tracing::{info, warn};

use crate::{
    AppState,
    model::{
        pin::service::pin_work_payload,
        relay::{
            service::{
                clear_relay_link, remember_relay_error, remember_relay_success,
                send_relay_inventory,
            },
            types::{
                RelayForceDisconnectMessage, RelayJobMessage, RelayJobResultMessage,
                RelayRequestInventoryMessage, RelayShareWorkPayload, RelayWelcomeMessage,
                build_relay_socket_url,
            },
        },
        system::service::notify_work_share_success,
    },
    util::text::is_valid_cid,
};

/// Prefix matched in `spawn_relay_socket_loop` to detect a server-initiated
/// force-disconnect. Keep this literal stable with the `anyhow!` message below.
const FORCE_DISCONNECT_ERROR_PREFIX: &str = "Archive relay disconnected";

/// Enforce hard size limits on relay share-work payload fields before the
/// pin pipeline sees them. Prevents an attacker who has already compromised
/// the relay server from pushing oversize / malformed values into the local
/// filesystem, logs, and notification OS APIs.
fn validate_share_work_payload(payload: &RelayShareWorkPayload) -> anyhow::Result<()> {
    if payload.title.len() > 512 {
        return Err(anyhow!("SHARE_WORK title exceeds 512 bytes"));
    }
    if payload.contract_address.len() > 128 {
        return Err(anyhow!("SHARE_WORK contract_address exceeds 128 bytes"));
    }
    if payload.token_id.len() > 128 {
        return Err(anyhow!("SHARE_WORK token_id exceeds 128 bytes"));
    }
    if let Some(artist) = payload.artist_username.as_deref()
        && artist.len() > 256
    {
        return Err(anyhow!("SHARE_WORK artist_username exceeds 256 bytes"));
    }
    if let Some(cid) = payload.metadata_cid.as_deref()
        && !cid.trim().is_empty()
        && !is_valid_cid(cid)
    {
        return Err(anyhow!("SHARE_WORK metadata_cid is not a valid CID"));
    }
    if let Some(cid) = payload.media_cid.as_deref()
        && !cid.trim().is_empty()
        && !is_valid_cid(cid)
    {
        return Err(anyhow!("SHARE_WORK media_cid is not a valid CID"));
    }
    Ok(())
}

pub fn spawn_relay_socket_loop(state: AppState) {
    tokio::spawn(async move {
        let mut backoff_seconds = 2u64;

        loop {
            let config = { state.config.read().await.clone() };

            if !config.relay_enabled
                || config.relay_server_url.trim().is_empty()
                || config.relay_device_token.as_deref().map(str::trim).unwrap_or("").is_empty()
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
                    let force_disconnect =
                        error.to_string().starts_with(FORCE_DISCONNECT_ERROR_PREFIX);
                    warn!("relay socket cycle failed: {error}");
                    if state.config.read().await.relay_enabled {
                        let _ = remember_relay_error(&state, error.to_string()).await;
                    }
                    let delay = if force_disconnect { 30 } else { backoff_seconds };
                    sleep(Duration::from_secs(delay)).await;
                    backoff_seconds =
                        if force_disconnect { 2 } else { (backoff_seconds * 2).min(30) };
                }
            }
        }
    });
}

async fn run_relay_socket_session(state: &AppState) -> anyhow::Result<()> {
    let config = { state.config.read().await.clone() };
    let device_token = config
        .relay_device_token
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("Relay device token is missing"))?;

    let socket_url = build_relay_socket_url(&config.relay_server_url, &device_token)?;
    // Defense-in-depth: send the device token both in the query string (for
    // back-compat with relay servers that still look at `?deviceToken=`) and
    // in an `Authorization: Bearer` header. A future server-side upgrade can
    // drop the query-string variant.
    let mut request = socket_url
        .as_str()
        .into_client_request()
        .context("Unable to build relay websocket client request")?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {device_token}").parse().context("Unable to encode relay auth header")?,
    );
    let (mut socket, response) =
        connect_async(request).await.context("Unable to connect to the archive relay websocket")?;

    remember_relay_success(
        state,
        config.relay_device_id.clone(),
        config.relay_device_label.clone(),
    )
    .await?;

    info!("relay websocket connected: {} ({})", response.status(), config.relay_server_url);

    send_relay_inventory(state, &mut socket).await?;

    while let Some(message) = socket.next().await {
        match message? {
            Message::Text(text) => {
                let value = serde_json::from_str::<serde_json::Value>(&text)
                    .context("Unable to parse relay websocket message")?;
                let message_type = value.get("type").and_then(|item| item.as_str()).unwrap_or("");

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
                                match validate_share_work_payload(&input) {
                                    Ok(()) => {
                                        let work_title = input.title.clone();
                                        let pins = pin_work_payload(state, input)
                                            .await
                                            .map_err(|error| anyhow!(error.message))?;
                                        notify_work_share_success(&work_title, pins.len());
                                        serde_json::to_string(&serde_json::json!({ "pins": pins }))
                                            .map_err(anyhow::Error::from)
                                    }
                                    Err(error) => Err(error),
                                }
                            }
                            "UPDATE_CONFIG" => {
                                let input = serde_json::from_str::<
                                    crate::model::config::UpdateBridgeConfigRequest,
                                >(&payload.payload)?;
                                crate::model::config::service::apply_config_update(state, input)
                                    .await
                                    .map_err(|error| anyhow!(error.message))?;
                                Ok("{\"ok\":true}".to_string())
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
                            "{FORCE_DISCONNECT_ERROR_PREFIX} this desktop app: {}",
                            payload.reason.unwrap_or_else(|| "connection closed".to_string())
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
