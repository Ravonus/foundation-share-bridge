//! Kubo HTTP wrappers — thin async callers over the local Kubo daemon's
//! `/api/v0/...` endpoints for pin lookups, CID pin operations, and IPFS path
//! reads (text, JSON, directory listings, recursive downloads).
//!
//! Every function takes `&AppState` so it can reuse the shared `reqwest::Client`
//! and (optionally) the `Authorization` header configured for the daemon.

use std::{collections::HashMap, path::Path, time::Duration};

use anyhow::{Context, anyhow};
use async_recursion::async_recursion;
use chrono::Utc;
use tokio::fs;

use crate::{
    AppError, AppState,
    model::{
        pin::types::{PinCidResult, PinLsResponse},
        system::KuboRepoStat,
    },
    util::file::sanitize_file_name,
};

// Large-enough bound to capture most dependency-probe inputs (HTML, JSON, SVG,
// glTF manifests) without letting a pathological file exhaust memory.
const MAX_DISCOVERY_TEXT_BYTES: usize = 512 * 1024;

pub async fn list_kubo_pinset(state: &AppState) -> anyhow::Result<HashMap<String, String>> {
    let endpoint = format!("{}/api/v0/pin/ls", state.ipfs_api_url.trim_end_matches('/'));

    let mut request = state.http.post(endpoint);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Unable to list the local IPFS pinset: {body}"));
    }

    let payload = response.json::<PinLsResponse>().await?;
    let mut pins = HashMap::new();
    for (cid, entry) in payload.keys.unwrap_or_default() {
        pins.insert(cid, entry.kind.unwrap_or_else(|| "recursive".to_string()));
    }

    Ok(pins)
}

pub async fn fetch_ipfs_text(state: &AppState, ipfs_path: &str) -> anyhow::Result<Option<String>> {
    let endpoint = format!("{}/api/v0/cat", state.ipfs_api_url.trim_end_matches('/'));
    let mut request = state.http.post(endpoint).query(&[("arg", ipfs_path)]);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request.send().await?;
    if !response.status().is_success() {
        return Ok(None);
    }

    let body = response.bytes().await?;
    if body.is_empty() {
        return Ok(None);
    }

    let bounded = &body[..body.len().min(MAX_DISCOVERY_TEXT_BYTES)];
    if bounded.contains(&0) {
        return Ok(None);
    }

    Ok(Some(String::from_utf8_lossy(bounded).into_owned()))
}

pub async fn fetch_ipfs_json(
    state: &AppState,
    ipfs_path: &str,
) -> anyhow::Result<Option<serde_json::Value>> {
    let endpoint = format!("{}/api/v0/cat", state.ipfs_api_url.trim_end_matches('/'));
    let mut request = state.http.post(endpoint).query(&[("arg", ipfs_path)]);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request.send().await?;
    if !response.status().is_success() {
        return Ok(None);
    }

    let body = response.bytes().await?;
    let parsed = serde_json::from_slice::<serde_json::Value>(&body).ok();
    Ok(parsed)
}

pub async fn resolve_single_child_path(
    state: &AppState,
    cid: &str,
    required_suffixes: &[&str],
) -> Option<String> {
    let links = list_ipfs_links(state, &format!("/ipfs/{}", cid.trim())).await.ok()?;
    if links.is_empty() {
        return None;
    }

    let mut names = links
        .iter()
        .filter_map(|link| link.get("Name").and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .filter(|name| {
            required_suffixes.is_empty()
                || required_suffixes.iter().any(|suffix| name.ends_with(suffix))
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    names.dedup();
    (names.len() == 1).then(|| names.remove(0))
}

pub async fn list_ipfs_links(
    state: &AppState,
    ipfs_path: &str,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let endpoint = format!("{}/api/v0/ls", state.ipfs_api_url.trim_end_matches('/'));

    let mut request = state.http.post(endpoint).query(&[("arg", ipfs_path)]);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request.send().await?;
    let status = response.status();
    let payload = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("Unable to list IPFS path {ipfs_path}: {payload}"));
    }

    let json = serde_json::from_str::<serde_json::Value>(&payload)?;
    let links = json
        .get("Objects")
        .and_then(|value| value.as_array())
        .and_then(|objects| objects.first())
        .and_then(|object| object.get("Links"))
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(links)
}

pub async fn download_ipfs_file(
    state: &AppState,
    ipfs_path: &str,
    destination_file: &Path,
) -> anyhow::Result<()> {
    if let Some(parent) = destination_file.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Unable to create file directory {}", parent.display()))?;
    }

    let endpoint = format!("{}/api/v0/cat", state.ipfs_api_url.trim_end_matches('/'));

    let mut request = state.http.post(endpoint).query(&[("arg", ipfs_path)]);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request.send().await?;
    let status = response.status();

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Unable to download IPFS file {ipfs_path}: {body}"));
    }

    let bytes = response.bytes().await?;
    fs::write(destination_file, &bytes).await.with_context(|| {
        format!("Unable to write synced IPFS file to {}", destination_file.display())
    })?;

    Ok(())
}

