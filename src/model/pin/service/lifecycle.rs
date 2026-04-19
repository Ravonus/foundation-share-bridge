//! Repair and sync loops that iterate watched pins and delegate to the
//! lifecycle bookkeeping in `core.rs`.

use anyhow::anyhow;
use chrono::Utc;
use tokio::time::{Duration, interval};
use tracing::{error, warn};

use super::core::{record_pin_failure, record_pin_repaired, remember_watched_pin};
use crate::{
    AppState, OperationStatus,
    model::{
        config::service::persist_bridge_state,
        pin::{
            client::{
                kubo::{is_cid_pinned, pin_single_cid},
                sync::sync_cid_to_download_dir,
            },
            types::{RepairCycleOutcome, SyncOutcome, WatchPinInput},
        },
        system::service::{
            clear_current_operation, set_current_operation, update_current_operation,
        },
    },
};

// Top-level repair-cycle loop: one contiguous state machine over every
// watched pin. Splitting fragments the retry/outcome invariants.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn repair_watched_pins(state: &AppState) -> anyhow::Result<RepairCycleOutcome> {
    let watched =
        { state.persistent.read().await.watched_pins.values().cloned().collect::<Vec<_>>() };

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

    let max_attempts = { state.config.read().await.max_retry_attempts.unwrap_or(10) };

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

        if let Some(next_retry_at) = pin.next_retry_at
            && next_retry_at > now
        {
            outcome.healthy += 1;
            continue;
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

pub async fn sync_all_watched_pins(state: &AppState, force: bool) -> anyhow::Result<SyncOutcome> {
    let watched =
        { state.persistent.read().await.watched_pins.values().cloned().collect::<Vec<_>>() };

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

pub fn spawn_repair_loop(state: AppState) {
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
