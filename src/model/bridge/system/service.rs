//! System service layer — storage snapshots, `OperationStatus` mutation,
//! user-visible OS-level notifications, and the config-response presentation
//! helper that the `/config` handlers reach for.

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
use anyhow::{Context, anyhow};
use chrono::Utc;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
use tokio::process::Command as TokioCommand;
use tracing::warn;

use super::types::StorageSnapshot;
use crate::{
    AppState, OperationStatus,
    model::{
        config::types::{BridgeConfig, BridgeConfigResponse},
        pin::client::{kubo::fetch_kubo_repo_stat, sync::measure_synced_bytes_on_disk},
    },
};

pub async fn build_storage_snapshot(state: &AppState) -> StorageSnapshot {
    let (repo_size, storage_max, num_objects, ipfs_daemon_reachable) =
        match fetch_kubo_repo_stat(state).await {
            Ok(stat) => (stat.repo_size, stat.storage_max, stat.num_objects, true),
            Err(_) => (None, None, None, false),
        };
    let synced_bytes = measure_synced_bytes_on_disk(state).await;
    let quota_gb = { state.config.read().await.storage_quota_gb };
    let quota_used_fraction = match (quota_gb, repo_size) {
        (Some(gb), Some(bytes)) if gb > 0.0 => {
            let max_bytes = gb * 1_000_000_000.0;
            // `bytes as f64` is a deliberate storage-stats approximation —
            // repo sizes up to several exabytes round to within a byte once
            // divided against a gigabyte quota.
            #[allow(clippy::cast_precision_loss)]
            let used = bytes as f64;
            if max_bytes > 0.0 { Some(used / max_bytes) } else { None }
        }
        _ => None,
    };
    StorageSnapshot {
        repo_size_bytes: repo_size,
        storage_max_bytes: storage_max,
        num_objects,
        synced_bytes_on_disk: synced_bytes,
        quota_gb,
        quota_used_fraction,
        ipfs_daemon_reachable,
        checked_at: Utc::now(),
    }
}

pub async fn set_current_operation(state: &AppState, status: OperationStatus) {
    *state.operation.write().await = status;
}

pub async fn update_current_operation(
    state: &AppState,
    detail: Option<String>,
    progress_current: Option<usize>,
) {
    let mut guard = state.operation.write().await;
    guard.updated_at = Utc::now();
    if let Some(detail) = detail {
        guard.detail = Some(detail);
    }
    if let Some(progress) = progress_current {
        guard.progress_current = Some(progress);
    }
}

pub async fn clear_current_operation(state: &AppState) {
    *state.operation.write().await = OperationStatus::idle();
}

pub async fn show_backup_notification(body: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = TokioCommand::new("osascript")
            .arg("-e")
            .arg(format!(
                "display notification \"{}\" with title \"Foundation Share Bridge\" subtitle \"Backup saved\"",
                crate::util::text::escape_notification_text(body)
            ))
            .status()
            .await
            .context("Unable to launch macOS notification command")?;

        if !status.success() {
            return Err(anyhow!("macOS notification command exited unsuccessfully"));
        }

        Ok(())
    }

    #[cfg(target_os = "linux")]
    {
        let safe_body = crate::util::text::escape_linux_notification_text(body);
        let status = TokioCommand::new("notify-send")
            .arg("--")
            .arg("Foundation Share Bridge")
            .arg(&safe_body)
            .status()
            .await
            .context("Unable to launch Linux notification command")?;

        if !status.success() {
            return Err(anyhow!("Linux notification command exited unsuccessfully"));
        }

        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        let script = format!(
            r#"
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType=WindowsRuntime] > $null
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType=WindowsRuntime] > $null
$template = @"
<toast>
  <visual>
    <binding template="ToastGeneric">
      <text>Foundation Share Bridge</text>
      <text>Backup saved</text>
      <text>{}</text>
    </binding>
  </visual>
</toast>
"@
$xml = New-Object Windows.Data.Xml.Dom.XmlDocument
$xml.LoadXml($template)
$toast = [Windows.UI.Notifications.ToastNotification]::new($xml)
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier("Foundation Share Bridge").Show($toast)
"#,
            crate::util::text::escape_xml_text(body)
        );

        let status = TokioCommand::new("powershell")
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(script)
            .status()
            .await
            .context("Unable to launch Windows notification command")?;

        if !status.success() {
            return Err(anyhow!("Windows notification command exited unsuccessfully"));
        }

        return Ok(());
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = body;
        Ok(())
    }
}

pub fn notify_work_share_success(title: &str, pin_count: usize) {
    let work_title = title.trim();
    let label = if work_title.is_empty() {
        "your Foundation work".to_string()
    } else {
        format!("\"{work_title}\"")
    };
    let plural = if pin_count == 1 { "" } else { "s" };
    let body = format!(
        "Saved backup for {label}. {pin_count} root{plural} pinned and now on the watch list."
    );

    tokio::spawn(async move {
        if let Err(error) = show_backup_notification(&body).await {
            warn!("backup notification failed: {error}");
        }
    });
}

pub fn build_config_response(state: &AppState, config: &BridgeConfig) -> BridgeConfigResponse {
    BridgeConfigResponse {
        download_root_dir: config.download_root_dir.clone(),
        sync_enabled: config.sync_enabled,
        local_gateway_base_url: config.local_gateway_base_url.clone(),
        public_gateway_base_url: config.public_gateway_base_url.clone(),
        relay_enabled: config.relay_enabled,
        relay_server_url: config.relay_server_url.clone(),
        relay_device_name: config.relay_device_name.clone(),
        relay_device_id: config.relay_device_id.clone(),
        relay_device_label: config.relay_device_label.clone(),
        relay_last_connected_at: config.relay_last_connected_at,
        relay_last_error: config.relay_last_error.clone(),
        tunnel_enabled: config.tunnel_enabled,
        tunnel_hostname: config.tunnel_hostname.clone(),
        tunnel_last_error: config.tunnel_last_error.clone(),
        tunnel_provisioned_at: config.tunnel_provisioned_at,
        libp2p_hostname: config.libp2p_hostname.clone(),
        libp2p_ws_local_port: config
            .libp2p_ws_local_port
            .unwrap_or(crate::model::relay::tunnel::kubo_announce::DEFAULT_LIBP2P_WS_PORT),
        libp2p_last_error: config.libp2p_last_error.clone(),
        libp2p_applied_at: config.libp2p_applied_at,
        config_file: state.config_file.display().to_string(),
        storage_quota_gb: config.storage_quota_gb,
        max_retry_attempts: config.max_retry_attempts,
        remote_pinning_enabled: config.remote_pinning_enabled,
        remote_pinning_service_name: config.remote_pinning_service_name.clone(),
        remote_pinning_service_url: config.remote_pinning_service_url.clone(),
        remote_pinning_access_token_configured: config
            .remote_pinning_access_token
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        onboarded_at: config.onboarded_at,
    }
}
