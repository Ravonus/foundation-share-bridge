//! HTTP handlers for session management.
#![allow(clippy::pedantic, clippy::nursery)]

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    response::Redirect,
};
use chrono::Utc;
use uuid::Uuid;

use crate::{
    AppError, AppState,
    model::{
        config::service::persist_bridge_state,
        session::types::{
            BridgeSession, ConnectSessionRequest, ConnectSessionResponse, DisconnectSessionRequest,
            DisconnectSessionResponse, SessionSummary,
        },
    },
    util::machine::deterministic_session_id,
};

pub async fn connect_session(
    State(state): State<AppState>,
    Json(input): Json<ConnectSessionRequest>,
) -> Result<Json<ConnectSessionResponse>, AppError> {
    let website_origin = input.website_origin.trim();
    if website_origin.is_empty() {
        return Err(AppError::bad_request("website_origin is required"));
    }
    let website_origin = website_origin.to_string();
    let deterministic_id = deterministic_session_id(&website_origin);

    let (session, is_new) = upsert_session(
        &state,
        UpsertSessionInput {
            website_origin,
            session_id: deterministic_id,
            account_address: input.account_address.filter(|value| !value.trim().is_empty()),
            profile_username: input.profile_username.filter(|value| !value.trim().is_empty()),
            client_name: input.client_name.filter(|value| !value.trim().is_empty()),
        },
    )
    .await?;

    let message = if is_new {
        "Session connected. The website can now hand work or profile share requests to the local bridge."
    } else {
        "Welcome back — reused the existing session for this machine."
    };

    Ok(Json(ConnectSessionResponse { session, message }))
}

struct UpsertSessionInput {
    website_origin: String,
    session_id: String,
    account_address: Option<String>,
    profile_username: Option<String>,
    client_name: Option<String>,
}

async fn upsert_session(
    state: &AppState,
    input: UpsertSessionInput,
) -> Result<(BridgeSession, bool), AppError> {
    let now = Utc::now();
    let (session, is_new) = {
        let mut sessions = state.sessions.write().await;
        let existing =
            sessions.values().find(|candidate| candidate.session_id == input.session_id).cloned();

        if let Some(mut existing) = existing {
            // Refresh the labels the site may have updated (account / profile)
            // but keep the stable session_secret so cached clients stay valid.
            existing.website_origin = input.website_origin;
            existing.account_address = input.account_address.or(existing.account_address);
            existing.profile_username = input.profile_username.or(existing.profile_username);
            existing.client_name = input.client_name.or(existing.client_name);
            existing.connected_at = now;
            sessions.insert(existing.session_secret.clone(), existing.clone());
            (existing, false)
        } else {
            let session = BridgeSession {
                session_id: input.session_id,
                session_secret: Uuid::new_v4().to_string(),
                website_origin: input.website_origin,
                account_address: input.account_address,
                profile_username: input.profile_username,
                client_name: input.client_name,
                connected_at: now,
            };
            sessions.insert(session.session_secret.clone(), session.clone());
            (session, true)
        }
    };

    sync_sessions_to_persistent(state).await?;
    Ok((session, is_new))
}

/// Mirror the in-memory session map into `BridgePersistentState` and flush it
/// to disk so sessions survive a bridge restart.
async fn sync_sessions_to_persistent(state: &AppState) -> Result<(), AppError> {
    let snapshot = state.sessions.read().await.clone();
    {
        let mut persistent = state.persistent.write().await;
        persistent.sessions = snapshot;
        persistent.updated_at = Some(Utc::now());
    }
    persist_bridge_state(state).await.map_err(AppError::internal)?;
    Ok(())
}

pub async fn disconnect_session(
    State(state): State<AppState>,
    Json(input): Json<DisconnectSessionRequest>,
) -> Result<Json<DisconnectSessionResponse>, AppError> {
    let removed = {
        let mut sessions = state.sessions.write().await;
        sessions.remove(&input.session_secret).is_some()
    };

    if removed {
        sync_sessions_to_persistent(&state).await?;
    }

    Ok(Json(DisconnectSessionResponse { disconnected: removed }))
}

/// Form-based counterpart to [`disconnect_session_by_id`]. Lets the bridge's
/// own HTML pages prune stale sessions without shipping JS for a fetch call.
pub async fn disconnect_session_by_id_form(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Redirect, AppError> {
    remove_session_by_id(&state, &session_id).await?;
    Ok(Redirect::to("/"))
}

async fn remove_session_by_id(state: &AppState, session_id: &str) -> Result<bool, AppError> {
    let secret = {
        let sessions = state.sessions.read().await;
        sessions
            .values()
            .find(|candidate| candidate.session_id == session_id)
            .map(|session| session.session_secret.clone())
    };

    let Some(secret) = secret else {
        return Ok(false);
    };

    let removed = {
        let mut sessions = state.sessions.write().await;
        sessions.remove(&secret).is_some()
    };

    if removed {
        sync_sessions_to_persistent(state).await?;
    }

    Ok(removed)
}

/// Disconnect a session identified by its public `session_id`. Lets the UI
/// prune stale sessions without needing to hold the secret.
pub async fn disconnect_session_by_id(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<DisconnectSessionResponse>, AppError> {
    let removed = remove_session_by_id(&state, &session_id).await?;
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
