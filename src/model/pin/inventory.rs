//! Pure inventory helpers — cursor parsing, page sizing, descriptor assembly,
//! inventory row rendering, and error categorization.
//!
//! Nothing in here touches [`crate::AppState`]. Anything that needs live state
//! belongs in the services layer.

use std::collections::{HashMap, HashSet};

use super::types::{InventoryEntryDescriptor, InventorySourcePin, PinInventoryItem, WatchedPin};
use crate::model::config::{
    BridgeConfig, BridgePersistentState, service::effective_public_gateway_base_url,
};
use crate::util::{
    format::format_timestamp,
    text::escape_html,
    url::{build_gateway_url, build_public_utility_gateway_url},
};

pub const INVENTORY_PAGE_SIZE: usize = 12;
pub const INVENTORY_MAX_PAGE_SIZE: usize = 24;

pub fn parse_inventory_cursor(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok()).unwrap_or(0)
}

pub fn resolve_inventory_page_size(raw: Option<usize>) -> usize {
    raw.unwrap_or(INVENTORY_PAGE_SIZE).clamp(1, INVENTORY_MAX_PAGE_SIZE)
}

pub fn collect_inventory_descriptors(
    pinset: &HashMap<String, String>,
    persistent: &BridgePersistentState,
) -> Vec<InventoryEntryDescriptor> {
    let mut grouped_work_members = HashMap::<String, Vec<InventorySourcePin>>::new();
    let mut descriptors = Vec::new();

    for watched in persistent.watched_pins.values() {
        let source = InventorySourcePin {
            cid: watched.cid.clone(),
            pinned: pinset.contains_key(&watched.cid),
            pin_type: pinset.get(&watched.cid).cloned(),
            watched: watched.clone(),
        };

        if let Some(group_key) = inventory_work_group_key(&source.watched) {
            grouped_work_members.entry(group_key).or_default().push(source);
        } else {
            descriptors.push(InventoryEntryDescriptor::Single(source));
        }
    }

    for members in grouped_work_members.into_values() {
        descriptors.push(InventoryEntryDescriptor::Work(members));
    }

    descriptors.sort_by_key(|entry| std::cmp::Reverse(entry.added_at()));
    descriptors
}

pub fn inventory_work_group_key(pin: &WatchedPin) -> Option<String> {
    if pin.source_kind != "work" {
        return None;
    }

    if let (Some(contract_address), Some(token_id)) =
        (pin.contract_address.as_deref(), pin.token_id.as_deref())
    {
        let contract = contract_address.trim().to_ascii_lowercase();
        let token = token_id.trim();
        if !contract.is_empty() && !token.is_empty() {
            return Some(format!("work:{contract}:{token}"));
        }
    }

    if let Some(foundation_url) = pin.foundation_url.as_deref() {
        let trimmed = foundation_url.trim();
        if !trimmed.is_empty() {
            return Some(format!("work-url:{trimmed}"));
        }
    }

    if let Some(title) = pin.title.as_deref() {
        let trimmed = title.trim();
        if !trimmed.is_empty() {
            return Some(format!(
                "work-title:{}:{}",
                trimmed.to_ascii_lowercase(),
                pin.artist_username.as_deref().unwrap_or_default().trim().to_ascii_lowercase()
            ));
        }
    }

    None
}

pub fn related_cids_from_members(members: &[InventorySourcePin]) -> Vec<String> {
    let mut seen = HashSet::new();
    members.iter().map(|member| member.cid.clone()).filter(|cid| seen.insert(cid.clone())).collect()
}

// The by-value `source` mirrors the original call sites, which always hand the
// helper an owned descriptor they have no further use for; taking it by
// reference would force every caller to clone every field instead.
#[allow(clippy::needless_pass_by_value)]
pub fn build_single_inventory_item(
    config: &BridgeConfig,
    source: InventorySourcePin,
) -> PinInventoryItem {
    let cid = source.cid.clone();

    PinInventoryItem {
        cid: cid.clone(),
        pinned: source.pinned,
        pin_type: source.pin_type,
        managed: true,
        label: source.watched.label,
        source_kind: Some(source.watched.source_kind),
        title: source.watched.title,
        contract_address: source.watched.contract_address,
        token_id: source.watched.token_id,
        foundation_url: source.watched.foundation_url,
        artist_username: source.watched.artist_username,
        account_address: source.watched.account_address,
        username: source.watched.username,
        added_at: Some(source.watched.added_at),
        last_verified_at: source.watched.last_verified_at,
        last_repaired_at: source.watched.last_repaired_at,
        last_error: source.watched.last_error,
        pin_reference: source.watched.pin_reference,
        verify_count: source.watched.verify_count,
        repair_count: source.watched.repair_count,
        sync_path: source.watched.sync_path,
        local_gateway_url: Some(build_gateway_url(&config.local_gateway_base_url, &cid)),
        public_gateway_url: Some(build_gateway_url(
            &effective_public_gateway_base_url(config),
            &cid,
        )),
        preview_local_gateway_url: Some(build_gateway_url(&config.local_gateway_base_url, &cid)),
        preview_public_gateway_url: Some(build_gateway_url(
            &effective_public_gateway_base_url(config),
            &cid,
        )),
        media_kind: None,
        metadata_view: None,
        metadata_cid: None,
        media_cid: None,
        related_cids: vec![cid],
        last_synced_at: source.watched.last_synced_at,
        last_sync_error: source.watched.last_sync_error,
        sync_count: source.watched.sync_count,
        retry_attempts: source.watched.retry_attempts,
        next_retry_at: source.watched.next_retry_at,
        error_category: source.watched.error_category,
        provider_count: source.watched.provider_count,
        provider_checked_at: source.watched.provider_checked_at,
        custom_tags: source.watched.custom_tags,
        remote_pinned: source.watched.remote_pinned,
        remote_pin_service: source.watched.remote_pin_service,
        remote_pin_last_error: source.watched.remote_pin_last_error,
    }
}

