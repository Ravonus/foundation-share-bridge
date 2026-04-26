//! Walk the archive site's artist browse API and pin every Foundation root
//! we find. Runs in a background task kicked off by the HTTP handler so the
//! UI can return immediately.

use std::time::Duration;

use anyhow::{Context, anyhow};
use serde::Deserialize;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::{
    AppError, AppState, OperationStatus,
    model::{pin::service::pin_work_payload, relay::types::RelayShareWorkPayload},
    util::url::trim_trailing_slash,
};

const PAGE_FETCH_RETRY_DELAY: Duration = Duration::from_secs(2);
const PAGE_FETCH_MAX_RETRIES: u32 = 3;
const CONSECUTIVE_FAILURE_LIMIT: u32 = 5;

#[derive(Debug, Deserialize)]
struct BrowseResponse {
    items: Vec<BrowseItem>,
    #[serde(rename = "foundationPage")]
    foundation_page: Option<u32>,
    #[serde(rename = "foundationExhausted")]
    foundation_exhausted: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct BrowseItem {
    title: Option<String>,
    contract_address: Option<String>,
    token_id: Option<String>,
    foundation_url: Option<String>,
    metadata_cid: Option<String>,
    media_cid: Option<String>,
    artist_username: Option<String>,
}

impl BrowseItem {
    fn into_payload(self) -> Option<RelayShareWorkPayload> {
        let contract_address = self.contract_address?.trim().to_string();
        let token_id = self.token_id?.trim().to_string();
        if contract_address.is_empty() || token_id.is_empty() {
            return None;
        }
        if self.metadata_cid.is_none() && self.media_cid.is_none() {
            return None;
        }
        Some(RelayShareWorkPayload {
            title: self.title.unwrap_or_else(|| "Untitled work".to_string()),
            contract_address,
            token_id,
            foundation_url: self.foundation_url,
            metadata_cid: self.metadata_cid,
            media_cid: self.media_cid,
            artist_username: self.artist_username,
        })
    }
}

pub fn sanitize_username(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_start_matches('@');
    if trimmed.is_empty() {
        return Err("Artist handle is required.".to_string());
    }
    if !trimmed.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
        return Err(
            "Invalid artist handle. Use the Foundation username (letters, numbers, -, _, .)."
                .to_string(),
        );
    }
    Ok(trimmed.to_string())
}

async fn fetch_page(
    state: &AppState,
    archive_base: &str,
    username: &str,
    page: u32,
) -> anyhow::Result<BrowseResponse> {
    let url = format!(
        "{base}/api/profile/{user}/browse?mode=foundation&page={page}",
        base = trim_trailing_slash(archive_base),
        user = urlencoding_encode(username),
    );

    let mut last_error: Option<anyhow::Error> = None;
    for attempt in 0..PAGE_FETCH_MAX_RETRIES {
        match state.http.get(&url).send().await {
            Ok(response) if response.status().is_success() => {
                return response
                    .json::<BrowseResponse>()
                    .await
                    .with_context(|| format!("Malformed browse payload from {url}"));
            }
            Ok(response) => {
                last_error = Some(anyhow!(
                    "Archive browse returned HTTP {} on attempt {}",
                    response.status(),
                    attempt + 1
                ));
            }
            Err(error) => {
                last_error = Some(error.into());
            }
        }
        sleep(PAGE_FETCH_RETRY_DELAY).await;
    }

    Err(last_error.unwrap_or_else(|| anyhow!("Archive browse failed with no details")))
}

fn urlencoding_encode(value: &str) -> String {
    // Keep it dependency-free; the characters that survive a Foundation
    // handle validation are the same ones that are safe in a URL path.
    value
        .chars()
        .map(|c| match c {
            c if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') => c.to_string(),
            c => format!("%{:02X}", c as u32),
        })
        .collect()
}

struct Progress {
    phase: String,
    detail: String,
    current: usize,
    total: Option<usize>,
}

async fn set_progress(state: &AppState, update: Progress) {
    let mut op = state.operation.write().await;
    *op = OperationStatus::busy(&update.phase, Some(update.detail), update.total);
    op.progress_current = Some(update.current);
}

async fn clear_progress(state: &AppState) {
    *state.operation.write().await = OperationStatus::idle();
}

