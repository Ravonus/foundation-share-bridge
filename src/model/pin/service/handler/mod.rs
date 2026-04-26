//! Pin + sync HTTP handlers. Each handler is `pub` but explicitly called from
//! the router wiring — the parent `mod.rs` does not re-export these.
#![allow(clippy::too_many_lines, clippy::cognitive_complexity, clippy::pedantic, clippy::nursery)]

mod upload;

use std::collections::HashSet;

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
};
use chrono::Utc;
use futures_util::{StreamExt, stream};

pub use upload::{add_files, add_files_form};

use crate::{
    AppError, AppState,
    model::{
        config::service::persist_bridge_state,
        pin::{
            PinCidRequest, PinCidResult, PinVerification, PinsPageQuery, PinsPageResponse,
            PinsResponse, RepairNowResponse, RetryPinResponse, RetrySyncResponse,
            SetPinTagsRequest, SetPinTagsResponse, SyncNowResponse, UnwatchPinsRequest,
            UnwatchPinsResponse, VerifyPinsRequest, VerifyPinsResponse, WatchPinInput,
            client::{
                kubo::pin_single_cid, remote::submit_to_remote_pinning_service,
                sync::sync_cid_to_download_dir,
            },
            inventory::{
                categorize_pin_error, parse_inventory_cursor, resolve_inventory_page_size,
            },
            service::{
                check_cid_network_providers, diagnose_pin, list_local_pin_inventory,
                list_local_pin_inventory_page, pin_and_watch_cid, remember_pin_verification,
                remember_watched_pin, repair_watched_pins, resolve_verify_targets,
                sync_all_watched_pins,
            },
        },
        session::service::validate_session,
        system::DiagnoseResponse,
    },
    util::{
        data::unique_trimmed_strings,
        text::{is_valid_cid, sanitize_custom_tag},
    },
};

const VERIFY_CONCURRENCY: usize = 6;

pub async fn list_pins(State(state): State<AppState>) -> Result<Json<PinsResponse>, AppError> {
    let response = list_local_pin_inventory(&state).await.map_err(AppError::internal)?;
    Ok(Json(response))
}

pub async fn list_pins_page(
    State(state): State<AppState>,
    Query(query): Query<PinsPageQuery>,
) -> Result<Json<PinsPageResponse>, AppError> {
    let cursor = parse_inventory_cursor(query.cursor.as_deref());
    let limit = resolve_inventory_page_size(query.limit);
    let response =
        list_local_pin_inventory_page(&state, cursor, limit).await.map_err(AppError::internal)?;
    Ok(Json(response))
}

pub async fn repair_now(
    State(state): State<AppState>,
) -> Result<Json<RepairNowResponse>, AppError> {
    let outcome = repair_watched_pins(&state).await.map_err(AppError::internal)?;

    Ok(Json(RepairNowResponse {
        repaired: outcome.repaired,
        healthy: outcome.healthy,
        failed: outcome.failed,
        message: "Repair cycle completed.",
    }))
}

