//! Pin service core — watched-pin bookkeeping, lifecycle mutations, and
//! the verify/diagnose helpers that back higher-level repair and inventory
//! code in sibling modules.

use std::collections::HashSet;

use anyhow::anyhow;
use chrono::{DateTime, Utc};
use tracing::{info, warn};

use crate::{
    AppError, AppState,
    model::{
        config::service::persist_bridge_state,
        pin::{
            client::{
                kubo::{is_cid_pinned, pin_single_cid},
                remote::submit_to_remote_pinning_service,
                sync::sync_cid_if_enabled,
            },
            inventory::categorize_pin_error,
            types::{PinCidResult, PinVerification, WatchPinInput, WatchedPin},
        },
        relay::service::send_relay_pin_failure,
    },
};

pub async fn pin_and_watch_cid(
    state: &AppState,
    input: WatchPinInput,
) -> Result<PinCidResult, AppError> {
    let result = pin_single_cid(state, &input.cid, input.label.clone()).await?;
    remember_watched_pin(state, input.clone(), Some(result.pin_reference.clone()), None, true)
        .await?;
    run_post_pin_side_effects(state, &input).await;
    Ok(result)
}

/// Fire-and-forget side effects that should not block the pin path: folder
/// sync + eager replication to the configured remote pinning service. The
/// tunnel makes this node reachable over HTTPS; libp2p stays behind NAT, so
/// replication gives the pin a publicly-dialable provider the moment it
/// lands here and ipfs.io / dweb.link start resolving it.
async fn run_post_pin_side_effects(state: &AppState, input: &WatchPinInput) {
    run_sync_side_effect(state, &input.cid).await;
    run_remote_replication_side_effect(state, &input.cid, input.title.as_deref()).await;
}

async fn run_sync_side_effect(state: &AppState, cid: &str) {
    if let Err(error) = sync_cid_if_enabled(state, cid).await {
        warn!("sync after pin failed for {}: {}", cid, error);
    }
}

async fn run_remote_replication_side_effect(state: &AppState, cid: &str, name_hint: Option<&str>) {
    if let Err(error) = replicate_to_remote_service(state, cid, name_hint).await {
        warn!("remote pin replication failed for {}: {error}", cid);
    }
}

async fn replicate_to_remote_service(
    state: &AppState,
    cid: &str,
    name_hint: Option<&str>,
) -> anyhow::Result<()> {
    let enabled = { state.config.read().await.remote_pinning_enabled };
    if !enabled {
        return Ok(());
    }
    let hint = name_hint.map(str::trim).filter(|value| !value.is_empty());
    match submit_to_remote_pinning_service(state, cid, hint).await {
        Ok(Some(service)) => {
            info!("remote pin {} replicated via {} on first pin", cid, service);
            mark_pin_remotely_replicated(state, cid, &service).await
        }
        Ok(None) => Ok(()),
        Err(error) => {
            mark_pin_remote_error(state, cid, &error.to_string()).await?;
            Err(error)
        }
    }
}

async fn mark_pin_remotely_replicated(
    state: &AppState,
    cid: &str,
    service: &str,
) -> anyhow::Result<()> {
    {
        let mut persistent = state.persistent.write().await;
        let now = Utc::now();
        if let Some(existing) = persistent.watched_pins.get_mut(cid) {
            existing.remote_pinned = true;
            existing.remote_pin_service = Some(service.to_string());
            existing.remote_pin_last_attempt_at = Some(now);
            existing.remote_pin_last_error = None;
        }
        persistent.updated_at = Some(now);
    }
    persist_bridge_state(state).await
}

async fn mark_pin_remote_error(state: &AppState, cid: &str, message: &str) -> anyhow::Result<()> {
    {
        let mut persistent = state.persistent.write().await;
        let now = Utc::now();
        if let Some(existing) = persistent.watched_pins.get_mut(cid) {
            existing.remote_pin_last_attempt_at = Some(now);
            existing.remote_pin_last_error = Some(message.to_string());
        }
        persistent.updated_at = Some(now);
    }
    persist_bridge_state(state).await
}

