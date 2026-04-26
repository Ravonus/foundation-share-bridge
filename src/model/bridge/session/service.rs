//! Session service layer — authentication + lifecycle helpers that operate on
//! the `AppState::sessions` map.

use crate::{AppError, AppState};

pub async fn validate_session(state: &AppState, session_secret: &str) -> Result<(), AppError> {
    let exists = state.sessions.read().await.contains_key(session_secret);
    if exists {
        return Ok(());
    }

    Err(AppError::unauthorized(
        "Unknown session_secret. Connect the website before sending share or pin requests.",
    ))
}