pub async fn verify_pins(
    State(state): State<AppState>,
    Json(input): Json<VerifyPinsRequest>,
) -> Result<Json<VerifyPinsResponse>, AppError> {
    let targets = resolve_verify_targets(&state, input.cids.as_deref()).await;
    let mut results = stream::iter(targets.into_iter().enumerate().map(|(index, cid)| {
        let state = state.clone();
        async move { (index, check_cid_network_providers(&state, &cid).await) }
    }))
    .buffer_unordered(VERIFY_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;
    results.sort_by_key(|(index, _)| *index);

    let mut ordered_results = Vec::with_capacity(results.len());
    for (_, result) in results {
        remember_pin_verification(&state, &result).await?;
        ordered_results.push(result);
    }

    Ok(Json(VerifyPinsResponse { checked_at: Utc::now(), results: ordered_results }))
}

pub async fn unwatch_pins(
    State(state): State<AppState>,
    Json(input): Json<UnwatchPinsRequest>,
) -> Result<Json<UnwatchPinsResponse>, AppError> {
    let cids = unique_trimmed_strings(input.cids);
    if cids.is_empty() {
        return Err(AppError::bad_request(
            "Provide at least one CID to remove from the forever-watch list.",
        ));
    }

    let mut removed = 0_usize;
    let mut missing = 0_usize;
    {
        let mut persistent = state.persistent.write().await;
        persistent.updated_at = Some(Utc::now());

        for cid in cids {
            if persistent.watched_pins.remove(&cid).is_some() {
                removed += 1;
            } else {
                missing += 1;
            }
        }
    }

    persist_bridge_state(&state).await.map_err(AppError::internal)?;

    Ok(Json(UnwatchPinsResponse {
        removed,
        missing,
        message: "Removed these roots from the forever-watch list. Existing IPFS pins were left alone.",
    }))
}

pub async fn verify_single_pin(
    AxumPath(cid): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<PinVerification>, AppError> {
    if !is_valid_cid(&cid) {
        return Err(AppError::bad_request("Invalid CID"));
    }
    let result = check_cid_network_providers(&state, &cid).await;
    remember_pin_verification(&state, &result).await?;
    Ok(Json(result))
}

pub async fn sync_now(State(state): State<AppState>) -> Result<Json<SyncNowResponse>, AppError> {
    let outcome = sync_all_watched_pins(&state, true).await.map_err(AppError::internal)?;

    Ok(Json(SyncNowResponse {
        synced: outcome.synced,
        failed: outcome.failed,
        skipped: outcome.skipped,
        message: "Sync cycle completed.",
    }))
}

pub async fn pin_cid(
    State(state): State<AppState>,
    Json(input): Json<PinCidRequest>,
) -> Result<Json<PinCidResult>, AppError> {
    let secret = input
        .session_secret
        .as_deref()
        .ok_or_else(|| AppError::unauthorized("session_secret is required to pin a CID"))?;
    validate_session(&state, secret).await?;

    if !is_valid_cid(&input.cid) {
        return Err(AppError::bad_request("Invalid CID"));
    }

    let result = pin_and_watch_cid(
        &state,
        WatchPinInput {
            cid: input.cid.clone(),
            label: input.label.clone(),
            preferred_file_name: None,
            source_kind: "manual".to_string(),
            title: None,
            contract_address: None,
            token_id: None,
            foundation_url: None,
            artist_username: None,
            account_address: None,
            username: None,
        },
    )
    .await?;

    Ok(Json(result))
}

pub async fn diagnose_single_pin(
    AxumPath(cid): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<DiagnoseResponse>, AppError> {
    let trimmed = cid.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("CID is required"));
    }
    if !is_valid_cid(trimmed) {
        return Err(AppError::bad_request("Invalid CID"));
    }
    Ok(Json(diagnose_pin(&state, trimmed).await))
}

pub async fn retry_pin_now(
    AxumPath(cid): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<RetryPinResponse>, AppError> {
    let trimmed = cid.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("CID is required"));
    }
    if !is_valid_cid(&trimmed) {
        return Err(AppError::bad_request("Invalid CID"));
    }

    {
        let mut persistent = state.persistent.write().await;
        if let Some(existing) = persistent.watched_pins.get_mut(&trimmed) {
            existing.next_retry_at = None;
        } else {
            return Err(AppError::bad_request("CID is not watched by this bridge"));
        }
        persistent.updated_at = Some(Utc::now());
    }

    let snapshot = {
        state
            .persistent
            .read()
            .await
            .watched_pins
            .get(&trimmed)
            .cloned()
            .ok_or_else(|| AppError::bad_request("CID disappeared during retry"))?
    };

    match pin_single_cid(&state, &trimmed, snapshot.label.clone()).await {
        Ok(_) => {
            remember_watched_pin(
                &state,
                WatchPinInput {
                    cid: snapshot.cid.clone(),
                    label: snapshot.label.clone(),
                    preferred_file_name: snapshot.preferred_file_name.clone(),
                    source_kind: snapshot.source_kind.clone(),
                    title: snapshot.title.clone(),
                    contract_address: snapshot.contract_address.clone(),
                    token_id: snapshot.token_id.clone(),
                    foundation_url: snapshot.foundation_url.clone(),
                    artist_username: snapshot.artist_username.clone(),
                    account_address: snapshot.account_address.clone(),
                    username: snapshot.username.clone(),
                },
                snapshot.pin_reference.clone(),
                None,
                true,
            )
            .await?;
            Ok(Json(RetryPinResponse {
                cid: trimmed,
                pinned: true,
                used_remote_service: None,
                message: "Pin refreshed locally.".to_string(),
            }))
        }
        Err(error) => {
            let message = error.message.clone();
            let (_category_label, hint) = categorize_pin_error(&message);
            let hint_name = snapshot.title.clone().or_else(|| Some(trimmed.clone()));
            let remote_result =
                submit_to_remote_pinning_service(&state, &trimmed, hint_name.as_deref()).await;
            let (used_remote, remote_err) = match remote_result {
                Ok(Some(service)) => (Some(service), None),
                Ok(None) => (None, None),
                Err(err) => (None, Some(err.to_string())),
            };
            {
                let mut persistent = state.persistent.write().await;
                let now = Utc::now();
                if let Some(existing) = persistent.watched_pins.get_mut(&trimmed) {
                    existing.last_error = Some(message.clone());
                    existing.error_category = Some(_category_label.to_string());
                    if let Some(service) = &used_remote {
                        existing.remote_pinned = true;
                        existing.remote_pin_service = Some(service.clone());
                        existing.remote_pin_last_attempt_at = Some(now);
                        existing.remote_pin_last_error = None;
                    } else if let Some(err) = &remote_err {
                        existing.remote_pin_last_error = Some(err.clone());
                        existing.remote_pin_last_attempt_at = Some(now);
                    }
                }
                persistent.updated_at = Some(now);
            }
            persist_bridge_state(&state).await.map_err(AppError::internal)?;
            let reply = if let Some(service) = used_remote.clone() {
                format!(
                    "Local pin failed ({hint}), but the remote pinning service {service} accepted it."
                )
            } else {
                format!("Local pin failed. {hint} Detail: {message}")
            };
            Ok(Json(RetryPinResponse {
                cid: trimmed,
                pinned: false,
                used_remote_service: used_remote,
                message: reply,
            }))
        }
    }
}