// 5 args bundle the incoming pin request plus 4 identifying fields.
//
// `or_fun_call` / `significant_drop_tightening` / `assigning_clones` are
// silenced on the inner field-copy block — the clones fire only when `input`
// fields are `None`, and the lock scope intentionally covers the whole insert.
#[allow(
    clippy::too_many_arguments,
    clippy::or_fun_call,
    clippy::significant_drop_tightening,
    clippy::assigning_clones
)]
pub async fn remember_watched_pin(
    state: &AppState,
    input: WatchPinInput,
    pin_reference: Option<String>,
    last_error: Option<String>,
    just_repaired: bool,
) -> Result<(), AppError> {
    {
        let mut persistent = state.persistent.write().await;
        let now = Utc::now();

        persistent.updated_at = Some(now);

        if let Some(existing) = persistent.watched_pins.get_mut(&input.cid) {
            existing.label = input.label.or(existing.label.clone());
            existing.preferred_file_name =
                input.preferred_file_name.or(existing.preferred_file_name.clone());
            existing.title = input.title.or(existing.title.clone());
            existing.contract_address =
                input.contract_address.or(existing.contract_address.clone());
            existing.token_id = input.token_id.or(existing.token_id.clone());
            existing.foundation_url = input.foundation_url.or(existing.foundation_url.clone());
            existing.artist_username = input.artist_username.or(existing.artist_username.clone());
            existing.account_address = input.account_address.or(existing.account_address.clone());
            existing.username = input.username.or(existing.username.clone());
            existing.source_kind = input.source_kind;
            existing.last_verified_at = Some(now);
            existing.pin_reference = pin_reference.or(existing.pin_reference.clone());
            existing.last_error = last_error.clone();
            existing.verify_count += 1;

            if just_repaired {
                existing.last_repaired_at = Some(now);
                existing.repair_count += 1;
                existing.retry_attempts = 0;
                existing.next_retry_at = None;
                existing.error_category = None;
                existing.final_failure_reported_at = None;
            }
        } else {
            persistent.watched_pins.insert(
                input.cid.clone(),
                WatchedPin {
                    cid: input.cid,
                    label: input.label,
                    preferred_file_name: input.preferred_file_name,
                    source_kind: input.source_kind,
                    title: input.title,
                    contract_address: input.contract_address,
                    token_id: input.token_id,
                    foundation_url: input.foundation_url,
                    artist_username: input.artist_username,
                    account_address: input.account_address,
                    username: input.username,
                    added_at: now,
                    last_verified_at: Some(now),
                    last_repaired_at: if just_repaired { Some(now) } else { None },
                    last_error,
                    pin_reference,
                    verify_count: 1,
                    repair_count: u64::from(just_repaired),
                    sync_path: None,
                    local_gateway_url: None,
                    public_gateway_url: None,
                    last_synced_at: None,
                    last_sync_error: None,
                    sync_count: 0,
                    retry_attempts: 0,
                    next_retry_at: None,
                    error_category: None,
                    provider_count: None,
                    provider_checked_at: None,
                    custom_tags: Vec::new(),
                    remote_pinned: false,
                    remote_pin_service: None,
                    remote_pin_last_attempt_at: None,
                    remote_pin_last_error: None,
                    final_failure_reported_at: None,
                },
            );
        }
    }

    persist_bridge_state(state).await.map_err(AppError::internal)
}

pub async fn mark_pin_checked(
    state: &AppState,
    cid: &str,
    last_error: Option<String>,
) -> Result<(), AppError> {
    {
        let mut persistent = state.persistent.write().await;
        let now = Utc::now();

        if let Some(existing) = persistent.watched_pins.get_mut(cid) {
            existing.last_verified_at = Some(now);
            existing.last_error = last_error;
            existing.verify_count += 1;
        }

        persistent.updated_at = Some(now);
    }

    persist_bridge_state(state).await.map_err(AppError::internal)
}

// 5 args: state + cid + sync_path + local_gateway_url + public_gateway_url —
// 4 independent columns on WatchedPin, no natural struct to bundle.
#[allow(clippy::too_many_arguments)]
pub async fn mark_pin_synced(
    state: &AppState,
    cid: &str,
    sync_path: String,
    local_gateway_url: String,
    public_gateway_url: String,
) -> anyhow::Result<()> {
    {
        let mut persistent = state.persistent.write().await;
        let now = Utc::now();

        if let Some(existing) = persistent.watched_pins.get_mut(cid) {
            existing.sync_path = Some(sync_path);
            existing.local_gateway_url = Some(local_gateway_url);
            existing.public_gateway_url = Some(public_gateway_url);
            existing.last_synced_at = Some(now);
            existing.last_sync_error = None;
            existing.sync_count += 1;
        }

        persistent.updated_at = Some(now);
    }

    persist_bridge_state(state).await
}

pub async fn mark_pin_sync_failed(
    state: &AppState,
    cid: &str,
    message: String,
) -> anyhow::Result<()> {
    {
        let mut persistent = state.persistent.write().await;
        let now = Utc::now();

        if let Some(existing) = persistent.watched_pins.get_mut(cid) {
            existing.last_sync_error = Some(message);
        }

        persistent.updated_at = Some(now);
    }

    persist_bridge_state(state).await
}

