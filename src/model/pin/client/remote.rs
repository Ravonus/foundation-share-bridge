//! Remote pinning service submission — post a CID to the IPFS Pinning Service
//! API endpoint the user configured (Pinata, Filebase, self-hosted, etc.).
//!
//! Returns `Ok(None)` when remote pinning is disabled so callers can treat the
//! feature as a no-op cleanly.

use std::time::Duration;

use anyhow::{Context, anyhow};

use crate::{AppState, util::url::trim_trailing_slash};

pub async fn submit_to_remote_pinning_service(
    state: &AppState,
    cid: &str,
    name_hint: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let (enabled, service_name, service_url, token) = {
        let config = state.config.read().await;
        (
            config.remote_pinning_enabled,
            config.remote_pinning_service_name.clone(),
            config.remote_pinning_service_url.clone(),
            config.remote_pinning_access_token.clone(),
        )
    };
    if !enabled {
        return Ok(None);
    }
    let service_url = service_url
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("Remote pinning is enabled but service URL is empty"))?;
    let parsed = url::Url::parse(&service_url)
        .with_context(|| format!("Invalid remote_pinning_service_url: {service_url}"))?;
    if parsed.scheme() != "https" {
        anyhow::bail!("remote_pinning_service_url must use https (got {})", parsed.scheme());
    }
    let token = token
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("Remote pinning is enabled but access token is empty"))?;
    let endpoint = format!("{}/pins", trim_trailing_slash(&service_url));
    let mut body = serde_json::json!({"cid": cid.trim()});
    if let Some(name) = name_hint.map(str::trim).filter(|value| !value.is_empty()) {
        body["name"] = serde_json::Value::String(name.to_string());
    }
    let response = state
        .http
        .post(endpoint)
        .bearer_auth(token.trim())
        .json(&body)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .context("Unable to reach remote pinning service")?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "Remote pin failed ({status}): {}",
            text.chars().take(300).collect::<String>()
        ));
    }
    let _ = response.bytes().await;
    Ok(Some(service_name.unwrap_or_else(|| "remote".to_string())))
}
