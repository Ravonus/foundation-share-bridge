//! Config service layer — load, persist, and mutate the bridge config +
//! state files. `apply_config_update` is the single entry point that all
//! `/config` handlers funnel through so validation + persistence stay in one
//! place.

use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::Context;
use chrono::Utc;
use tokio::{fs, io::AsyncWriteExt};

use super::types::{
    BridgeConfig, BridgeConfigResponse, BridgePersistentState, UpdateBridgeConfigRequest,
    bridge_config_uses_yaml, default_bridge_config, legacy_bridge_json_path, parse_bridge_config,
};
use crate::{AppError, AppState};

/// Write a file with 0o600 permissions on Unix so config + state can't be
/// read by other local users. On Windows the default ACLs apply.
async fn write_private_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .await
        .with_context(|| format!("Unable to open {} for write", path.display()))?;
    file.write_all(bytes).await.with_context(|| format!("Unable to write {}", path.display()))?;
    file.flush().await.ok();
    Ok(())
}

pub async fn load_persistent_state(path: &Path) -> anyhow::Result<BridgePersistentState> {
    match fs::read_to_string(path).await {
        Ok(contents) => {
            serde_json::from_str::<BridgePersistentState>(&contents).with_context(|| {
                format!("Unable to parse persistent bridge state from {}", path.display())
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(BridgePersistentState::default())
        }
        Err(error) => Err(error).with_context(|| {
            format!("Unable to read persistent bridge state at {}", path.display())
        }),
    }
}

pub async fn load_bridge_config(path: &Path, state_file: &Path) -> anyhow::Result<BridgeConfig> {
    let defaults = default_bridge_config(state_file);

    match fs::read_to_string(path).await {
        Ok(contents) => {
            let mut config = parse_bridge_config(&contents, path)?;
            apply_config_defaults(&mut config, &defaults);
            Ok(config)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(legacy_path) = legacy_bridge_json_path(path) {
                match fs::read_to_string(&legacy_path).await {
                    Ok(contents) => {
                        let mut config = parse_bridge_config(&contents, &legacy_path)?;
                        apply_config_defaults(&mut config, &defaults);
                        return Ok(config);
                    }
                    Err(legacy_error) if legacy_error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(legacy_error) => {
                        return Err(legacy_error).with_context(|| {
                            format!(
                                "Unable to read legacy bridge config at {}",
                                legacy_path.display()
                            )
                        });
                    }
                }
            }

            Ok(defaults)
        }
        Err(error) => Err(error)
            .with_context(|| format!("Unable to read bridge config at {}", path.display())),
    }
}

fn apply_config_defaults(config: &mut BridgeConfig, defaults: &BridgeConfig) {
    if config.download_root_dir.trim().is_empty() {
        config.download_root_dir.clone_from(&defaults.download_root_dir);
    }
    if config.local_gateway_base_url.trim().is_empty() {
        config.local_gateway_base_url.clone_from(&defaults.local_gateway_base_url);
    }
    if config.public_gateway_base_url.trim().is_empty() {
        config.public_gateway_base_url.clone_from(&defaults.public_gateway_base_url);
    }
    if config.relay_server_url.trim().is_empty() {
        config.relay_server_url.clone_from(&defaults.relay_server_url);
    }
    if config.relay_device_name.trim().is_empty() {
        config.relay_device_name.clone_from(&defaults.relay_device_name);
    }
}

pub async fn persist_bridge_state(state: &AppState) -> anyhow::Result<()> {
    let snapshot = { state.persistent.read().await.clone() };

    if let Some(parent) = state.state_file.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Unable to create state directory {}", parent.display()))?;
    }

    let json = serde_json::to_vec_pretty(&snapshot).context("Unable to encode bridge state")?;
    write_private_file(&state.state_file, &json).await.with_context(|| {
        format!("Unable to write persistent bridge state to {}", state.state_file.display())
    })?;

    Ok(())
}

pub async fn persist_bridge_config(state: &AppState) -> anyhow::Result<()> {
    let snapshot = { state.config.read().await.clone() };

    if let Some(parent) = state.config_file.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Unable to create config directory {}", parent.display()))?;
    }

    if bridge_config_uses_yaml(&state.config_file) {
        let yaml =
            serde_yaml::to_string(&snapshot).context("Unable to encode bridge config as YAML")?;
        write_private_file(&state.config_file, yaml.as_bytes()).await.with_context(|| {
            format!("Unable to write bridge config to {}", state.config_file.display())
        })?;
    } else {
        let json =
            serde_json::to_vec_pretty(&snapshot).context("Unable to encode bridge config")?;
        write_private_file(&state.config_file, &json).await.with_context(|| {
            format!("Unable to write bridge config to {}", state.config_file.display())
        })?;
    }

    Ok(())
}

