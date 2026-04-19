//! Preservation-summary card aggregating pinned works by artist.

use std::collections::{HashMap, HashSet};

use crate::model::config::BridgePersistentState;
use crate::model::pin::inventory::inventory_work_group_key;
use crate::model::session::BridgeSession;
use crate::util::text::escape_html;

pub fn render_artist_summary(
    persistent: &BridgePersistentState,
    sessions: &HashMap<String, BridgeSession>,
) -> String {
    if persistent.watched_pins.is_empty() {
        return String::new();
    }

    let current_username = sessions.values().find_map(|s| s.profile_username.clone());

    let mut artist_counts: HashMap<String, HashSet<String>> = HashMap::new();
    let mut group_keys: HashSet<String> = HashSet::new();
    let mut works_by_you = 0_usize;

    for pin in persistent.watched_pins.values() {
        let group = inventory_work_group_key(pin).unwrap_or_else(|| pin.cid.clone());
        if group_keys.insert(group.clone()) {
            let artist = pin.artist_username.clone().unwrap_or_else(|| "unknown".to_string());
            artist_counts.entry(artist).or_default().insert(group.clone());
            if let Some(me) = current_username.as_deref()
                && pin.artist_username.as_deref().is_some_and(|v| v.eq_ignore_ascii_case(me))
            {
                works_by_you += 1;
            }
        }
    }

    let total_works = group_keys.len();
    let artists_tracked = artist_counts.len();
    let mut top: Vec<(String, usize)> =
        artist_counts.iter().map(|(artist, set)| (artist.clone(), set.len())).collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    top.truncate(5);

    let chips = if top.is_empty() {
        String::new()
    } else {
        let inner = top
            .iter()
            .map(|(artist, count)| {
                format!(r#"<span class="pill">@{} · {count}</span>"#, escape_html(artist))
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!(r#"<div class="btn-row" style="margin-top:14px;">{inner}</div>"#)
    };

    let me_line = match current_username.as_deref() {
        Some(name) if works_by_you > 0 => format!(
            r#"<p class="pin-context" style="margin-top:10px;">You are pinning {works_by_you} work{} that credit @{} as the artist.</p>"#,
            if works_by_you == 1 { "" } else { "s" },
            escape_html(name)
        ),
        Some(name) => format!(
            r#"<p class="pin-context" style="margin-top:10px;">No works by @{} are pinned on this device yet.</p>"#,
            escape_html(name)
        ),
        None => String::new(),
    };

    format!(
        r#"<section id="artists" class="card">
  <p class="eyebrow">Preservation summary</p>
  <h2 style="margin-top:8px;">You are caring for {total_works} work{plural} from {artists_tracked} artist{a_plural}</h2>
  <p class="muted settings-copy">This device keeps these roots pinned forever. Other collectors running the bridge may be pinning the same works alongside you.</p>
  {me_line}
  {chips}
</section>"#,
        plural = if total_works == 1 { "" } else { "s" },
        a_plural = if artists_tracked == 1 { "" } else { "s" },
    )
}
