//! Preservation-summary card aggregating pinned works by artist.

use std::collections::{HashMap, HashSet};

use crate::model::config::BridgePersistentState;
use crate::model::pin::inventory::inventory_work_group_key;
use crate::model::session::BridgeSession;
use crate::util::text::escape_html;

fn archive_any_form_html() -> &'static str {
    r#"<form action="/artists/archive-all/form" method="post" class="archive-any-form">
  <label class="field">
    <span>Archive an artist's full Foundation catalog</span>
    <input name="username" placeholder="foundation-username" autocomplete="off" />
  </label>
  <div class="btn-row">
    <button type="submit" class="btn">Archive everything by this artist</button>
  </div>
  <p class="muted" style="margin-top:6px;font-size:0.78rem;">Uses the archive site's Foundation catalog feed. Progress shows up in the live-status panel above.</p>
</form>"#
}

fn render_empty_artist_summary(current_username: Option<&str>) -> String {
    let me_line = match current_username {
        Some(name) => format!(
            r#"<p class="pin-context" style="margin-top:10px;">No works by @{} pinned here yet.</p>"#,
            escape_html(name)
        ),
        None => String::new(),
    };
    let form = archive_any_form_html();
    format!(
        r#"<section id="artists" class="card">
  <p class="eyebrow">Preservation summary</p>
  <h2 style="margin-top:8px;">Start archiving an artist's full Foundation catalog</h2>
  <p class="muted settings-copy">Enter any Foundation username and this bridge will pin every work they have.</p>
  {me_line}
  {form}
</section>"#,
    )
}

pub fn render_artist_summary(
    persistent: &BridgePersistentState,
    sessions: &HashMap<String, BridgeSession>,
) -> String {
    let current_username = sessions.values().find_map(|s| s.profile_username.clone());

    if persistent.watched_pins.is_empty() {
        return render_empty_artist_summary(current_username.as_deref());
    }

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
                let escaped = escape_html(artist);
                let unknown = artist == "unknown" || artist.trim().is_empty();
                let action = if unknown {
                    String::new()
                } else {
                    format!(
                        r#"<form action="/artists/archive-all/form" method="post" class="archive-chip-form">
<input type="hidden" name="username" value="{escaped}" />
<button type="submit" class="btn ghost small" title="Pin everything by @{escaped} onto this machine">Archive all</button>
</form>"#,
                    )
                };
                format!(
                    r#"<span class="archive-chip"><span class="pill">@{escaped} · {count}</span>{action}</span>"#,
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!(r#"<div class="archive-chip-row">{inner}</div>"#)
    };

    let archive_any_form = archive_any_form_html();

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
  {archive_any_form}
</section>"#,
        plural = if total_works == 1 { "" } else { "s" },
        a_plural = if artists_tracked == 1 { "" } else { "s" },
    )
}
