//! Session DTOs — request/response bodies for the `/session/*` handlers and
//! the persisted [`BridgeSession`] record kept in `AppState::sessions`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeSession {
    pub session_id: String,
    pub session_secret: String,
    pub website_origin: String,
    pub account_address: Option<String>,
    pub profile_username: Option<String>,
    pub client_name: Option<String>,
    pub connected_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ConnectSessionRequest {
    pub website_origin: String,
    pub account_address: Option<String>,
    pub profile_username: Option<String>,
    pub client_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConnectSessionResponse {
    pub session: BridgeSession,
    pub message: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct DisconnectSessionRequest {
    pub session_secret: String,
}

#[derive(Debug, Serialize)]
pub struct DisconnectSessionResponse {
    pub disconnected: bool,
}

#[derive(Debug, Serialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub website_origin: String,
    pub account_address: Option<String>,
    pub profile_username: Option<String>,
    pub client_name: Option<String>,
    pub connected_at: DateTime<Utc>,
}
