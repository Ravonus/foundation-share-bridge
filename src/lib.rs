//! Foundation Share Bridge — library entry point and crate-core primitives.
//!
//! This crate powers a per-user desktop IPFS pinning companion for the
//! Foundation Archive. The binary in `src/main.rs` is a thin shell that
//! initialises logging and delegates to [`run`].
//!
//! The three ubiquitous types — [`AppState`], [`AppError`], [`OperationStatus`]
//! — live here at the crate root so every other module has a single place to
//! look. `AppState` must stay `Clone`; a compile-time assertion guards it.
//!
//! ## Refactor status
//!
//! The codebase is mid-migration from a single 9k-line `main.rs` into focused
//! per-domain modules. During the transition, [`inline`] holds everything that
//! hasn't yet been split out. It is removed entirely in the final stage.

#![forbid(unsafe_code)]

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Serialize;
use tokio::sync::RwLock;

pub(crate) mod html;
pub(crate) mod inline;
pub(crate) mod model;
pub(crate) mod util;

pub use inline::run;

use crate::model::{
    config::{BridgeConfig, BridgePersistentState},
    session::BridgeSession,
};

/// Shared, cheaply-cloneable handle to every mutable and immutable piece of
/// bridge state. All async handlers receive this via `axum::extract::State`.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) http: Client,
    pub(crate) ipfs_api_url: String,
    pub ipfs_api_auth_header: Option<String>,
    pub state_file: PathBuf,
    pub config_file: PathBuf,
    pub repair_interval_seconds: u64,
    pub sessions: Arc<RwLock<HashMap<String, BridgeSession>>>,
    pub persistent: Arc<RwLock<BridgePersistentState>>,
    pub config: Arc<RwLock<BridgeConfig>>,
    pub operation: Arc<RwLock<OperationStatus>>,
}

// Compile-time invariant: AppState must remain Clone.
// Background loops spawn tokio tasks with `state.clone()`; losing Clone silently
// breaks them at spawn time.
const _: fn() = || {
    const fn assert_clone<T: Clone>() {}
    assert_clone::<AppState>();
};

/// Progress indicator for long-running operations (repair cycle, sync, etc.).
/// Exposed via `GET /status/live` for the dashboard's live progress bar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OperationStatus {
    pub phase: String,
    pub detail: Option<String>,
    pub progress_current: Option<usize>,
    pub progress_total: Option<usize>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl OperationStatus {
    pub fn idle() -> Self {
        let now = Utc::now();
        Self {
            phase: "idle".to_string(),
            detail: None,
            progress_current: None,
            progress_total: None,
            started_at: now,
            updated_at: now,
        }
    }

    pub fn busy(phase: &str, detail: Option<String>, total: Option<usize>) -> Self {
        let now = Utc::now();
        Self {
            phase: phase.to_string(),
            detail,
            progress_current: Some(0),
            progress_total: total,
            started_at: now,
            updated_at: now,
        }
    }
}

/// Crate-wide HTTP error type. Every handler returns `Result<T, AppError>`.
#[derive(Debug)]
pub(crate) struct AppError {
    pub status: StatusCode,
    pub message: String,
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, message: message.into() }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self { status: StatusCode::UNAUTHORIZED, message: message.into() }
    }

    // `anyhow::Error` is taken by value so callers can write
    // `.map_err(AppError::internal)?` without borrowing.
    #[allow(clippy::needless_pass_by_value)]
    pub fn internal(error: anyhow::Error) -> Self {
        Self { status: StatusCode::INTERNAL_SERVER_ERROR, message: error.to_string() }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({ "error": self.message }))).into_response()
    }
}