pub async fn record_pin_repaired(state: &AppState, pin: &WatchedPin) -> anyhow::Result<()> {
    let cid = pin.cid.clone();
    {
        let mut persistent = state.persistent.write().await;
        let now = Utc::now();
        if let Some(existing) = persistent.watched_pins.get_mut(&cid) {
            existing.last_verified_at = Some(now);
            existing.last_error = None;
            existing.error_category = None;
            existing.retry_attempts = 0;
            existing.next_retry_at = None;
            existing.final_failure_reported_at = None;
            existing.verify_count += 1;
        }
        persistent.updated_at = Some(now);
    }
    persist_bridge_state(state).await
}

// Failure-path orchestrator: update state, categorize, schedule retry, maybe
// hand off to remote pinning, maybe notify relay. Single transaction by design.
#[allow(clippy::cognitive_complexity)]
pub async fn record_pin_failure(
    state: &AppState,
    pin: &WatchedPin,
    message: &str,
    max_attempts: u32,
) -> anyhow::Result<()> {
    let (category_label, _hint) = categorize_pin_error(message);
    let next_attempt = pin.retry_attempts.saturating_add(1);
    let next_retry_at = compute_next_retry_at(state, next_attempt).await;
    let should_try_remote = next_attempt >= max_attempts
        && category_label != "invalid_cid"
        && category_label != "unauthorized";

    let mut remote_service: Option<String> = None;
    let mut remote_error: Option<String> = None;

    if should_try_remote {
        let hint = pin.title.clone().or_else(|| Some(pin.cid.clone()));
        match submit_to_remote_pinning_service(state, &pin.cid, hint.as_deref()).await {
            Ok(Some(service)) => {
                info!("remote pin service accepted {} via {}", pin.cid, service);
                remote_service = Some(service);
            }
            Ok(None) => {}
            Err(error) => {
                warn!("remote pin service rejected {}: {}", pin.cid, error);
                remote_error = Some(error.to_string());
            }
        }
    }

    let mut should_notify_relay = false;
    {
        let mut persistent = state.persistent.write().await;
        let now = Utc::now();
        if let Some(existing) = persistent.watched_pins.get_mut(&pin.cid) {
            existing.last_verified_at = Some(now);
            existing.last_error = Some(message.to_string());
            existing.error_category = Some(category_label.to_string());
            existing.retry_attempts = next_attempt;
            existing.next_retry_at = Some(next_retry_at);
            existing.verify_count += 1;

            if let Some(service) = &remote_service {
                existing.remote_pinned = true;
                existing.remote_pin_service = Some(service.clone());
                existing.remote_pin_last_attempt_at = Some(now);
                existing.remote_pin_last_error = None;
            } else if let Some(err) = &remote_error {
                existing.remote_pin_last_error = Some(err.clone());
                existing.remote_pin_last_attempt_at = Some(now);
            }

            if next_attempt >= max_attempts && existing.final_failure_reported_at.is_none() {
                existing.final_failure_reported_at = Some(now);
                should_notify_relay = true;
            }
        }
        persistent.updated_at = Some(now);
    }

    persist_bridge_state(state).await?;

    if should_notify_relay {
        let latest = state.persistent.read().await.watched_pins.get(&pin.cid).cloned();
        if let Some(latest) = latest
            && let Err(error) = send_relay_pin_failure(state, &latest, message).await
        {
            warn!("relay pin-failure callback failed for {}: {error}", pin.cid);
        }
    }

    Ok(())
}

pub async fn remember_pin_verification(
    state: &AppState,
    result: &PinVerification,
) -> Result<(), AppError> {
    mark_pin_checked(state, &result.cid, result.error.clone()).await
}

pub async fn check_cid_network_providers(state: &AppState, cid: &str) -> PinVerification {
    let trimmed = cid.trim();
    let checked_at = Utc::now();

    if trimmed.is_empty() {
        return PinVerification {
            cid: cid.to_string(),
            reachable: false,
            provider_count: 0,
            checked_at,
            error: Some("Empty CID".to_string()),
        };
    }

    // Kubo uses /api/v0/routing/findprovs in newer releases, with
    // /api/v0/dht/findprovs kept as a deprecated alias. We try the modern path
    // first and fall back so older daemons keep working.
    match fetch_provider_count(state, trimmed, "routing/findprovs").await {
        Ok(count) => PinVerification {
            cid: trimmed.to_string(),
            reachable: count > 0,
            provider_count: count,
            checked_at,
            error: None,
        },
        Err(primary_error) => match fetch_provider_count(state, trimmed, "dht/findprovs").await {
            Ok(count) => PinVerification {
                cid: trimmed.to_string(),
                reachable: count > 0,
                provider_count: count,
                checked_at,
                error: None,
            },
            Err(fallback_error) => PinVerification {
                cid: trimmed.to_string(),
                reachable: false,
                provider_count: 0,
                checked_at,
                error: Some(format!(
                    "routing/findprovs failed: {primary_error}; dht/findprovs failed: {fallback_error}"
                )),
            },
        },
    }
}

