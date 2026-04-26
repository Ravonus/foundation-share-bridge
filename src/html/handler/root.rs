//! HTML handler for the bridge root (`GET /`) page.
#![allow(clippy::too_many_lines, clippy::cognitive_complexity, clippy::pedantic, clippy::nursery)]

use std::collections::HashMap;

use axum::{
    extract::{Query, State},
    response::Html,
};

use crate::{
    AppError, AppState,
    html::{
        render::{
            artist::render_artist_summary,
            page::render_page,
            settings::{render_export_card, render_gateway_card},
            status::render_live_status_panel,
        },
        scripts::{
            autolink::ROOT_AUTOLINK_SCRIPT, inventory::INVENTORY_BROWSER_SCRIPT,
            live_status::LIVE_STATUS_SCRIPT,
        },
    },
    model::{
        config::RootPageQuery,
        pin::{
            inventory::{INVENTORY_PAGE_SIZE, render_inventory_fallback_table},
            service::list_local_pin_inventory,
        },
        relay::relay_is_connected,
        session::types::BridgeSession,
        system::service::build_storage_snapshot,
    },
    util::{
        format::{format_bytes_human, format_timestamp},
        text::escape_html,
    },
};

/// Upload card — lets the local UI drop files that aren't on Foundation into
/// the bridge's IPFS node. Loopback-trusted (server binds 127.0.0.1 + CORS).
fn render_upload_card() -> &'static str {
    r#"<section id="upload" class="card">
  <p class="eyebrow">Upload &amp; pin</p>
  <h2 style="margin-top:8px;">Pin your own files on this node</h2>
  <p class="muted settings-copy">Drop one or more files here. The bridge wraps multi-file uploads in a directory and keeps the root CID pinned forever. Use this for anything that isn't already on Foundation.</p>
  <form action="/ipfs/add/form" method="post" enctype="multipart/form-data" class="upload-form">
    <label class="field">
      <span>Label (optional)</span>
      <input name="label" placeholder="e.g. Studio archive 2026" autocomplete="off" />
    </label>
    <label class="field">
      <span>Files</span>
      <input type="file" name="file" multiple required />
    </label>
    <div class="btn-row">
      <button type="submit" class="btn">Upload &amp; pin</button>
    </div>
  </form>
</section>"#
}

