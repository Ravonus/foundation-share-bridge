//! HTML handlers for the `/share/work` confirm + post-submit pages.
#![allow(clippy::too_many_lines, clippy::cognitive_complexity, clippy::pedantic, clippy::nursery)]

use axum::{
    Form,
    extract::{Query, State},
    response::Html,
};

use crate::{
    AppError, AppState,
    html::render::page::render_page,
    model::{
        config::ShareWorkViewQuery,
        relay::{ShareWorkRequest, service::share_work_inner},
    },
    util::text::escape_html,
};

pub async fn share_work_view(
    Query(query): Query<ShareWorkViewQuery>,
) -> Result<Html<String>, AppError> {
    let mut detail_rows = String::new();
    if let Some(cid) = query.metadata_cid.as_deref().filter(|cid| !cid.is_empty()) {
        detail_rows.push_str(&format!(
            r#"<li><span class="muted">Metadata</span><code>{}</code></li>"#,
            escape_html(cid)
        ));
    }
    if let Some(cid) = query.media_cid.as_deref().filter(|cid| !cid.is_empty()) {
        detail_rows.push_str(&format!(
            r#"<li><span class="muted">Media</span><code>{}</code></li>"#,
            escape_html(cid)
        ));
    }

    let details_block = if detail_rows.is_empty() {
        String::new()
    } else {
        format!(r#"<ul class="plain" style="margin-top: 16px;">{}</ul>"#, detail_rows)
    };

    let artist = query
        .artist_username
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown artist");

    let body = format!(
        r#"<main class="shell narrow">
  <div class="stack">
    <section class="section-head">
      <p class="eyebrow">Rescue handoff</p>
      <h1>Pin this rescued Foundation work.</h1>
      <p class="lead">Once you confirm, the bridge pins the rescued roots now and keeps watching them for self-repair later.</p>
    </section>

    <section class="card">
      <p class="eyebrow">Work</p>
      <h2 style="margin-top: 8px;">{title}</h2>
      <p class="muted" style="margin-top: 8px;">{artist} · token #{token}</p>
      {details}

      <form method="post" action="/share/work/form" class="btn-row" style="margin-top: 24px;">
        <input type="hidden" name="session_secret" value="{secret}" />
        <input type="hidden" name="title" value="{title_h}" />
        <input type="hidden" name="contract_address" value="{contract}" />
        <input type="hidden" name="token_id" value="{token_h}" />
        <input type="hidden" name="foundation_url" value="{foundation}" />
        <input type="hidden" name="artist_username" value="{artist_h}" />
        <input type="hidden" name="metadata_cid" value="{meta}" />
        <input type="hidden" name="media_cid" value="{media}" />
        <button type="submit" class="btn">Pin and keep watching forever</button>
        <a class="btn ghost" href="/">Cancel</a>
      </form>
    </section>
  </div>
</main>
<style>
  ul.plain li {{ display: grid; grid-template-columns: 120px 1fr; align-items: center; gap: 16px; }}
  ul.plain li .muted {{
    font-family: ui-monospace, Menlo, Consolas, monospace;
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.22em;
  }}
</style>"#,
        title = escape_html(&query.title),
        artist = escape_html(artist),
        token = escape_html(&query.token_id),
        details = details_block,
        secret = escape_html(&query.session_secret),
        title_h = escape_html(&query.title),
        contract = escape_html(&query.contract_address),
        token_h = escape_html(&query.token_id),
        foundation = escape_html(query.foundation_url.as_deref().unwrap_or("")),
        artist_h = escape_html(query.artist_username.as_deref().unwrap_or("")),
        meta = escape_html(query.metadata_cid.as_deref().unwrap_or("")),
        media = escape_html(query.media_cid.as_deref().unwrap_or("")),
    );

    Ok(Html(render_page("Pin rescued work", &body)))
}

pub async fn share_work_form(
    State(state): State<AppState>,
    Form(input): Form<ShareWorkRequest>,
) -> Result<Html<String>, AppError> {
    let response = share_work_inner(&state, input).await?;
    let pin_rows = response
        .pins
        .iter()
        .map(|pin| {
            format!(
                r#"<li><span class="muted">{}</span><code>{}</code></li>"#,
                escape_html(pin.label.as_deref().unwrap_or("pin")),
                escape_html(&pin.cid)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let pins_block = if pin_rows.is_empty() {
        String::new()
    } else {
        format!(r#"<ul class="plain" style="margin-top: 16px;">{}</ul>"#, pin_rows)
    };

    let body = format!(
        r#"<main class="shell narrow">
  <div class="stack">
    <section class="section-head">
      <p class="eyebrow">Pinned</p>
      <h1>{title}</h1>
      <p class="lead">{message}</p>
    </section>

    <section class="card">
      <p class="eyebrow">Watched roots</p>
      <h2 style="margin-top: 8px;">Now part of the forever list</h2>
      <p class="muted" style="margin-top: 10px;">The bridge will keep checking these on every repair cycle and re-pin them if they ever disappear.</p>
      {pins}
      <div class="btn-row">
        <a class="btn ghost" href="/">Back to bridge home</a>
      </div>
    </section>
  </div>
</main>
<style>
  ul.plain li {{ display: grid; grid-template-columns: 120px 1fr; align-items: center; gap: 16px; }}
  ul.plain li .muted {{
    font-family: ui-monospace, Menlo, Consolas, monospace;
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.22em;
  }}
</style>"#,
        title = escape_html(&response.title),
        message = escape_html(response.message),
        pins = pins_block,
    );

    Ok(Html(render_page("Pinned", &body)))
}