pub async fn fetch_provider_count(
    state: &AppState,
    cid: &str,
    endpoint: &str,
) -> anyhow::Result<usize> {
    let url = format!(
        "{}/api/v0/{}?arg={}&num-providers=5&verbose=false",
        state.ipfs_api_url.trim_end_matches('/'),
        endpoint,
        cid
    );

    let mut request = state.http.post(url).timeout(std::time::Duration::from_secs(12));
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let mut response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("IPFS {endpoint} responded with status {status}: {body}"));
    }

    // The findprovs endpoint streams ndjson. Each line is a JSON object with
    // a `Responses` array that contains peer IDs. A non-empty peer list on any
    // line means at least one provider was found for this CID.
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => body.extend_from_slice(&chunk),
            Ok(None) => break,
            Err(error) => {
                if body.is_empty() {
                    return Err(anyhow!("Unable to read IPFS {endpoint} response body: {error}"));
                }
                break;
            }
        }
    }

    let body = String::from_utf8_lossy(&body);
    let mut unique_providers = HashSet::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let Some(responses) = value.get("Responses").and_then(serde_json::Value::as_array) else {
            continue;
        };
        for entry in responses {
            if let Some(peer_id) = entry.get("ID").and_then(|v| v.as_str())
                && !peer_id.is_empty()
            {
                unique_providers.insert(peer_id.to_string());
            }
        }
    }

    Ok(unique_providers.len())
}

pub async fn resolve_verify_targets(state: &AppState, requested: Option<&[String]>) -> Vec<String> {
    if let Some(raw) = requested {
        let mut seen = HashSet::new();
        return raw
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty() && seen.insert(value.clone()))
            .collect();
    }

    let persistent = state.persistent.read().await;
    persistent.watched_pins.keys().cloned().collect()
}

pub async fn compute_next_retry_at(state: &AppState, attempt: u32) -> DateTime<Utc> {
    let cap_attempts = { state.config.read().await.max_retry_attempts.unwrap_or(10) };
    let effective = attempt.min(cap_attempts).min(14);
    let base = 30u64.saturating_mul(1u64 << effective.min(10));
    let capped = base.min(60 * 60 * 6);
    // `capped` is at most 21_600 so the cast to i64 never truncates.
    Utc::now()
        + chrono::Duration::seconds(capped.min(i64::MAX as u64).try_into().unwrap_or(i64::MAX))
}

pub async fn diagnose_pin(
    state: &AppState,
    cid: &str,
) -> crate::model::system::types::DiagnoseResponse {
    use crate::model::system::probe::check_gateway_reachability;

    let checked_at = Utc::now();
    let pinned_locally = matches!(is_cid_pinned(state, cid).await, Ok(true));
    let provider_result = check_cid_network_providers(state, cid).await;
    let _ = remember_pin_verification(state, &provider_result).await;

    let (last_error, stored_category) = {
        let persistent = state.persistent.read().await;
        persistent
            .watched_pins
            .get(cid)
            .map_or((None, None), |pin| (pin.last_error.clone(), pin.error_category.clone()))
    };

    let combined_error = provider_result.error.clone().or_else(|| last_error.clone());
    let (category, hint) = combined_error.as_deref().map_or_else(
        || (stored_category.clone(), None),
        |error_message| {
            let (cat, hint) = categorize_pin_error(error_message);
            (Some(cat.to_string()), Some(hint.to_string()))
        },
    );

    let (gateway_local_ok, gateway_public_ok) = check_gateway_reachability(state, cid).await;

    crate::model::system::types::DiagnoseResponse {
        cid: cid.to_string(),
        pinned_locally,
        provider_count: provider_result.provider_count,
        reachable_on_dht: provider_result.reachable,
        error_category: category,
        error_hint: hint,
        last_error,
        raw_error: provider_result.error,
        checked_at,
        gateway_local_ok,
        gateway_public_ok,
    }
}
