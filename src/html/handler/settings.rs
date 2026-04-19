//! HTML handler for the bridge settings (`GET /settings`) page.
#![allow(clippy::too_many_lines, clippy::cognitive_complexity, clippy::pedantic, clippy::nursery)]

use axum::{
    extract::{Query, State},
    response::Html,
};

use crate::{
    AppError, AppState,
    html::{
        render::page::render_page,
        scripts::settings::{SETTINGS_CONTROLS_SCRIPT, SETTINGS_GATEWAY_HELPER_SCRIPT},
        styles::settings::SETTINGS_PAGE_STYLE,
    },
    model::{
        config::SettingsPageQuery, relay::relay_is_connected, system::probe::detect_public_ipv4,
    },
    util::{text::escape_html, url::build_direct_ip_gateway_base_url},
};

pub async fn settings_page(
    State(state): State<AppState>,
    Query(query): Query<SettingsPageQuery>,
) -> Result<Html<String>, AppError> {
    let config = state.config.read().await.clone();
    let relay_connected = relay_is_connected(&config);
    let relay_status_label = if relay_connected {
        "Connected"
    } else if config.relay_enabled {
        "Waiting to link"
    } else {
        "Not linked"
    };
    let relay_status_class = if relay_connected { "pill ok" } else { "pill" };
    let sync_checked = if config.sync_enabled { "checked" } else { "" };
    let relay_checked = if config.relay_enabled { "checked" } else { "" };
    let tunnel_checked = if config.tunnel_enabled { "checked" } else { "" };
    let remote_pinning_checked = if config.remote_pinning_enabled { "checked" } else { "" };
    let tunnel_hostname_line = config
        .tunnel_hostname
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|host| {
            format!(
                r#"<div class="settings-field-note"><strong>Public URL:</strong> <a href="https://{host}" target="_blank" rel="noreferrer">https://{host}</a></div>"#,
                host = escape_html(host)
            )
        })
        .unwrap_or_default();
    let tunnel_error_line = config
        .tunnel_last_error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|message| format!(r#"<div class="flash warn">{}</div>"#, escape_html(message)))
        .unwrap_or_default();
    let storage_quota_display =
        config.storage_quota_gb.map(|value| format!("{value}")).unwrap_or_default();
    let max_retry_attempts_display =
        config.max_retry_attempts.map(|value| format!("{value}")).unwrap_or_default();
    let remote_pinning_service_name_display =
        config.remote_pinning_service_name.clone().unwrap_or_default();
    let remote_pinning_service_url_display =
        config.remote_pinning_service_url.clone().unwrap_or_default();
    let token_saved = config
        .remote_pinning_access_token
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let (token_badge, token_placeholder) = if token_saved {
        (
            r#"<span class="token-badge saved" title="A token is saved">saved</span>"#,
            "•••••••• leave blank to keep",
        )
    } else {
        (r#"<span class="token-badge empty" title="No token saved">empty</span>"#, "Paste token")
    };

    let flash_block = if query.saved.as_deref() == Some("1") {
        r#"<div class="flash ok">Settings saved. The helper updated its YAML config file for you.</div>"#
            .to_string()
    } else if let Some(error) = query.error.as_deref() {
        format!(r#"<div class="flash err">{}</div>"#, escape_html(error))
    } else {
        String::new()
    };

    let relay_note = config
        .relay_last_error
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|message| {
            format!(r#"<div class="flash warn">Relay note: {}</div>"#, escape_html(message))
        })
        .unwrap_or_default();

    let detected_public_ipv4 = detect_public_ipv4(&state).await;
    let current_external_gateway = escape_html(&config.public_gateway_base_url);

    let gateway_helper = {
        let detected_ip_line = detected_public_ipv4
            .as_deref()
            .map(|ip| {
                format!(
                    r#"<div class="gw-detected">Detected public IP <code>{ip}</code> · <button type="button" class="gw-link" id="gateway_fill_ip" data-gateway-url="{url}">Use IP directly</button></div>"#,
                    ip = escape_html(ip),
                    url = escape_html(&build_direct_ip_gateway_base_url(ip)),
                )
            })
            .unwrap_or_else(|| r#"<div class="gw-detected muted">Public IPv4 not detected.</div>"#.to_string());

        format!(
            r#"<details class="gw-helper">
  <summary>Help me build this URL</summary>
  <div class="gw-helper-body">
    <div class="gw-row">
      <input type="text" id="gateway_hostname_input" placeholder="ipfs.example.com" />
      <button type="button" class="btn ghost" id="gateway_fill_hostname">Use hostname</button>
    </div>
    {detected_ip_line}
    <div class="gw-preview">Preview · <code id="gateway_helper_preview_value">{current_external_gateway}</code></div>
  </div>
</details>"#,
            detected_ip_line = detected_ip_line,
            current_external_gateway = current_external_gateway,
        )
    };

    let body = format!(
        r#"<main class="shell narrow settings-shell">
  <div class="stack">
    <header class="settings-head">
      <div>
        <p class="eyebrow">Settings</p>
        <h1>Bridge preferences</h1>
      </div>
      <div class="settings-head-meta">
        <span class="{relay_class}">{relay_label}</span>
        <a class="btn ghost" href="/">← Back</a>
      </div>
    </header>

    {flash}
    {relay_note}

    <form action="/settings/form" method="post" class="settings-form-v2" id="settings-form-v2">
      <section class="settings-card">
        <h2>Storage</h2>
        <div class="settings-field">
          <label for="field_download_root_dir">Download folder</label>
          <input type="text" id="field_download_root_dir" name="download_root_dir" value="{download_root_dir}" placeholder="/Users/you/Archive Pins" spellcheck="false" />
        </div>
        <div class="settings-row">
          <div class="settings-row-text">
            <strong>Keep synced copies on disk</strong>
            <span>Mirror each pin into the download folder.</span>
          </div>
          <label class="toggle" aria-label="Keep synced copies on disk">
            <input type="checkbox" name="sync_enabled" value="1" {sync_checked} />
            <span class="toggle-track"><span class="toggle-thumb"></span></span>
          </label>
        </div>
        <div class="settings-pair">
          <div class="settings-field">
            <label for="field_storage_quota_gb">Quota (GB)</label>
            <div class="num-stepper">
              <button type="button" data-step="-1" aria-label="Decrease">−</button>
              <input type="number" id="field_storage_quota_gb" step="0.1" min="0" name="storage_quota_gb" value="{storage_quota_gb}" placeholder="none" inputmode="decimal" />
              <button type="button" data-step="1" aria-label="Increase">+</button>
            </div>
          </div>
          <div class="settings-field">
            <label for="field_max_retry_attempts">Max retries</label>
            <div class="num-stepper">
              <button type="button" data-step="-1" aria-label="Decrease">−</button>
              <input type="number" id="field_max_retry_attempts" step="1" min="1" max="20" name="max_retry_attempts" value="{max_retry_attempts}" placeholder="10" inputmode="numeric" />
              <button type="button" data-step="1" aria-label="Increase">+</button>
            </div>
          </div>
        </div>
      </section>

      <section class="settings-card">
        <h2>Gateways</h2>
        <div class="settings-field">
          <label for="field_local_gateway_base_url">Local gateway</label>
          <input type="url" id="field_local_gateway_base_url" name="local_gateway_base_url" value="{local_gateway_base_url}" placeholder="http://127.0.0.1:8080" spellcheck="false" />
        </div>
        <div class="settings-field">
          <label for="public_gateway_base_url">External gateway</label>
          <input type="url" id="public_gateway_base_url" name="public_gateway_base_url" value="{public_gateway_base_url}" placeholder="https://ipfs.example.com" spellcheck="false" />
        </div>
        {gateway_helper}
      </section>

      <section class="settings-card">
        <h2>Remote pin fallback</h2>
        <div class="settings-row">
          <div class="settings-row-text">
            <strong>Enable remote fallback</strong>
            <span>Used only after local retries are exhausted.</span>
          </div>
          <label class="toggle" aria-label="Enable remote pin fallback">
            <input type="checkbox" name="remote_pinning_enabled" value="1" {remote_pinning_checked} />
            <span class="toggle-track"><span class="toggle-thumb"></span></span>
          </label>
        </div>
        <div class="settings-pair">
          <div class="settings-field">
            <label for="field_remote_pinning_service_name">Service name</label>
            <input type="text" id="field_remote_pinning_service_name" name="remote_pinning_service_name" value="{remote_pinning_service_name}" placeholder="Pinata" spellcheck="false" />
          </div>
          <div class="settings-field">
            <label for="field_remote_pinning_service_url">API base URL</label>
            <input type="url" id="field_remote_pinning_service_url" name="remote_pinning_service_url" value="{remote_pinning_service_url}" placeholder="https://api.pinata.cloud/psa" spellcheck="false" />
          </div>
        </div>
        <div class="settings-field">
          <label for="field_remote_pinning_access_token">Access token {token_badge}</label>
          <div class="password-field">
            <input type="password" id="field_remote_pinning_access_token" name="remote_pinning_access_token" value="" placeholder="{token_placeholder}" autocomplete="off" spellcheck="false" />
            <button type="button" class="password-reveal" data-reveal>Show</button>
          </div>
        </div>
      </section>

      <section class="settings-card">
        <h2>Public gateway (Cloudflare tunnel)</h2>
        <div class="settings-row">
          <div class="settings-row-text">
            <strong>Enable public gateway</strong>
            <span>Publishes this desktop&apos;s IPFS gateway at a unique HTTPS subdomain via Cloudflare Tunnel.</span>
          </div>
          <label class="toggle" aria-label="Enable public gateway">
            <input type="checkbox" name="tunnel_enabled" value="1" {tunnel_checked} />
            <span class="toggle-track"><span class="toggle-thumb"></span></span>
          </label>
        </div>
        {tunnel_hostname_line}
        {tunnel_error_line}
      </section>

      <section class="settings-card">
        <h2>Archive relay</h2>
        <div class="settings-row">
          <div class="settings-row-text">
            <strong>Enable relay link</strong>
            <span>Lets the archive site hand work to this helper.</span>
          </div>
          <label class="toggle" aria-label="Enable archive relay link">
            <input type="checkbox" name="relay_enabled" value="1" {relay_checked} />
            <span class="toggle-track"><span class="toggle-thumb"></span></span>
          </label>
        </div>
        <div class="settings-pair">
          <div class="settings-field">
            <label for="field_relay_server_url">Archive server URL</label>
            <input type="url" id="field_relay_server_url" name="relay_server_url" value="{relay_server_url}" placeholder="https://foundation.agorix.io" spellcheck="false" />
          </div>
          <div class="settings-field">
            <label for="field_relay_device_name">Desktop name</label>
            <input type="text" id="field_relay_device_name" name="relay_device_name" value="{relay_device_name}" placeholder="Studio MacBook" />
          </div>
        </div>
      </section>

      <div class="settings-save-bar" id="settings-save-bar">
        <span class="settings-save-hint" id="settings-save-hint">All changes saved.</span>
        <button type="submit" class="btn">Save settings</button>
      </div>
    </form>
  </div>
</main>
<style>{settings_css}</style>
<script>{settings_gateway_script}</script>
<script>{settings_controls_script}</script>"#,
        relay_class = relay_status_class,
        relay_label = escape_html(relay_status_label),
        flash = flash_block,
        relay_note = relay_note,
        download_root_dir = escape_html(&config.download_root_dir),
        sync_checked = sync_checked,
        storage_quota_gb = escape_html(&storage_quota_display),
        max_retry_attempts = escape_html(&max_retry_attempts_display),
        remote_pinning_checked = remote_pinning_checked,
        remote_pinning_service_name = escape_html(&remote_pinning_service_name_display),
        remote_pinning_service_url = escape_html(&remote_pinning_service_url_display),
        token_badge = token_badge,
        token_placeholder = token_placeholder,
        local_gateway_base_url = escape_html(&config.local_gateway_base_url),
        public_gateway_base_url = escape_html(&config.public_gateway_base_url),
        relay_checked = relay_checked,
        relay_server_url = escape_html(&config.relay_server_url),
        relay_device_name = escape_html(&config.relay_device_name),
        tunnel_checked = tunnel_checked,
        tunnel_hostname_line = tunnel_hostname_line,
        tunnel_error_line = tunnel_error_line,
        gateway_helper = gateway_helper,
        settings_css = SETTINGS_PAGE_STYLE,
        settings_gateway_script = SETTINGS_GATEWAY_HELPER_SCRIPT,
        settings_controls_script = SETTINGS_CONTROLS_SCRIPT,
    );

    Ok(Html(render_page("Bridge settings", &body)))
}
