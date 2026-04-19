//! Local filesystem sync — mirror pinned CIDs into the configured download
//! directory, measure their on-disk footprint, and sniff media MIME kinds for
//! URLs we've surfaced to the UI.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Context;
use async_recursion::async_recursion;
use tokio::fs;

use crate::{
    AppState,
    model::pin::metadata::detect_media_kind_from_text,
    util::{
        file::{ensure_leaf_file_extension, leaf_name_from_ipfs_path, sniff_leaf_file_extension},
        url::{build_gateway_url, parse_ipfs_path},
    },
};

use super::kubo::download_ipfs_path_recursive;

#[async_recursion]
pub async fn sum_dir_size(path: &Path) -> u64 {
    let Ok(metadata) = fs::metadata(path).await else {
        return 0;
    };
    if metadata.is_file() {
        return metadata.len();
    }
    if !metadata.is_dir() {
        return 0;
    }
    let Ok(mut entries) = fs::read_dir(path).await else {
        return 0;
    };
    let mut total = 0u64;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let child = entry.path();
        total = total.saturating_add(sum_dir_size(&child).await);
    }
    total
}

pub async fn measure_synced_bytes_on_disk(state: &AppState) -> u64 {
    let paths = {
        state
            .persistent
            .read()
            .await
            .watched_pins
            .values()
            .filter_map(|pin| pin.sync_path.clone())
            .collect::<Vec<_>>()
    };
    let mut total = 0u64;
    for path in paths {
        total = total.saturating_add(sum_dir_size(&PathBuf::from(path)).await);
    }
    total
}

pub async fn sync_cid_to_download_dir(state: &AppState, cid: &str) -> anyhow::Result<PathBuf> {
    let config = { state.config.read().await.clone() };
    let root_dir = PathBuf::from(config.download_root_dir.clone()).join(cid.trim());

    let sync_result = async {
        if fs::try_exists(&root_dir).await.unwrap_or(false) {
            let _ = fs::remove_dir_all(&root_dir).await;
        }

        fs::create_dir_all(&root_dir)
            .await
            .with_context(|| format!("Unable to create sync directory {}", root_dir.display()))?;

        download_ipfs_path_recursive(state, &format!("/ipfs/{}", cid.trim()), &root_dir).await?;

        let local_gateway_url = build_gateway_url(&config.local_gateway_base_url, cid);
        let public_gateway_url = build_gateway_url(&config.public_gateway_base_url, cid);

        crate::inline::mark_pin_synced(
            state,
            cid,
            root_dir.display().to_string(),
            local_gateway_url,
            public_gateway_url,
        )
        .await?;

        Ok::<PathBuf, anyhow::Error>(root_dir.clone())
    }
    .await;

    if let Err(error) = &sync_result {
        let _ = crate::inline::mark_pin_sync_failed(state, cid, error.to_string()).await;
    }

    sync_result
}

pub async fn sync_cid_if_enabled(state: &AppState, cid: &str) -> anyhow::Result<bool> {
    let sync_enabled = { state.config.read().await.sync_enabled };
    if !sync_enabled {
        return Ok(false);
    }

    sync_cid_to_download_dir(state, cid).await?;
    Ok(true)
}

/// Resolves the file name to write when persisting a leaf CID from IPFS. Looks
/// at the path tail first, falls back to the watched-pin's preferred name, and
/// finally ensures the extension matches the sniffed magic bytes.
pub async fn resolve_sync_leaf_file_name(
    state: &AppState,
    ipfs_path: &str,
    bytes: &[u8],
) -> String {
    let mut file_name = leaf_name_from_ipfs_path(ipfs_path);

    if file_name.is_none()
        && let Some((cid, relative_path)) = parse_ipfs_path(ipfs_path)
        && relative_path.is_empty()
    {
        file_name = state
            .persistent
            .read()
            .await
            .watched_pins
            .get(&cid)
            .and_then(|pin| pin.preferred_file_name.clone());
    }

    let mut resolved = file_name.unwrap_or_else(|| "content".to_string());
    if let Some(extension) = sniff_leaf_file_extension(bytes) {
        resolved = ensure_leaf_file_extension(&resolved, extension);
    }
    resolved
}

pub async fn detect_media_kind_for_url(
    state: &AppState,
    local_url: Option<&str>,
    hints: &[Option<String>],
) -> Option<String> {
    for value in hints.iter().flatten() {
        if let Some(kind) = detect_media_kind_from_text(value) {
            return Some(kind);
        }
    }

    let url = local_url?.trim();
    if url.is_empty() {
        return None;
    }

    let response = state.http.head(url).timeout(Duration::from_secs(6)).send().await.ok()?;

    if let Some(content_type) = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(detect_media_kind_from_text)
    {
        return Some(content_type);
    }

    detect_media_kind_from_text(response.url().as_str())
}
