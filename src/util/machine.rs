//! Stable machine identity helpers.
//!
//! `session_id` values stay the same for the same (website, machine) pair so
//! the archive site can auto-reconnect to the same session instead of
//! accumulating fresh ones each page load. The hostname is resolved with
//! `hostname(1)` on unix and `COMPUTERNAME` / `HOSTNAME` env vars otherwise;
//! callers fall back to a generic label if nothing is available.

use std::hash::{DefaultHasher, Hash, Hasher};

/// Best-effort machine hostname. Trimmed, never empty.
pub fn machine_hostname() -> String {
    if let Some(name) = read_hostname_command() {
        return name;
    }
    for var in ["COMPUTERNAME", "HOSTNAME", "HOST"] {
        if let Ok(value) = std::env::var(var) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    "foundation-desktop".to_string()
}

fn read_hostname_command() -> Option<String> {
    let output = std::process::Command::new("hostname").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8(output.stdout).ok()?;
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// Return a short, human-friendly label derived from the hostname. Used as
/// the default relay device name so users see something recognizable in the
/// desktop page instead of a generic placeholder.
pub fn default_device_label() -> String {
    let raw = machine_hostname();
    let stripped = raw.strip_suffix(".local").unwrap_or(&raw);
    let cleaned = stripped.trim();
    if cleaned.is_empty() {
        return "Foundation desktop app".to_string();
    }
    format!("Foundation on {cleaned}")
}

/// Deterministic session identifier for a given website origin. The same
/// machine + same origin always produces the same value so auto-reconnect
/// finds existing sessions instead of minting new ones.
pub fn deterministic_session_id(website_origin: &str) -> String {
    let host = machine_hostname();
    let mut hasher = DefaultHasher::new();
    host.hash(&mut hasher);
    website_origin.trim().to_ascii_lowercase().hash(&mut hasher);
    let digest = hasher.finish();
    format!("bridge-{host}-{digest:016x}")
}