pub async fn retry_sync_single(
    AxumPath(cid): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<RetrySyncResponse>, AppError> {
    let trimmed = cid.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("CID is required"));
    }
    if !is_valid_cid(&trimmed) {
        return Err(AppError::bad_request("Invalid CID"));
    }
    let exists = state.persistent.read().await.watched_pins.contains_key(&trimmed);
    if !exists {
        return Err(AppError::bad_request("CID is not watched by this bridge"));
    }
    match sync_cid_to_download_dir(&state, &trimmed).await {
        Ok(path) => Ok(Json(RetrySyncResponse {
            cid: trimmed,
            synced: true,
            path: Some(path.display().to_string()),
            error: None,
        })),
        Err(error) => Ok(Json(RetrySyncResponse {
            cid: trimmed,
            synced: false,
            path: None,
            error: Some(error.to_string()),
        })),
    }
}

pub async fn set_pin_tags(
    AxumPath(cid): AxumPath<String>,
    State(state): State<AppState>,
    Json(input): Json<SetPinTagsRequest>,
) -> Result<Json<SetPinTagsResponse>, AppError> {
    let trimmed = cid.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("CID is required"));
    }
    if !is_valid_cid(&trimmed) {
        return Err(AppError::bad_request("Invalid CID"));
    }
    let cleaned: Vec<String> = {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for raw in input.tags {
            if let Some(tag) = sanitize_custom_tag(&raw) {
                let key = tag.to_ascii_lowercase();
                if seen.insert(key) {
                    out.push(tag);
                }
            }
        }
        out
    };
    {
        let mut persistent = state.persistent.write().await;
        let existing = persistent
            .watched_pins
            .get_mut(&trimmed)
            .ok_or_else(|| AppError::bad_request("CID is not watched by this bridge"))?;
        existing.custom_tags = cleaned.clone();
        persistent.updated_at = Some(Utc::now());
    }
    persist_bridge_state(&state).await.map_err(AppError::internal)?;
    Ok(Json(SetPinTagsResponse { cid: trimmed, tags: cleaned }))
}