struct ArchiveState {
    phase: String,
    username: String,
    page: u32,
    seen: usize,
    pinned: usize,
    consecutive_failures: u32,
}

impl ArchiveState {
    fn new(username: String) -> Self {
        Self {
            phase: format!("archive-all:{username}"),
            username,
            page: 0,
            seen: 0,
            pinned: 0,
            consecutive_failures: 0,
        }
    }

    fn detail(&self) -> String {
        format!(
            "@{user} · {pinned}/{seen} pinned · page {page}",
            user = self.username,
            pinned = self.pinned,
            seen = self.seen,
            page = self.page + 1,
        )
    }
}

async fn publish(state: &AppState, archive: &ArchiveState) {
    set_progress(
        state,
        Progress {
            phase: archive.phase.clone(),
            detail: archive.detail(),
            current: archive.pinned,
            total: None,
        },
    )
    .await;
}

fn record_failure(archive: &mut ArchiveState, payload: &RelayShareWorkPayload, error: &AppError) {
    archive.consecutive_failures += 1;
    warn!(
        "archive-all @{user} failed to pin {contract}/{token}: {error:?}",
        user = archive.username,
        contract = payload.contract_address,
        token = payload.token_id,
    );
}

fn should_abort(archive: &ArchiveState) -> bool {
    if archive.consecutive_failures >= CONSECUTIVE_FAILURE_LIMIT {
        warn!(
            "archive-all @{user} bailing after {limit} consecutive pin failures",
            user = archive.username,
            limit = CONSECUTIVE_FAILURE_LIMIT,
        );
        return true;
    }
    false
}

/// Pin one item; returns `Some(())` to continue, `None` to abort (too many
/// consecutive failures).
async fn pin_one_item(
    state: &AppState,
    archive: &mut ArchiveState,
    payload: RelayShareWorkPayload,
) -> Option<()> {
    archive.seen += 1;
    publish(state, archive).await;

    match pin_work_payload(state, payload.clone()).await {
        Ok(_pins) => {
            archive.pinned += 1;
            archive.consecutive_failures = 0;
            Some(())
        }
        Err(error) => {
            record_failure(archive, &payload, &error);
            if should_abort(archive) { None } else { Some(()) }
        }
    }
}

/// Returns `true` to keep paging, `false` to stop (exhausted or no progress).
async fn consume_page(
    state: &AppState,
    archive: &mut ArchiveState,
    response: BrowseResponse,
) -> bool {
    for item in response.items {
        let Some(payload) = item.into_payload() else {
            continue;
        };
        if pin_one_item(state, archive, payload).await.is_none() {
            return false;
        }
    }

    let next_page = response.foundation_page.unwrap_or(archive.page + 1);
    if response.foundation_exhausted || next_page == archive.page {
        return false;
    }
    archive.page = next_page;
    true
}

async fn announce_start(state: &AppState, archive: &ArchiveState) {
    set_progress(
        state,
        Progress {
            phase: archive.phase.clone(),
            detail: format!("Looking up @{}…", archive.username),
            current: 0,
            total: None,
        },
    )
    .await;
}

async fn walk_pages(state: &AppState, archive_base: &str, archive: &mut ArchiveState) {
    loop {
        let fetched = fetch_page(state, archive_base, &archive.username, archive.page).await;
        let response = match fetched {
            Ok(payload) => payload,
            Err(error) => {
                warn!(
                    "archive-all @{user} page {page} failed: {error:#}",
                    user = archive.username,
                    page = archive.page,
                );
                return;
            }
        };
        if !consume_page(state, archive, response).await {
            return;
        }
    }
}

/// Walk every Foundation page for `username`, pinning each work as it comes
/// back. Safe to call from a detached `tokio::spawn` — errors are logged but
/// do not propagate (UI sees the eventual idle operation status).
pub async fn archive_artist_collection(state: AppState, archive_base: String, username: String) {
    let mut archive = ArchiveState::new(username);
    announce_start(&state, &archive).await;
    walk_pages(&state, &archive_base, &mut archive).await;
    info!(
        "archive-all complete for @{user}: {pinned} pinned of {seen} seen",
        user = archive.username,
        pinned = archive.pinned,
        seen = archive.seen,
    );
    clear_progress(&state).await;
}