pub async fn download_ipfs_leaf(
    state: &AppState,
    ipfs_path: &str,
    destination_dir: &Path,
) -> anyhow::Result<()> {
    let endpoint = format!("{}/api/v0/cat", state.ipfs_api_url.trim_end_matches('/'));
    let mut request = state.http.post(endpoint).query(&[("arg", ipfs_path)]);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request.send().await?;
    let status = response.status();

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Unable to download IPFS file {ipfs_path}: {body}"));
    }

    let bytes = response.bytes().await?;
    let file_name = super::sync::resolve_sync_leaf_file_name(state, ipfs_path, &bytes).await;
    let target_file = destination_dir.join(file_name);

    if let Some(parent) = target_file.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Unable to create file directory {}", parent.display()))?;
    }

    fs::write(&target_file, &bytes).await.with_context(|| {
        format!("Unable to write synced IPFS file to {}", target_file.display())
    })?;

    Ok(())
}

#[async_recursion]
pub async fn download_ipfs_path_recursive(
    state: &AppState,
    ipfs_path: &str,
    destination_dir: &Path,
) -> anyhow::Result<()> {
    let Ok(links) = list_ipfs_links(state, ipfs_path).await else {
        download_ipfs_leaf(state, ipfs_path, destination_dir).await?;
        return Ok(());
    };

    if links.is_empty() {
        download_ipfs_leaf(state, ipfs_path, destination_dir).await?;
        return Ok(());
    }

    fs::create_dir_all(destination_dir).await.with_context(|| {
        format!("Unable to create destination directory {}", destination_dir.display())
    })?;

    for link in links {
        let name = link.get("Name").and_then(|value| value.as_str()).unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }

        let Some(safe_name) = sanitize_file_name(name) else {
            tracing::warn!("skipping IPFS entry with unsafe name: {}", name);
            continue;
        };
        let child_destination = destination_dir.join(&safe_name);
        if !child_destination.starts_with(destination_dir) {
            tracing::warn!("refusing to write outside sync dir: {}", child_destination.display());
            continue;
        }
        let child_ipfs_path = format!("{}/{}", ipfs_path.trim_end_matches('/'), name);
        let link_type = link.get("Type").and_then(serde_json::Value::as_i64).unwrap_or(0);

        if matches!(link_type, 1 | 5) {
            download_ipfs_path_recursive(state, &child_ipfs_path, &child_destination).await?;
        } else {
            download_ipfs_file(state, &child_ipfs_path, &child_destination).await?;
        }
    }

    Ok(())
}

pub async fn is_cid_pinned(state: &AppState, cid: &str) -> anyhow::Result<bool> {
    let endpoint =
        format!("{}/api/v0/pin/ls?arg={}", state.ipfs_api_url.trim_end_matches('/'), cid.trim());

    let mut request = state.http.post(endpoint);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request.send().await?;
    if response.status().is_success() {
        return Ok(true);
    }

    let body = response.text().await.unwrap_or_default();
    if body.to_lowercase().contains("not pinned") {
        return Ok(false);
    }

    Err(anyhow!("Unable to verify pin status for {cid}: {body}"))
}

pub async fn pin_single_cid(
    state: &AppState,
    cid: &str,
    label: Option<String>,
) -> Result<PinCidResult, AppError> {
    let trimmed = cid.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("CID is required"));
    }

    let endpoint =
        format!("{}/api/v0/pin/add?arg={}", state.ipfs_api_url.trim_end_matches('/'), trimmed);

    let mut request = state.http.post(endpoint);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request
        .send()
        .await
        .map_err(|error| AppError::internal(anyhow!("Failed to reach IPFS API: {error}")))?;

    let status = response.status();

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::internal(anyhow!("IPFS pin failed with status {status}: {body}")));
    }

    let payload = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| AppError::internal(anyhow!("Unable to decode IPFS response: {error}")))?;

    let pin_reference = payload
        .get("Pinned")
        .and_then(|value| value.as_str())
        .or_else(|| {
            payload
                .get("Pins")
                .and_then(|value| value.as_array())
                .and_then(|pins| pins.first())
                .and_then(|value| value.as_str())
        })
        .unwrap_or(trimmed)
        .to_string();

    Ok(PinCidResult {
        cid: trimmed.to_string(),
        label,
        pinned: true,
        provider: "kubo",
        pin_reference,
        requested_at: Utc::now(),
    })
}

pub async fn fetch_kubo_repo_stat(state: &AppState) -> anyhow::Result<KuboRepoStat> {
    let endpoint = format!("{}/api/v0/repo/stat", state.ipfs_api_url.trim_end_matches('/'));
    let mut request = state.http.post(endpoint).timeout(Duration::from_secs(8));
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }
    let response = request.send().await?;
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Unable to read IPFS repo/stat: {body}"));
    }
    Ok(response.json::<KuboRepoStat>().await?)
}
