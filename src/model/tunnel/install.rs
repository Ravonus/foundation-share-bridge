//! Locate or fetch the `cloudflared` binary. Tried in order: explicit path
//! via `CLOUDFLARED_BINARY`, then system `PATH`, then a cached download from
//! Cloudflare's GitHub releases into the bridge's state directory.
//!
//! The tarball-vs-raw split is a Cloudflare release-asset quirk — macOS
//! builds are `.tgz`, Linux + Windows are raw binaries.
#![allow(clippy::pedantic, clippy::nursery)]

use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, anyhow};
use tokio::{fs, process::Command};
use tracing::{info, warn};

const DOWNLOAD_BASE: &str =
    "https://github.com/cloudflare/cloudflared/releases/latest/download";

/// Resolve the cloudflared binary path. Returns a path to an executable that
/// is known to launch (version check succeeded) by the time this returns.
pub async fn ensure_cloudflared_binary(cache_dir: &Path) -> anyhow::Result<PathBuf> {
    if let Some(path) = explicit_override()
        && is_runnable(&path).await
    {
        return Ok(path);
    }

    if is_runnable(Path::new("cloudflared")).await {
        return Ok(PathBuf::from("cloudflared"));
    }

    let cached = cache_dir.join(cached_binary_name());
    if is_runnable(&cached).await {
        return Ok(cached);
    }

    info!("cloudflared not found — downloading");
    download_and_install(cache_dir).await
}

fn explicit_override() -> Option<PathBuf> {
    env::var("CLOUDFLARED_BINARY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

async fn is_runnable(path: &Path) -> bool {
    Command::new(path)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .ok()
        .and_then(|status| status.success().then_some(()))
        .is_some()
}

fn cached_binary_name() -> &'static str {
    if cfg!(target_os = "windows") { "cloudflared.exe" } else { "cloudflared" }
}

fn asset_name() -> anyhow::Result<&'static str> {
    let name = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "x86_64") => "cloudflared-darwin-amd64.tgz",
        ("macos", "aarch64") => "cloudflared-darwin-arm64.tgz",
        ("linux", "x86_64") => "cloudflared-linux-amd64",
        ("linux", "aarch64") => "cloudflared-linux-arm64",
        ("linux", "arm") => "cloudflared-linux-armhf",
        ("windows", "x86_64") => "cloudflared-windows-amd64.exe",
        (os, arch) => {
            return Err(anyhow!(
                "No cloudflared prebuilt available for {os}/{arch}. Install cloudflared manually and set CLOUDFLARED_BINARY."
            ));
        }
    };
    Ok(name)
}

#[allow(clippy::cognitive_complexity)]
async fn download_and_install(cache_dir: &Path) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(cache_dir)
        .await
        .with_context(|| format!("Unable to create cache dir {}", cache_dir.display()))?;

    let asset = asset_name()?;
    let url = format!("{DOWNLOAD_BASE}/{asset}");
    let response = reqwest::get(&url)
        .await
        .with_context(|| format!("Unable to download {url}"))?;

    if !response.status().is_success() {
        return Err(anyhow!("cloudflared download failed: {}", response.status()));
    }

    let bytes =
        response.bytes().await.with_context(|| "Unable to read cloudflared download body")?;

    let target = cache_dir.join(cached_binary_name());

    if asset.ends_with(".tgz") {
        let tgz_path = cache_dir.join("cloudflared-download.tgz");
        fs::write(&tgz_path, &bytes).await.with_context(|| {
            format!("Unable to write cloudflared tarball to {}", tgz_path.display())
        })?;

        let status = Command::new("tar")
            .arg("-xzf")
            .arg(&tgz_path)
            .arg("-C")
            .arg(cache_dir)
            .status()
            .await
            .context("Unable to invoke tar to extract cloudflared")?;

        if !status.success() {
            return Err(anyhow!("tar failed to extract cloudflared ({status})"));
        }

        if let Err(error) = fs::remove_file(&tgz_path).await {
            warn!("Unable to clean up {}: {error}", tgz_path.display());
        }
    } else {
        fs::write(&target, &bytes)
            .await
            .with_context(|| format!("Unable to write cloudflared to {}", target.display()))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&target)
            .with_context(|| format!("Unable to stat {}", target.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target, perms)
            .with_context(|| format!("Unable to chmod {}", target.display()))?;
    }

    if !is_runnable(&target).await {
        return Err(anyhow!(
            "Downloaded cloudflared at {} did not run --version successfully",
            target.display()
        ));
    }

    info!("cloudflared installed at {}", target.display());
    Ok(target)
}
