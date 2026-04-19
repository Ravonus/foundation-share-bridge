//! Config HTTP handlers.

use crate::{
    AppError, AppState,
    model::{
        config::{
            BridgeConfigResponse, UpdateBridgeConfigFormRequest, UpdateBridgeConfigRequest,
            service::apply_config_update,
        },
        system::service::build_config_response,
    },
    util::url::encode_query_component,
};

use axum::{Form, Json, extract::State, response::Redirect};

pub async fn get_config(
    State(state): State<AppState>,
) -> Result<Json<BridgeConfigResponse>, AppError> {
    let config = state.config.read().await;
    Ok(Json(build_config_response(&state, &config)))
}

pub async fn update_config(
    State(state): State<AppState>,
    Json(input): Json<UpdateBridgeConfigRequest>,
) -> Result<Json<BridgeConfigResponse>, AppError> {
    let updated = apply_config_update(&state, input).await?;
    Ok(Json(updated))
}

pub async fn update_config_form(
    State(state): State<AppState>,
    Form(input): Form<UpdateBridgeConfigFormRequest>,
) -> Result<Redirect, AppError> {
    let quota = input.storage_quota_gb.as_deref().map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            trimmed.parse::<f64>().ok().filter(|value| *value > 0.0)
        }
    });

    let retries = input.max_retry_attempts.as_deref().map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() { None } else { trimmed.parse::<u32>().ok() }
    });

    let name = input.remote_pinning_service_name.as_deref().map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
    });

    let url = input.remote_pinning_service_url.as_deref().map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
    });

    let token = input.remote_pinning_access_token.as_deref().map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
    });

    let request = UpdateBridgeConfigRequest {
        download_root_dir: Some(input.download_root_dir),
        sync_enabled: Some(input.sync_enabled.is_some()),
        local_gateway_base_url: Some(input.local_gateway_base_url),
        public_gateway_base_url: Some(input.public_gateway_base_url),
        relay_enabled: Some(input.relay_enabled.is_some()),
        relay_server_url: Some(input.relay_server_url),
        relay_device_name: Some(input.relay_device_name),
        tunnel_enabled: Some(input.tunnel_enabled.is_some()),
        storage_quota_gb: quota,
        max_retry_attempts: retries,
        remote_pinning_enabled: Some(input.remote_pinning_enabled.is_some()),
        remote_pinning_service_name: name,
        remote_pinning_service_url: url,
        remote_pinning_access_token: token,
    };

    match apply_config_update(&state, request).await {
        Ok(_) => Ok(Redirect::to("/settings?saved=1")),
        Err(error) => {
            Ok(Redirect::to(&format!("/settings?error={}", encode_query_component(&error.message))))
        }
    }
}