/// Render a card listing every active session with a disconnect button so
/// stale links can be pruned from the local UI. `selected` highlights the
/// session the user opened via `?session_id=` from the archive site.
fn render_sessions_card(
    sessions: &HashMap<String, BridgeSession>,
    selected: Option<&BridgeSession>,
) -> String {
    if sessions.is_empty() {
        return r#"<section class="card">
  <p class="eyebrow">Sessions</p>
  <h2>No archive sites connected</h2>
  <p class="muted" style="margin-top: 10px;">Sessions show up here after the archive site hands off a share.</p>
</section>"#
            .to_string();
    }

    let mut entries: Vec<&BridgeSession> = sessions.values().collect();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.connected_at));

    let rows = entries
        .into_iter()
        .map(|session| {
            let is_selected = selected.map(|s| s.session_id == session.session_id).unwrap_or(false);
            format!(
                r#"<li class="session-row{selected_cls}">
  <div class="session-meta">
    <p class="session-origin">{origin}</p>
    <p class="session-started muted">Started {started}</p>
    <p class="session-id muted"><code>{id}</code></p>
  </div>
  <form action="/session/{id_attr}/disconnect/form" method="post" class="session-delete">
    <button type="submit" class="btn ghost">Delete</button>
  </form>
</li>"#,
                selected_cls = if is_selected { " selected" } else { "" },
                origin = escape_html(&session.website_origin),
                started = escape_html(&format_timestamp(session.connected_at)),
                id = escape_html(&session.session_id),
                id_attr = escape_html(&session.session_id),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<section class="card">
  <p class="eyebrow">Sessions</p>
  <h2>Connected archive sites</h2>
  <p class="muted" style="margin-top: 10px;">Delete any rows that belong to tabs, origins, or machines you don't use anymore.</p>
  <ul class="session-list">
{rows}
  </ul>
</section>"#,
    )
}

pub async fn root_page(
    State(state): State<AppState>,
    Query(query): Query<RootPageQuery>,
) -> Result<Html<String>, AppError> {
    let persistent = state.persistent.read().await.clone();
    let sessions = state.sessions.read().await.clone();
    let config = state.config.read().await.clone();

    let selected_session = query.session_id.as_deref().and_then(|session_id| {
        sessions.values().find(|session| session.session_id == session_id).cloned()
    });

    let inventory = list_local_pin_inventory(&state).await.map_err(AppError::internal)?;

    let relay_connected = relay_is_connected(&config);
    let relay_server_value =
        query.relay_server_url.as_deref().unwrap_or(config.relay_server_url.as_str());
    let pairing_code_value = query.pairing_code.as_deref().unwrap_or("");
    let device_name_value = query
        .device_name
        .as_deref()
        .or(Some(config.relay_device_name.as_str()))
        .unwrap_or("Foundation desktop helper");
    let autolink_requested =
        query.autolink.as_deref() == Some("1") && !pairing_code_value.trim().is_empty();

    let connection_block = if relay_connected {
        format!(
            r#"<section id="connection" class="card">
  <p class="eyebrow">Archive relay</p>
  <h2>Connected</h2>
  <dl class="kv" style="margin-top: 14px;">
    <dt>Device</dt><dd>{device}</dd>
    <dt>Server</dt><dd>{server}</dd>
    <dt>Last connected</dt><dd>{last}</dd>
  </dl>
  <form action="/relay/unlink/form" method="post" class="btn-row">
    <button type="submit" class="btn ghost">Disconnect this app</button>
  </form>
</section>"#,
            device = escape_html(
                config
                    .relay_device_label
                    .as_deref()
                    .or(config.relay_device_id.as_deref())
                    .unwrap_or("Connected")
            ),
            server = escape_html(&config.relay_server_url),
            last = escape_html(
                &config
                    .relay_last_connected_at
                    .map(format_timestamp)
                    .unwrap_or_else(|| "not yet".to_string())
            ),
        )
    } else if autolink_requested {
        format!(
            r#"<section id="connection" class="card">
  <p class="eyebrow">Pair with archive</p>
  <h2>Finishing your connection…</h2>
  <p class="muted" style="margin-top: 10px;">This local helper page opened from the archive site. It will confirm the one-time pairing automatically so you can see the connection happen here instead of guessing in the background.</p>
  <dl class="kv" style="margin-top: 16px;">
    <dt>Archive server</dt><dd>{server}</dd>
    <dt>Desktop name</dt><dd>{name}</dd>
    <dt>Pairing code</dt><dd><code>{code}</code></dd>
  </dl>
  <form id="autolink-form" action="/relay/link/form" method="post" class="btn-row" style="margin-top: 24px;">
    <input type="hidden" name="relay_server_url" value="{server_attr}" />
    <input type="hidden" name="pairing_code" value="{code_attr}" />
    <input type="hidden" name="device_name" value="{name_attr}" />
    <button type="submit" class="btn">Finish connection now</button>
    <a class="btn ghost" href="/settings">Open settings</a>
  </form>
  <p class="muted" id="autolink-status" style="margin-top: 12px;">Waiting for this helper to confirm with the archive site…</p>
</section>
<script>{script}</script>"#,
            server = escape_html(relay_server_value),
            name = escape_html(device_name_value),
            code = escape_html(pairing_code_value),
            server_attr = escape_html(relay_server_value),
            code_attr = escape_html(pairing_code_value),
            name_attr = escape_html(device_name_value),
            script = ROOT_AUTOLINK_SCRIPT,
        )
    } else {
        format!(
            r#"<section id="connection" class="card">
  <p class="eyebrow">Pair with archive</p>
  <h2>Connect with a pairing code</h2>
  <p class="muted" style="margin-top: 10px;">Open the app link from the archive site, or paste the pairing details here. The socket only stays active after this link is confirmed.</p>
  <form action="/relay/link/form" method="post">
    <label class="field">
      <span>Archive server URL</span>
      <input name="relay_server_url" value="{server}" placeholder="https://archive.example.com" />
    </label>
    <label class="field">
      <span>Pairing code</span>
      <input name="pairing_code" value="{code}" placeholder="ABCD1234" />
    </label>
    <label class="field">
      <span>Desktop name</span>
      <input name="device_name" value="{name}" placeholder="Studio MacBook" />
    </label>
    <div class="btn-row">
      <button type="submit" class="btn">Link this app</button>
    </div>
  </form>
</section>"#,
            server = escape_html(relay_server_value),
            code = escape_html(pairing_code_value),
            name = escape_html(device_name_value),
        )
    };

    let flash_block = if query.linked.as_deref() == Some("1") {
        r#"<div class="flash ok">Archive relay connected. This desktop app can now receive live pin jobs.</div>"#.to_string()
    } else if query.unlinked.as_deref() == Some("1") {
        r#"<div class="flash warn">Archive relay disconnected on this desktop app.</div>"#
            .to_string()
    } else if let Some(username) = query.archiving.as_deref() {
        format!(
            r#"<div class="flash ok">Archiving everything by @{}. The live-status panel below shows progress.</div>"#,
            escape_html(username),
        )
    } else if let Some(cid) = query.uploaded.as_deref() {
        format!(
            r#"<div class="flash ok">Upload pinned as <code>{}</code>. It will survive future repair cycles.</div>"#,
            escape_html(cid),
        )
    } else if let Some(error) = query.error.as_deref() {
        format!(r#"<div class="flash err">{}</div>"#, escape_html(error))
    } else {
        String::new()
    };

    let session_block = render_sessions_card(&sessions, selected_session.as_ref());

    let connection_status = if relay_connected { "Live" } else { "Not linked" };
    let connection_pill_class = if relay_connected { "pill ok" } else { "pill" };

    let inventory_body = if inventory.items.is_empty() {
        r#"<div class="empty">No pins yet. Once the archive site hands you something to rescue, it will appear here.</div>"#.to_string()
    } else {
        let fallback_table = render_inventory_fallback_table(&inventory.items);
        format!(
            r#"<div class="inventory-browser-head">
  <p class="muted">Live previews load {page_size} pins at a time so the bridge doesn&apos;t hit every gateway all at once.</p>
</div>
<div id="inventory-browser" class="inventory-browser" data-page-size="{page_size}">
  <div id="inventory-grid" class="pin-grid" aria-live="polite"></div>
  <div id="inventory-empty" class="empty" hidden>No pins are available right now.</div>
  <div class="inventory-load-row">
    <button type="button" id="inventory-load-more" class="btn ghost" hidden>Load more pins</button>
    <p id="inventory-status" class="muted inventory-status">Loading previews…</p>
  </div>
  <div id="inventory-sentinel" class="inventory-sentinel" aria-hidden="true"></div>
</div>
<noscript>{fallback}</noscript>
<script>{script}</script>"#,
            page_size = INVENTORY_PAGE_SIZE,
            fallback = fallback_table,
            script = INVENTORY_BROWSER_SCRIPT,
        )
    };

    let pinned_count = inventory.pinned_count;
    let managed_count = inventory.managed_count;
    let repair_interval = state.repair_interval_seconds;
    let last_repair = persistent
        .last_repair_cycle_at
        .map(format_timestamp)
        .unwrap_or_else(|| "never".to_string());

    let storage_snapshot = build_storage_snapshot(&state).await;
    let disk_used = match storage_snapshot.repo_size_bytes {
        Some(bytes) => format_bytes_human(bytes),
        None => "—".to_string(),
    };
    let disk_body = match (storage_snapshot.quota_gb, storage_snapshot.quota_used_fraction) {
        (Some(gb), Some(fraction)) => {
            format!("Quota {:.1} GB · {}% used", gb, (fraction * 100.0).round() as i64)
        }
        _ => {
            if storage_snapshot.ipfs_daemon_reachable {
                "Reported by the Kubo repo/stat API.".to_string()
            } else {
                "IPFS daemon not reachable — start Kubo to see usage.".to_string()
            }
        }
    };

    let pending_failures =
        persistent.watched_pins.values().filter(|pin| pin.last_error.is_some()).count();
    let final_failures = persistent
        .watched_pins
        .values()
        .filter(|pin| pin.final_failure_reported_at.is_some())
        .count();
    let failure_banner = if pending_failures == 0 {
        String::new()
    } else {
        let cls = if final_failures > 0 { "flash err" } else { "flash warn" };
        let copy = if final_failures > 0 {
            format!(
                "{pending_failures} pin{} report errors right now, and {final_failures} have exhausted their retry budget. Open a card to diagnose or retry sooner.",
                if pending_failures == 1 { "" } else { "s" }
            )
        } else {
            format!(
                "{pending_failures} pin{} are waiting for a retry. Open a card to diagnose or retry sooner.",
                if pending_failures == 1 { "" } else { "s" }
            )
        };
        format!(r#"<div class="{cls}">{}</div>"#, escape_html(&copy))
    };

    let artist_summary_html = {
        let sessions_guard = state.sessions.read().await;
        render_artist_summary(&persistent, &sessions_guard)
    };
    let gateway_card = render_gateway_card(&config);
    let export_card = render_export_card();
    let upload_card = render_upload_card();
    let live_status_block = {
        let op_guard = state.operation.read().await;
        render_live_status_panel(&op_guard)
    };

    let body = format!(
        r##"<main class="shell">
  <div class="stack">
    <section class="section-head">
      <p class="eyebrow">Agorix · Share bridge</p>
      <h1>Keep rescued IPFS roots pinned and self-repaired.</h1>
      <p class="lead">This local companion app for the Agorix Foundation archive keeps a memory of watched CIDs, re-checks them forever, and re-pins anything your IPFS node drops. Pair it with the archive site once, then leave it running.</p>
      <div class="btn-row">
        <a class="pill {conn_pill}" href="#connection">{conn_status}</a>
        <span class="pill">{repair_interval}s repair cadence</span>
        <a class="btn ghost" href="/settings">Open settings</a>
      </div>
    </section>

    {flash}
    {failure_banner}

    {live_status_block}

    <section id="status">
      <div class="stats">
        <div class="stat">
          <p class="eyebrow">Pinned now</p>
          <p class="stat-value">{pinned}</p>
          <p class="stat-body">Currently present in your local IPFS node.</p>
        </div>
        <div class="stat">
          <p class="eyebrow">Managed forever</p>
          <p class="stat-value">{managed}</p>
          <p class="stat-body">Watched roots this app will keep repairing.</p>
        </div>
        <div class="stat">
          <p class="eyebrow">Disk used</p>
          <p class="stat-value" style="font-size: 1.4rem;">{disk_used}</p>
          <p class="stat-body">{disk_body}</p>
        </div>
        <div class="stat">
          <p class="eyebrow">Last repair</p>
          <p class="stat-value" style="font-size: 1rem; font-family: ui-monospace, Menlo, Consolas, monospace;">{last_repair}</p>
          <p class="stat-body">{repair_interval}s cadence · missing pins are restored on the next cycle.</p>
        </div>
      </div>
    </section>

    <section class="two-col">
      {connection}
      {session}
    </section>

    {artist_summary_html}

    {upload_card}

    <section id="inventory">
      <div class="section-head" style="border-bottom: 0; padding-bottom: 0;">
        <p class="eyebrow">Local inventory</p>
        <h2 style="margin-top: 8px;">Everything this node has pinned</h2>
        <p class="lead">Foundation-linked roots keep their rescue context. Each card now shows retry state, provider count, and action buttons to diagnose or retry a pin individually.</p>
      </div>
      <div style="margin-top: 20px;">{inventory_body}</div>
    </section>

    <section class="two-col">
      {gateway_card}
      {export_card}
    </section>

    <p class="footer">Agorix share bridge · local-only · {repair_interval}s repair interval · last cycle {last_repair}</p>
  </div>
</main>
<script>{live_status_script}</script>"##,
        conn_pill = connection_pill_class,
        conn_status = connection_status,
        pinned = pinned_count,
        managed = managed_count,
        repair_interval = repair_interval,
        last_repair = escape_html(&last_repair),
        disk_used = escape_html(&disk_used),
        disk_body = escape_html(&disk_body),
        flash = flash_block,
        failure_banner = failure_banner,
        live_status_block = live_status_block,
        connection = connection_block,
        session = session_block,
        artist_summary_html = artist_summary_html,
        upload_card = upload_card,
        inventory_body = inventory_body,
        gateway_card = gateway_card,
        export_card = export_card,
        live_status_script = LIVE_STATUS_SCRIPT,
    );

    Ok(Html(render_page("Foundation Share Bridge", &body)))
}
