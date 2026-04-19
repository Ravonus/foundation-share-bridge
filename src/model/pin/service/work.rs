//! Share-work helpers — pinning a full work payload, resolving gateway
//! display URLs from metadata, and assembling grouped `PinInventoryItem`s.

use std::collections::HashSet;

use super::core::pin_and_watch_cid;
use crate::{
    AppError, AppState,
    model::{
        config::types::BridgeConfig,
        pin::{
            client::{
                discovery::{
                    discover_work_dependency_inputs, load_work_metadata_record,
                    resolve_work_root_file_hints,
                },
                kubo::resolve_single_child_path,
                sync::detect_media_kind_for_url,
            },
            inventory::related_cids_from_members,
            metadata::{
                build_metadata_view, metadata_file_url, metadata_image_url,
                metadata_primary_media_url,
            },
            types::{
                InventorySourcePin, PinCidResult, PinInventoryItem, ResolvedWorkDisplay,
                WatchPinInput,
            },
        },
    },
    util::{
        data::{first_present_error, first_present_string, max_timestamp_by},
        url::{build_gateway_asset_url, build_gateway_url, normalize_asset_url_for_gateway},
    },
};

pub async fn pin_work_payload(
    state: &AppState,
    input: crate::model::relay::types::RelayShareWorkPayload,
) -> Result<Vec<PinCidResult>, AppError> {
    let mut pins = Vec::new();
    let (metadata_file_name, media_file_name) = resolve_work_root_file_hints(state, &input).await;

    if let Some(cid) = input.metadata_cid.as_deref().filter(|cid| !cid.trim().is_empty()) {
        pins.push(
            pin_and_watch_cid(
                state,
                WatchPinInput {
                    cid: cid.to_string(),
                    label: Some("metadata".to_string()),
                    preferred_file_name: metadata_file_name.clone(),
                    source_kind: "work".to_string(),
                    title: Some(input.title.clone()),
                    contract_address: Some(input.contract_address.clone()),
                    token_id: Some(input.token_id.clone()),
                    foundation_url: input.foundation_url.clone(),
                    artist_username: input.artist_username.clone(),
                    account_address: None,
                    username: None,
                },
            )
            .await?,
        );
    }

    if let Some(cid) = input.media_cid.as_deref().filter(|cid| !cid.trim().is_empty()) {
        pins.push(
            pin_and_watch_cid(
                state,
                WatchPinInput {
                    cid: cid.to_string(),
                    label: Some("media".to_string()),
                    preferred_file_name: media_file_name.clone(),
                    source_kind: "work".to_string(),
                    title: Some(input.title.clone()),
                    contract_address: Some(input.contract_address.clone()),
                    token_id: Some(input.token_id.clone()),
                    foundation_url: input.foundation_url.clone(),
                    artist_username: input.artist_username.clone(),
                    account_address: None,
                    username: None,
                },
            )
            .await?,
        );
    }

    for dependency in
        discover_work_dependency_inputs(state, &input, metadata_file_name, media_file_name).await
    {
        pins.push(pin_and_watch_cid(state, dependency).await?);
    }

    Ok(pins)
}

