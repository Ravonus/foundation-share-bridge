//! HTTP handlers for the bulk archive-artist-catalog flow.
//!
//! The JSON endpoint is designed for programmatic callers (scripts, other
//! tools on the same machine). The form endpoint lets the bridge's own
//! HTML root page kick one off with a plain `<form>` submit and redirect
//! back so the live-status panel picks up the progress.

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    response::Redirect,
};
use serde::Serialize;

use crate::{
    AppError, AppState,
    model::catalog::service::{archive_artist_collection, sanitize_username},
    util::url::encode_query_component,
};

#[derive(Debug, Serialize)]
pub struct ArchiveAllResponse {
    pub username: String,
    pub started: bool,
    pub message: &'static str,
}

fn archive_base_url(config_relay_server_url: &str) -> String {
    let trimmed = config_relay_server_url.trim();
    if trimmed.is_empty() {
        "https://foundation.agorix.io".to_string()
    } else {
        trimmed.to_string()
    }
}

fn spawn_archive_job(state: AppState, username: String) {
    tokio::spawn(async move {
        let archive_base = {
            let config = state.config.read().await;
            archive_base_url(&config.relay_server_url)
        };
        archive_artist_collection(state, archive_base, username).await;
    });
}

/// JSON endpoint — returns `202`-style acknowledgement immediately; the
/// actual walk happens in a detached task.
pub async fn archive_all_for_artist(
    AxumPath(username): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<ArchiveAllResponse>, AppError> {
    let sanitized = sanitize_username(&username).map_err(AppError::bad_request)?;
    spawn_archive_job(state, sanitized.clone());

    Ok(Json(ArchiveAllResponse {
        username: sanitized,
        started: true,
        message: "Archive-all started. Live status panel will show progress.",
    }))
}

/// Form variant — kicked off from the bridge's own HTML root page; redirects
/// back to `/` with a flash flag so the user sees the operation kick in.
#[derive(Debug, serde::Deserialize)]
pub struct ArchiveAllFormInput {
    pub username: String,
}

pub async fn archive_all_for_artist_form(
    State(state): State<AppState>,
    axum::Form(input): axum::Form<ArchiveAllFormInput>,
) -> Result<Redirect, AppError> {
    let sanitized = match sanitize_username(&input.username) {
        Ok(value) => value,
        Err(message) => {
            return Ok(Redirect::to(&format!("/?error={}", encode_query_component(&message))));
        }
    };

    spawn_archive_job(state, sanitized.clone());

    Ok(Redirect::to(&format!("/?archiving={}", encode_query_component(&sanitized))))
}