// Wide patch shape: each optional field drives one independent branch. Not
// genuinely convoluted; the cognitive score reflects breadth, not depth.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn apply_config_update(
    state: &AppState,
    input: UpdateBridgeConfigRequest,
) -> Result<BridgeConfigResponse, AppError> {
    {
        let mut config = state.config.write().await;

        if let Some(download_root_dir) = input.download_root_dir {
            let trimmed = download_root_dir.trim();
            if trimmed.is_empty() {
                return Err(AppError::bad_request("download_root_dir cannot be empty"));
            }
            config.download_root_dir = trimmed.to_string();
        }

        if let Some(sync_enabled) = input.sync_enabled {
            config.sync_enabled = sync_enabled;
        }

        if let Some(local_gateway_base_url) = input.local_gateway_base_url {
            let trimmed = local_gateway_base_url.trim();
            if trimmed.is_empty() {
                return Err(AppError::bad_request("local_gateway_base_url cannot be empty"));
            }
            config.local_gateway_base_url = trimmed.to_string();
        }

        if let Some(public_gateway_base_url) = input.public_gateway_base_url {
            let trimmed = public_gateway_base_url.trim();
            if trimmed.is_empty() {
                return Err(AppError::bad_request("public_gateway_base_url cannot be empty"));
            }
            config.public_gateway_base_url = trimmed.to_string();
        }

        if let Some(relay_enabled) = input.relay_enabled {
            config.relay_enabled = relay_enabled;
        }

        if let Some(relay_server_url) = input.relay_server_url {
            let trimmed = relay_server_url.trim();
            if trimmed.is_empty() {
                return Err(AppError::bad_request("relay_server_url cannot be empty"));
            }
            if trimmed != config.relay_server_url.trim() {
                config.relay_enabled = false;
                config.relay_device_id = None;
                config.relay_device_label = None;
                config.relay_device_token = None;
                config.relay_last_connected_at = None;
                config.relay_last_error =
                    Some("Relay server changed. Pair this desktop app again.".to_string());
            }
            config.relay_server_url = trimmed.to_string();
        }

        if let Some(relay_device_name) = input.relay_device_name {
            let trimmed = relay_device_name.trim();
            if trimmed.is_empty() {
                return Err(AppError::bad_request("relay_device_name cannot be empty"));
            }
            config.relay_device_name = trimmed.to_string();
        }

        if let Some(tunnel_enabled) = input.tunnel_enabled
            && tunnel_enabled != config.tunnel_enabled
        {
            config.tunnel_enabled = tunnel_enabled;
            if !tunnel_enabled {
                // Clear stale error on toggle-off; the tunnel loop handles
                // the actual revoke + stopping cloudflared.
                config.tunnel_last_error = None;
            }
        }

        if let Some(quota) = input.storage_quota_gb {
            config.storage_quota_gb = quota;
        }

        if let Some(retries) = input.max_retry_attempts {
            config.max_retry_attempts = retries;
        }

        if let Some(enabled) = input.remote_pinning_enabled {
            config.remote_pinning_enabled = enabled;
        }

        if let Some(name) = input.remote_pinning_service_name {
            config.remote_pinning_service_name = name.filter(|value| !value.trim().is_empty());
        }

        if let Some(url) = input.remote_pinning_service_url {
            config.remote_pinning_service_url = url.filter(|value| !value.trim().is_empty());
        }

        if let Some(token) = input.remote_pinning_access_token {
            config.remote_pinning_access_token = token.filter(|value| !value.trim().is_empty());
        }

        if config.onboarded_at.is_none() {
            config.onboarded_at = Some(Utc::now());
        }
    }

    persist_bridge_config(state).await.map_err(AppError::internal)?;

    let config = state.config.read().await;
    Ok(crate::model::system::service::build_config_response(state, &config))
}

pub fn bridge_state_file_from_env() -> anyhow::Result<PathBuf> {
    if let Some(value) = env::var("BRIDGE_STATE_FILE").ok().filter(|value| !value.trim().is_empty())
    {
        return Ok(PathBuf::from(value));
    }

    let cwd = env::current_dir().context("Unable to determine current directory")?;
    Ok(cwd.join("bridge-state.json"))
}

pub fn bridge_config_file_from_env(state_file: &Path) -> anyhow::Result<PathBuf> {
    if let Some(value) =
        env::var("BRIDGE_CONFIG_FILE").ok().filter(|value| !value.trim().is_empty())
    {
        return Ok(PathBuf::from(value));
    }

    if let Some(parent) = state_file.parent() {
        let yaml_path = parent.join("bridge-config.yaml");
        if yaml_path.exists() {
            return Ok(yaml_path);
        }

        let yml_path = parent.join("bridge-config.yml");
        if yml_path.exists() {
            return Ok(yml_path);
        }

        let json_path = parent.join("bridge-config.json");
        if json_path.exists() {
            return Ok(json_path);
        }

        return Ok(yaml_path);
    }

    let cwd = env::current_dir().context("Unable to determine current directory")?;
    Ok(cwd.join("bridge-config.yaml"))
}
