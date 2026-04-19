//! Settings-page fragments: gateway-health card and export (backup) card.

use crate::model::config::BridgeConfig;
use crate::util::text::escape_html;
use crate::util::url::PUBLIC_UTILITY_GATEWAY_BASE_URL;

pub fn render_gateway_card(config: &BridgeConfig) -> String {
    format!(
        r#"<section id="gateway-card" class="card">
  <p class="eyebrow">Gateway health</p>
  <h2 style="margin-top:8px;">Are your gateways reachable?</h2>
  <p class="muted settings-copy">Confirms the local Kubo gateway, your configured external gateway, and the public fallback can all serve pinned content.</p>
  <dl class="kv" style="margin-top:12px;">
    <dt>Local</dt><dd><code>{local}</code> <span class="pill" id="gateway-check-local-pill">Idle</span></dd>
    <dt>External</dt><dd><code>{public}</code> <span class="pill" id="gateway-check-public-pill">Idle</span></dd>
    <dt>Fallback</dt><dd><code>{utility}</code> <span class="pill" id="gateway-check-utility-pill">Idle</span></dd>
  </dl>
  <div class="btn-row">
    <button type="button" class="btn ghost" id="gateway-check-run">Check gateways now</button>
  </div>
  <p class="muted inventory-status" id="gateway-check-status">Not yet tested in this session.</p>
</section>"#,
        local = escape_html(&config.local_gateway_base_url),
        public = escape_html(&config.public_gateway_base_url),
        utility = escape_html(PUBLIC_UTILITY_GATEWAY_BASE_URL),
    )
}

pub fn render_export_card() -> String {
    r#"<section id="export-card" class="card">
  <p class="eyebrow">Backup</p>
  <h2 style="margin-top:8px;">Export your rescue list</h2>
  <p class="muted settings-copy">If this machine ever fails, keep an offline copy of what you are pinning. JSON is a complete restore payload; CSV is easier to skim in a spreadsheet.</p>
  <div class="btn-row">
    <a class="btn" href="/pins/export?format=json" download>Download JSON</a>
    <a class="btn ghost" href="/pins/export?format=csv" download>Download CSV</a>
  </div>
</section>"#
        .to_string()
}
