//! Settings-page fragments: gateway-health card and export (backup) card.

use std::ops::Not;

use crate::model::config::BridgeConfig;
use crate::util::text::escape_html;
use crate::util::url::PUBLIC_UTILITY_GATEWAY_BASE_URL;

pub fn render_gateway_card(config: &BridgeConfig) -> String {
    let reach_note = render_reachability_note(config);
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
  {reach_note}
</section>"#,
        local = escape_html(&config.local_gateway_base_url),
        public = escape_html(&config.public_gateway_base_url),
        utility = escape_html(PUBLIC_UTILITY_GATEWAY_BASE_URL),
    )
}

/// When the tunnel is advertised as "public gateway" but no pinning service
/// is configured, spell out the NAT / libp2p story so users know why
/// ipfs.io still 504s for them. The tunnel serves HTTP via the bridge, but
/// the wider IPFS network still needs a libp2p-reachable provider — a
/// pinning service gives us that for free.
fn render_reachability_note(config: &BridgeConfig) -> String {
    if !config.tunnel_enabled {
        return String::new();
    }
    let token_configured =
        config.remote_pinning_access_token.as_deref().map_or("", str::trim).is_empty().not();
    if config.remote_pinning_enabled && token_configured {
        return format!(
            r#"<div class="flash ok" style="margin-top:14px;">Remote replication on via {service}. New pins are pushed there immediately so ipfs.io / dweb.link can resolve them through that provider.</div>"#,
            service = escape_html(
                config.remote_pinning_service_name.as_deref().unwrap_or("the configured service"),
            ),
        );
    }
    r#"<div class="flash warn" style="margin-top:14px;">
  <strong>Tunnel is on but public IPFS can&apos;t find your pins.</strong>
  <p style="margin:6px 0 0;">The tunnel only exposes this node&apos;s HTTP gateway — libp2p / bitswap still sit behind NAT, so ipfs.io and dweb.link see your DHT announcement but can&apos;t dial through. Set a remote pinning service (Pinata, Storacha, Filebase\u2026) in the section below and each new pin will replicate there automatically. That gives your content a publicly-dialable provider.</p>
</div>"#
        .to_string()
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
