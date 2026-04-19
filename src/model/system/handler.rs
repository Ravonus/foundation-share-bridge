//! System HTTP handlers: health, storage stats, live status, and export.
#![allow(clippy::too_many_lines, clippy::cognitive_complexity, clippy::pedantic, clippy::nursery)]

use std::collections::{HashMap, HashSet};

use crate::{
    AppError, AppState, OperationStatus,
    model::{
        pin::{ExportQuery, inventory::inventory_work_group_key},
        session::service::validate_session,
        system::{
            ArtistEntry, ArtistSummary, GatewayHealthResponse, HealthResponse, StorageSnapshot,
            probe::gateway_health_probe, service::build_storage_snapshot,
        },
    },
    util::text::csv_escape,
};

use anyhow::anyhow;
use axum::{
    Json,
    extract::{Query, State},
    http::{
        StatusCode,
        header::{HeaderName, HeaderValue},
    },
    response::{IntoResponse, Response},
};
use chrono::Utc;

pub async fn add_private_network_access_header(mut response: Response) -> Response {
    response.headers_mut().insert(
        HeaderName::from_static("access-control-allow-private-network"),
        HeaderValue::from_static("true"),
    );
    response
}

pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let (active_sessions, watched_pin_count, last_repair_cycle_at) = {
        let sessions = state.sessions.read().await;
        let persistent = state.persistent.read().await;
        (sessions.len(), persistent.watched_pins.len(), persistent.last_repair_cycle_at)
    };
    let (
        download_root_dir,
        sync_enabled,
        local_gateway_base_url,
        public_gateway_base_url,
        relay_enabled,
        relay_server_url,
        relay_device_name,
        relay_device_id,
        relay_device_label,
        relay_last_connected_at,
        relay_last_error,
        remote_pinning_enabled,
        onboarded,
    ) = {
        let config = state.config.read().await;
        (
            config.download_root_dir.clone(),
            config.sync_enabled,
            config.local_gateway_base_url.clone(),
            config.public_gateway_base_url.clone(),
            config.relay_enabled,
            config.relay_server_url.clone(),
            config.relay_device_name.clone(),
            config.relay_device_id.clone(),
            config.relay_device_label.clone(),
            config.relay_last_connected_at,
            config.relay_last_error.clone(),
            config.remote_pinning_enabled,
            config.onboarded_at.is_some(),
        )
    };
    let storage = build_storage_snapshot(&state).await;
    let operation = state.operation.read().await.clone();

    Json(HealthResponse {
        status: "ok",
        service: "foundation-share-bridge",
        ipfs_api_url: state.ipfs_api_url.clone(),
        state_file: state.state_file.display().to_string(),
        config_file: state.config_file.display().to_string(),
        active_sessions,
        watched_pin_count,
        repair_interval_seconds: state.repair_interval_seconds,
        last_repair_cycle_at,
        download_root_dir,
        sync_enabled,
        local_gateway_base_url,
        public_gateway_base_url,
        relay_enabled,
        relay_server_url,
        relay_device_name,
        relay_device_id,
        relay_device_label,
        relay_last_connected_at,
        relay_last_error,
        now: Utc::now(),
        storage,
        operation,
        remote_pinning_enabled,
        onboarded,
    })
}

pub async fn gateway_health_handler(State(state): State<AppState>) -> Json<GatewayHealthResponse> {
    Json(gateway_health_probe(&state).await)
}

pub async fn storage_stats_handler(State(state): State<AppState>) -> Json<StorageSnapshot> {
    Json(build_storage_snapshot(&state).await)
}

pub async fn live_status_handler(State(state): State<AppState>) -> Json<OperationStatus> {
    Json(state.operation.read().await.clone())
}

