//! Crate-core primitives: shared `AppState`, `OperationStatus`, and `AppError`.
//!
//! These three types are ubiquitous — every handler imports `AppState` +
//! `AppError`; background loops and render services also touch
//! `OperationStatus`. They live together at the crate root so every other
//! module has a single place to look.
//!
//! `AppState` must stay `Clone` — `tokio::spawn(state.clone())` is the
//! pattern used by both background loops. A compile-time assertion guards it.

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

use crate::inline::{BridgeConfig, BridgePersistentState, BridgeSession};

/// Shared, cheaply-cloneable handle to every mutable and immutable piece of
/// bridge state. All async handlers receive this via `axum::extract::State`.
#[derive(Clone)]
pub struct AppState {
    pub http: Client,
    pub ipfs_api_url: String,
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
pub struct OperationStatus {
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
pub struct AppError {
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