// 5 args are 4 independent hints + state. Handler decomposition (Stage 9)
// will replace them with a typed WorkDisplayInput.
#[allow(clippy::too_many_arguments)]
pub async fn resolve_work_display(
    state: &AppState,
    config: &BridgeConfig,
    metadata_cid: Option<&str>,
    media_cid: Option<&str>,
    token_id: Option<&str>,
) -> ResolvedWorkDisplay {
    let mut display = ResolvedWorkDisplay::default();

    let metadata = if let Some(metadata_cid) = metadata_cid.filter(|value| !value.trim().is_empty())
    {
        load_work_metadata_record(state, metadata_cid, token_id).await
    } else {
        None
    };

    let image_raw = metadata.as_ref().and_then(metadata_image_url);
    let media_raw = metadata.as_ref().and_then(|record| {
        let image = image_raw.clone();
        metadata_primary_media_url(record).or_else(|| metadata_file_url(record)).or(image)
    });
    display.metadata_view = metadata
        .as_ref()
        .and_then(|record| build_metadata_view(record, image_raw.as_deref(), media_raw.as_deref()));

    if let Some(raw) = media_raw.as_deref().filter(|value| !value.trim().is_empty()) {
        display.local_open_url =
            Some(normalize_asset_url_for_gateway(raw, &config.local_gateway_base_url));
        display.public_open_url =
            Some(normalize_asset_url_for_gateway(raw, &config.public_gateway_base_url));
    } else if let Some(media_cid) = media_cid.filter(|value| !value.trim().is_empty()) {
        if let Some(child) = resolve_single_child_path(state, media_cid, &[]).await {
            display.local_open_url =
                Some(build_gateway_asset_url(&config.local_gateway_base_url, media_cid, &child));
            display.public_open_url =
                Some(build_gateway_asset_url(&config.public_gateway_base_url, media_cid, &child));
        } else {
            display.local_open_url =
                Some(build_gateway_url(&config.local_gateway_base_url, media_cid));
            display.public_open_url =
                Some(build_gateway_url(&config.public_gateway_base_url, media_cid));
        }
    } else if let Some(metadata_cid) = metadata_cid.filter(|value| !value.trim().is_empty()) {
        display.local_open_url =
            Some(build_gateway_url(&config.local_gateway_base_url, metadata_cid));
        display.public_open_url =
            Some(build_gateway_url(&config.public_gateway_base_url, metadata_cid));
    }

    if let Some(raw) = image_raw.as_deref().filter(|value| !value.trim().is_empty()) {
        display.preview_local_url =
            Some(normalize_asset_url_for_gateway(raw, &config.local_gateway_base_url));
        display.preview_public_url =
            Some(normalize_asset_url_for_gateway(raw, &config.public_gateway_base_url));
    }

    display.media_kind = detect_media_kind_for_url(
        state,
        display.local_open_url.as_deref().or(display.preview_local_url.as_deref()),
        &[
            media_raw.clone(),
            image_raw.clone(),
            display.local_open_url.clone(),
            display.preview_local_url.clone(),
        ],
    )
    .await;

    if display.preview_local_url.is_none()
        && matches!(display.media_kind.as_deref(), Some("IMAGE" | "VIDEO" | "HTML" | "MODEL"))
    {
        display.preview_local_url.clone_from(&display.local_open_url);
        display.preview_public_url.clone_from(&display.public_open_url);
    }

    display
}

