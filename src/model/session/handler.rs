//! HTTP handlers for session management.
#![allow(clippy::pedantic, clippy::nursery)]

use axum::{
    Json,
    extract::{Path as AxumPath, State},
};
use chrono::Utc;
use uuid::Uuid;

use crate::{
    AppError, AppState,
    model::session::types::{
        BridgeSession, ConnectSessionRequest, ConnectSessionResponse, DisconnectSessionRequest,
        DisconnectSessionResponse, SessionSummary,
    },
};

pub async fn connect_session(
    State(state): State<AppState>,
    Json(input): Json<ConnectSessionRequest>,
) -> Result<Json<ConnectSessionResponse>, AppError> {
    if input.website_origin.trim().is_empty() {
        return Err(AppError::bad_request("website_origin is required"));
    }

    let session = BridgeSession {
        session_id: Uuid::new_v4().to_string(),
        session_secret: Uuid::new_v4().to_string(),
        website_origin: input.website_origin.trim().to_string(),
        account_address: input.account_address.filter(|value| !value.trim().is_empty()),
        profile_username: input.profile_username.filter(|value| !value.trim().is_empty()),
        client_name: input.client_name.filter(|value| !value.trim().is_empty()),
        connected_at: Utc::now(),
    };

    let mut sessions = state.sessions.write().await;
    sessions.insert(session.session_secret.clone(), session.clone());

    Ok(Json(ConnectSessionResponse {
        session,
        message: "Session connected. The website can now hand work or profile share requests to the local bridge.",
    }))
}

pub async fn disconnect_session(
    State(state): State<AppState>,
    Json(input): Json<DisconnectSessionRequest>,
) -> Result<Json<DisconnectSessionResponse>, AppError> {
    let mut sessions = state.sessions.write().await;
    let removed = sessions.remove(&input.session_secret).is_some();

    Ok(Json(DisconnectSessionResponse { disconnected: removed }))
}

pub async fn list_sessions(
    State(state): State<AppState>,
) -> Result<Json<Vec<SessionSummary>>, AppError> {
    let sessions = state.sessions.read().await;
    let data = sessions
        .values()
        .map(|session| SessionSummary {
            session_id: session.session_id.clone(),
            website_origin: session.website_origin.clone(),
            account_address: session.account_address.clone(),
            profile_username: session.profile_username.clone(),
            client_name: session.client_name.clone(),
            connected_at: session.connected_at,
        })
        .collect();

    Ok(Json(data))
}

pub async fn session_by_id(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<SessionSummary>, AppError> {
    let sessions = state.sessions.read().await;
    let session = sessions
        .values()
        .find(|session| session.session_id == session_id)
        .ok_or_else(|| AppError::bad_request("Session was not found"))?;

    Ok(Json(SessionSummary {
        session_id: session.session_id.clone(),
        website_origin: session.website_origin.clone(),
        account_address: session.account_address.clone(),
        profile_username: session.profile_username.clone(),
        client_name: session.client_name.clone(),
        connected_at: session.connected_at,
    }))
}