pub async fn export_pins_handler(
    State(state): State<AppState>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, AppError> {
    validate_session(&state, &query.session_secret).await?;
    let snapshot = state.persistent.read().await.clone();
    let format = query
        .format
        .as_deref()
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "json".to_string());
    match format.as_str() {
        "csv" => {
            let mut body = String::new();
            body.push_str(
                "cid,title,artist_username,contract_address,token_id,foundation_url,source_kind,label,added_at,last_verified_at,last_repaired_at,verify_count,repair_count,sync_count,last_error,error_category,retry_attempts,remote_pinned,remote_pin_service,custom_tags,sync_path\n",
            );
            for pin in snapshot.watched_pins.values() {
                body.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                    csv_escape(&pin.cid),
                    csv_escape(pin.title.as_deref().unwrap_or("")),
                    csv_escape(pin.artist_username.as_deref().unwrap_or("")),
                    csv_escape(pin.contract_address.as_deref().unwrap_or("")),
                    csv_escape(pin.token_id.as_deref().unwrap_or("")),
                    csv_escape(pin.foundation_url.as_deref().unwrap_or("")),
                    csv_escape(&pin.source_kind),
                    csv_escape(pin.label.as_deref().unwrap_or("")),
                    csv_escape(&pin.added_at.to_rfc3339()),
                    csv_escape(&pin.last_verified_at.map(|t| t.to_rfc3339()).unwrap_or_default()),
                    csv_escape(&pin.last_repaired_at.map(|t| t.to_rfc3339()).unwrap_or_default()),
                    pin.verify_count,
                    pin.repair_count,
                    pin.sync_count,
                    csv_escape(pin.last_error.as_deref().unwrap_or("")),
                    csv_escape(pin.error_category.as_deref().unwrap_or("")),
                    pin.retry_attempts,
                    pin.remote_pinned,
                    csv_escape(pin.remote_pin_service.as_deref().unwrap_or("")),
                    csv_escape(&pin.custom_tags.join(";")),
                    csv_escape(pin.sync_path.as_deref().unwrap_or("")),
                ));
            }
            Ok((
                StatusCode::OK,
                [
                    ("content-type", "text/csv; charset=utf-8"),
                    (
                        "content-disposition",
                        "attachment; filename=\"foundation-share-bridge-pins.csv\"",
                    ),
                ],
                body,
            )
                .into_response())
        }
        _ => {
            let json = serde_json::to_vec_pretty(&snapshot)
                .map_err(|err| AppError::internal(anyhow!("Unable to encode pins: {err}")))?;
            Ok((
                StatusCode::OK,
                [
                    ("content-type", "application/json"),
                    (
                        "content-disposition",
                        "attachment; filename=\"foundation-share-bridge-pins.json\"",
                    ),
                ],
                json,
            )
                .into_response())
        }
    }
}

pub async fn artist_summary_handler(State(state): State<AppState>) -> Json<ArtistSummary> {
    let persistent = state.persistent.read().await.clone();
    let sessions = state.sessions.read().await.clone();
    let mut artist_counts: HashMap<String, HashSet<String>> = HashMap::new();
    let mut works_by_group: HashSet<String> = HashSet::new();
    let mut total_copies = 0_usize;
    let current_username = sessions.values().filter_map(|s| s.profile_username.clone()).next();
    let mut works_by_you = 0_usize;
    for pin in persistent.watched_pins.values() {
        total_copies += 1;
        let group = inventory_work_group_key(pin).unwrap_or_else(|| pin.cid.clone());
        if works_by_group.insert(group.clone()) {
            let artist = pin.artist_username.clone().unwrap_or_else(|| "unknown".to_string());
            artist_counts.entry(artist).or_default().insert(group.clone());
            if let Some(me) = current_username.as_deref()
                && pin
                    .artist_username
                    .as_deref()
                    .map(|v| v.eq_ignore_ascii_case(me))
                    .unwrap_or(false)
            {
                works_by_you += 1;
            }
        }
    }
    let artists_tracked = artist_counts.len();
    let mut top_artists: Vec<ArtistEntry> = artist_counts
        .into_iter()
        .map(|(username, works)| ArtistEntry { artist_username: username, works: works.len() })
        .collect();
    top_artists.sort_by(|a, b| {
        b.works.cmp(&a.works).then_with(|| a.artist_username.cmp(&b.artist_username))
    });
    top_artists.truncate(5);
    Json(ArtistSummary {
        total_works_managed: works_by_group.len(),
        works_by_you,
        artists_tracked,
        top_artists,
        total_copies_pinned: total_copies,
    })
}