// Aggregates metadata + media + related-pin data from 5+ sources into one
// PinInventoryItem. Long by nature; further DTO decomposition is a follow-up.
#[allow(clippy::too_many_lines)]
pub async fn build_work_inventory_item(
    state: &AppState,
    config: &BridgeConfig,
    members: &[InventorySourcePin],
) -> PinInventoryItem {
    let metadata_member =
        members.iter().find(|member| matches!(member.watched.label.as_deref(), Some("metadata")));
    let media_member =
        members.iter().find(|member| matches!(member.watched.label.as_deref(), Some("media")));
    // SAFETY: callers only invoke this on a non-empty group; `members.first()` is the fallback.
    #[allow(clippy::expect_used)]
    let primary_member = media_member
        .or(metadata_member)
        .or_else(|| members.first())
        .expect("work groups always contain at least one member");

    let metadata_cid = metadata_member.map(|member| member.cid.clone());
    let media_cid = media_member.map(|member| member.cid.clone());
    let display = resolve_work_display(
        state,
        config,
        metadata_cid.as_deref(),
        media_cid.as_deref(),
        primary_member.watched.token_id.as_deref(),
    )
    .await;

    let primary_cid = media_cid
        .clone()
        .or_else(|| metadata_cid.clone())
        .unwrap_or_else(|| primary_member.cid.clone());

    PinInventoryItem {
        cid: primary_cid.clone(),
        pinned: members.iter().all(|member| member.pinned),
        pin_type: members
            .iter()
            .find_map(|member| member.pin_type.clone())
            .or_else(|| Some("watched".to_string())),
        managed: true,
        label: None,
        source_kind: Some("work".to_string()),
        title: first_present_string(members.iter().map(|member| member.watched.title.clone())),
        contract_address: first_present_string(
            members.iter().map(|member| member.watched.contract_address.clone()),
        ),
        token_id: first_present_string(
            members.iter().map(|member| member.watched.token_id.clone()),
        ),
        foundation_url: first_present_string(
            members.iter().map(|member| member.watched.foundation_url.clone()),
        ),
        artist_username: first_present_string(
            members.iter().map(|member| member.watched.artist_username.clone()),
        ),
        account_address: first_present_string(
            members.iter().map(|member| member.watched.account_address.clone()),
        ),
        username: first_present_string(
            members.iter().map(|member| member.watched.username.clone()),
        ),
        added_at: max_timestamp_by(members, |member| Some(member.watched.added_at)),
        last_verified_at: max_timestamp_by(members, |member| member.watched.last_verified_at),
        last_repaired_at: max_timestamp_by(members, |member| member.watched.last_repaired_at),
        last_error: first_present_error(members, |member| member.watched.last_error.as_ref()),
        pin_reference: primary_member.watched.pin_reference.clone(),
        verify_count: members.iter().map(|member| member.watched.verify_count).sum(),
        repair_count: members.iter().map(|member| member.watched.repair_count).sum(),
        sync_path: media_member
            .and_then(|member| member.watched.sync_path.clone())
            .or_else(|| metadata_member.and_then(|member| member.watched.sync_path.clone()))
            .or_else(|| primary_member.watched.sync_path.clone()),
        local_gateway_url: display
            .local_open_url
            .clone()
            .or_else(|| Some(build_gateway_url(&config.local_gateway_base_url, &primary_cid))),
        public_gateway_url: display
            .public_open_url
            .clone()
            .or_else(|| Some(build_gateway_url(&config.public_gateway_base_url, &primary_cid))),
        preview_local_gateway_url: display.preview_local_url.clone(),
        preview_public_gateway_url: display.preview_public_url.clone(),
        media_kind: display.media_kind.clone(),
        metadata_view: display.metadata_view,
        metadata_cid,
        media_cid,
        related_cids: related_cids_from_members(members),
        last_synced_at: max_timestamp_by(members, |member| member.watched.last_synced_at),
        last_sync_error: first_present_error(members, |member| {
            member.watched.last_sync_error.as_ref()
        }),
        sync_count: members.iter().map(|member| member.watched.sync_count).sum(),
        retry_attempts: members
            .iter()
            .map(|member| member.watched.retry_attempts)
            .max()
            .unwrap_or(0),
        next_retry_at: members.iter().filter_map(|member| member.watched.next_retry_at).min(),
        error_category: first_present_error(members, |member| {
            member.watched.error_category.as_ref()
        }),
        provider_count: members.iter().filter_map(|member| member.watched.provider_count).min(),
        provider_checked_at: max_timestamp_by(members, |member| member.watched.provider_checked_at),
        custom_tags: {
            let mut tags = Vec::new();
            let mut seen = HashSet::new();
            for member in members {
                for tag in &member.watched.custom_tags {
                    if seen.insert(tag.clone()) {
                        tags.push(tag.clone());
                    }
                }
            }
            tags
        },
        remote_pinned: members.iter().any(|member| member.watched.remote_pinned),
        remote_pin_service: first_present_string(
            members.iter().map(|member| member.watched.remote_pin_service.clone()),
        ),
        remote_pin_last_error: first_present_error(members, |member| {
            member.watched.remote_pin_last_error.as_ref()
        }),
    }
}
