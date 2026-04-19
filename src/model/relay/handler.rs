//! Relay HTTP handlers: link/unlink pairing flows plus share-work /
//! share-profile JSON endpoints. The matching HTML handlers for the
//! share-work flow live alongside the other page renderers in `html/handler`.

use axum::{Form, Json, extract::State, response::Redirect};

use crate::{
    AppError, AppState,
    model::relay::{
        RelayLinkFormRequest, RelayLinkRequest, RelayLinkResponse, RelayUnlinkResponse,
        ShareProfileRequest, ShareProfileResponse, ShareWorkRequest, ShareWorkResponse,
        service::{
            perform_relay_link, perform_relay_unlink, share_profile_inner, share_work_inner,
        },
    },
    util::url::encode_query_component,
};

pub async fn link_relay_device(
    State(state): State<AppState>,
    Json(input): Json<RelayLinkRequest>,
) -> Result<Json<RelayLinkResponse>, AppError> {
    let payload = perform_relay_link(&state, input).await?;
    Ok(Json(payload))
}

pub async fn link_relay_device_form(
    State(state): State<AppState>,
    Form(input): Form<RelayLinkFormRequest>,
) -> Result<Redirect, AppError> {
    let redirect_relay_server_url = input.relay_server_url.clone();
    let redirect_pairing_code = input.pairing_code.clone();
    let redirect_device_name = input.device_name.clone();

    match perform_relay_link(
        &state,
        RelayLinkRequest {
            relay_server_url: Some(input.relay_server_url),
            pairing_code: input.pairing_code,
            device_name: input.device_name,
        },
    )
    .await
    {
        Ok(_) => Ok(Redirect::to("/?linked=1")),
        Err(error) => Ok(Redirect::to(&format!(
            "/?error={}&relay_server_url={}&pairing_code={}&device_name={}",
            encode_query_component(&error.message),
            encode_query_component(&redirect_relay_server_url),
            encode_query_component(&redirect_pairing_code),
            encode_query_component(redirect_device_name.as_deref().unwrap_or("")),
        ))),
    }
}

pub async fn unlink_relay_device(
    State(state): State<AppState>,
) -> Result<Json<RelayUnlinkResponse>, AppError> {
    perform_relay_unlink(&state, true).await.map_err(AppError::internal)?;

    Ok(Json(RelayUnlinkResponse { unlinked: true }))
}

pub async fn unlink_relay_device_form(State(state): State<AppState>) -> Result<Redirect, AppError> {
    perform_relay_unlink(&state, true).await.map_err(AppError::internal)?;

    Ok(Redirect::to("/?unlinked=1"))
}

pub async fn share_work(
    State(state): State<AppState>,
    Json(input): Json<ShareWorkRequest>,
) -> Result<Json<ShareWorkResponse>, AppError> {
    let response = share_work_inner(&state, input).await?;
    Ok(Json(response))
}

pub async fn share_profile(
    State(state): State<AppState>,
    Json(input): Json<ShareProfileRequest>,
) -> Result<Json<ShareProfileResponse>, AppError> {
    let response = share_profile_inner(&state, input).await?;
    Ok(Json(response))
}
