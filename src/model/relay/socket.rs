//! Background task that maintains the relay WebSocket connection and
//! dispatches incoming share-work jobs to the pin domain.
#![allow(clippy::too_many_lines, clippy::cognitive_complexity, clippy::pedantic, clippy::nursery)]

use anyhow::{Context, anyhow};
use futures_util::{SinkExt, StreamExt};
use tokio::time::{Duration, sleep};
use tokio_tungstenite::{connect_async, tungstenite::Message};
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
};

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
                                let work_title = input.title.clone();
                                let pins = pin_work_payload(state, input)
                                    .await
                                    .map_err(|error| anyhow!(error.message))?;
                                notify_work_share_success(&work_title, pins.len());
                                serde_json::to_string(&serde_json::json!({ "pins": pins }))
                                    .map_err(anyhow::Error::from)
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
                            "Archive relay disconnected this desktop app: {}",
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