#[allow(clippy::format_collect)]
pub fn render_inventory_table_rows(items: &[PinInventoryItem], limit: usize) -> String {
    items
        .iter()
        .take(limit)
        .map(|pin| {
            let status_pill = if pin.pinned {
                format!(
                    r#"<span class="pill ok">{}</span>"#,
                    escape_html(pin.pin_type.as_deref().unwrap_or("pinned"))
                )
            } else {
                r#"<span class="pill warn">Repair needed</span>"#.to_string()
            };

            let links = {
                let mut parts = Vec::new();
                if let Some(url) = pin.local_gateway_url.as_deref() {
                    parts.push(format!(
                        r#"<a href="{}" target="_blank" rel="noreferrer">local</a>"#,
                        escape_html(url)
                    ));
                }
                if let Some(url) = pin.public_gateway_url.as_deref() {
                    parts.push(format!(
                        r#"<a href="{}" target="_blank" rel="noreferrer">pinned</a>"#,
                        escape_html(url)
                    ));
                }
                parts.push(format!(
                    r#"<a href="{}" target="_blank" rel="noreferrer">public</a>"#,
                    escape_html(&build_public_utility_gateway_url(&pin.cid))
                ));
                if parts.is_empty() {
                    r#"<span class="muted">—</span>"#.to_string()
                } else {
                    parts.join(" · ")
                }
            };

            format!(
                r#"<tr><td><div>{}</div><div class="cid">{}</div></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                escape_html(
                    pin.title.as_deref().or(pin.label.as_deref()).unwrap_or("Local IPFS pin")
                ),
                escape_html(&pin.cid),
                escape_html(
                    pin.foundation_url
                        .as_deref()
                        .or(pin.contract_address.as_deref())
                        .or(pin.username.as_deref())
                        .unwrap_or("—")
                ),
                status_pill,
                escape_html(
                    &pin.last_verified_at
                        .map_or_else(|| "never".to_string(), format_timestamp)
                ),
                links,
            )
        })
        .collect::<String>()
}

pub fn render_inventory_fallback_table(items: &[PinInventoryItem]) -> String {
    let rows = render_inventory_table_rows(items, INVENTORY_PAGE_SIZE);
    if rows.is_empty() {
        return r#"<div class="empty">No pins yet. Once the archive site hands you something to rescue, it will appear here.</div>"#.to_string();
    }

    format!(
        r#"<div class="table-wrap">
  <table>
    <thead>
      <tr>
        <th>Item</th>
        <th>Context</th>
        <th>Status</th>
        <th>Last verified</th>
        <th>Gateway</th>
      </tr>
    </thead>
    <tbody>{rows}</tbody>
  </table>
</div>"#
    )
}

pub fn categorize_pin_error(message: &str) -> (&'static str, &'static str) {
    let lower = message.to_ascii_lowercase();
    if lower.contains("connection refused")
        || lower.contains("failed to reach ipfs")
        || lower.contains("failed to connect")
        || lower.contains("connection reset")
        || lower.contains("dial tcp")
    {
        return (
            "daemon_unreachable",
            "The local IPFS daemon is not responding. Start Kubo and retry.",
        );
    }
    if lower.contains("deadline exceeded")
        || lower.contains("timeout")
        || lower.contains("timed out")
    {
        return ("timeout", "The IPFS network took too long to answer. Try again in a minute.");
    }
    if lower.contains("no providers")
        || lower.contains("could not find provider")
        || lower.contains("no route to host")
    {
        return (
            "no_providers",
            "No peers know about this CID yet. A remote pinning service can keep a copy.",
        );
    }
    if lower.contains("not pinned") {
        return ("not_pinned", "The CID is not pinned locally. The next cycle will pin it.");
    }
    if lower.contains("invalid") || lower.contains("not a valid cid") {
        return ("invalid_cid", "The CID looks malformed. Re-request the share.");
    }
    if lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("401")
        || lower.contains("403")
    {
        return ("unauthorized", "The IPFS API rejected the request. Verify IPFS_API_AUTH_HEADER.");
    }
    if lower.contains("disk") || lower.contains("no space") || lower.contains("quota") {
        return (
            "disk_full",
            "The IPFS datastore cannot accept more data. Free space or raise the quota.",
        );
    }
    ("unknown", "Cause not recognized. Check the detail for the raw message.")
}
