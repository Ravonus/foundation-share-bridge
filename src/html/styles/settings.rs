//! Settings-page-specific stylesheet constants.

#[allow(clippy::needless_raw_string_hashes)]
pub const SETTINGS_PAGE_STYLE: &str = r####"
.settings-shell { padding-top: 32px; padding-bottom: 80px; max-width: 720px; }
.settings-head { display: flex; align-items: flex-end; justify-content: space-between; gap: 20px; margin-bottom: 28px; padding-bottom: 20px; border-bottom: 1px solid var(--line); flex-wrap: wrap; }
.settings-head h1 { font-size: clamp(1.75rem, 3vw, 2.1rem); margin-top: 4px; }
.settings-head-meta { display: inline-flex; gap: 10px; align-items: center; }
.settings-form-v2 { display: grid; gap: 18px; }
.settings-card { background: var(--surface); border: 1px solid var(--line); border-radius: 12px; padding: 22px 24px; display: grid; gap: 18px; }
.settings-card h2 { font-size: 1.1rem; font-family: var(--font-fraunces), ui-serif, Georgia, serif; font-weight: 500; color: var(--ink); letter-spacing: -0.01em; padding-bottom: 8px; border-bottom: 1px solid var(--line); margin: 0; }
.settings-field { display: grid; gap: 6px; }
.settings-field label { font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; font-size: 0.68rem; letter-spacing: 0.22em; text-transform: uppercase; color: var(--muted); }
.settings-field input[type="text"],
.settings-field input[type="url"],
.settings-field input[type="number"],
.settings-field input[type="password"] { width: 100%; padding: 11px 13px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface-quiet); color: var(--ink); font: inherit; font-size: 0.92rem; transition: border-color 140ms ease, background 140ms ease, box-shadow 140ms ease; }
.settings-field input[type="text"]:focus,
.settings-field input[type="url"]:focus,
.settings-field input[type="number"]:focus,
.settings-field input[type="password"]:focus { outline: none; border-color: var(--brand-green); background: var(--surface); box-shadow: 0 0 0 3px color-mix(in oklab, var(--brand-green) 22%, transparent); }
.settings-pair { display: grid; gap: 14px; grid-template-columns: 1fr; }
@media (min-width: 620px) { .settings-pair { grid-template-columns: 1fr 1fr; } }
.settings-row { display: flex; align-items: center; justify-content: space-between; gap: 16px; padding: 6px 0; border-top: 1px solid var(--line); padding-top: 16px; }
.settings-row:first-of-type { border-top: 0; padding-top: 0; }
.settings-row-text strong { display: block; color: var(--ink); font-size: 0.96rem; font-weight: 500; }
.settings-row-text span { display: block; color: var(--muted); font-size: 0.82rem; margin-top: 2px; }
.toggle { position: relative; display: inline-flex; flex: 0 0 auto; cursor: pointer; user-select: none; }
.toggle input { position: absolute; opacity: 0; pointer-events: none; width: 0; height: 0; }
.toggle-track { display: inline-block; width: 46px; height: 26px; background: var(--line-strong); border-radius: 999px; position: relative; transition: background 180ms ease; }
.toggle-thumb { position: absolute; top: 3px; left: 3px; width: 20px; height: 20px; background: var(--surface); border-radius: 999px; box-shadow: 0 1px 3px rgba(0,0,0,0.22); transition: transform 200ms cubic-bezier(0.22, 1, 0.36, 1); }
.toggle input:checked + .toggle-track { background: var(--brand-green); }
.toggle input:checked + .toggle-track .toggle-thumb { transform: translateX(20px); }
.toggle input:focus-visible + .toggle-track { box-shadow: 0 0 0 3px color-mix(in oklab, var(--brand-green) 30%, transparent); }
.num-stepper { display: grid; grid-template-columns: 40px 1fr 40px; align-items: stretch; border: 1px solid var(--line-strong); border-radius: 8px; background: var(--surface-quiet); overflow: hidden; transition: border-color 140ms ease, box-shadow 140ms ease; }
.num-stepper:focus-within { border-color: var(--brand-green); box-shadow: 0 0 0 3px color-mix(in oklab, var(--brand-green) 22%, transparent); }
.num-stepper input { border: 0; background: transparent; text-align: center; padding: 11px 4px; font-variant-numeric: tabular-nums; appearance: textfield; -moz-appearance: textfield; }
.num-stepper input:focus { outline: none; box-shadow: none; border: 0; background: transparent; }
.num-stepper input::-webkit-outer-spin-button,
.num-stepper input::-webkit-inner-spin-button { -webkit-appearance: none; margin: 0; }
.num-stepper button { background: transparent; border: 0; color: var(--muted); font-size: 1.05rem; cursor: pointer; font-weight: 500; transition: color 140ms ease, background 140ms ease; }
.num-stepper button:hover { color: var(--ink); background: color-mix(in oklab, var(--ink) 6%, transparent); }
.num-stepper button:first-child { border-right: 1px solid var(--line); }
.num-stepper button:last-child { border-left: 1px solid var(--line); }
.password-field { position: relative; }
.password-field input { padding-right: 68px; font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; letter-spacing: 0.12em; }
.password-reveal { position: absolute; right: 6px; top: 50%; transform: translateY(-50%); background: transparent; border: 0; color: var(--muted); font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; font-size: 0.68rem; letter-spacing: 0.16em; text-transform: uppercase; cursor: pointer; padding: 6px 10px; border-radius: 5px; transition: color 140ms ease, background 140ms ease; }
.password-reveal:hover { color: var(--ink); background: color-mix(in oklab, var(--ink) 6%, transparent); }
.token-badge { display: inline-flex; padding: 2px 7px; border-radius: 999px; font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; font-size: 0.58rem; letter-spacing: 0.2em; text-transform: uppercase; vertical-align: middle; margin-left: 8px; }
.token-badge.saved { background: var(--tint-ok); color: var(--ok); }
.token-badge.empty { background: var(--tint-warn); color: var(--warn); }
.gw-helper { border: 1px dashed var(--line-strong); border-radius: 10px; padding: 0; background: var(--surface-quiet); overflow: hidden; }
.gw-helper > summary { cursor: pointer; list-style: none; color: var(--muted); font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; font-size: 0.72rem; letter-spacing: 0.18em; text-transform: uppercase; padding: 12px 16px; display: flex; align-items: center; gap: 10px; }
.gw-helper > summary::-webkit-details-marker { display: none; }
.gw-helper > summary::before { content: "+"; display: inline-flex; width: 18px; height: 18px; align-items: center; justify-content: center; border-radius: 999px; background: color-mix(in oklab, var(--ink) 8%, transparent); color: var(--ink); font-family: ui-monospace; transition: transform 180ms ease; }
.gw-helper[open] > summary::before { content: "−"; }
.gw-helper[open] > summary { color: var(--ink); border-bottom: 1px solid var(--line); }
.gw-helper-body { display: grid; gap: 12px; padding: 14px 16px; }
.gw-row { display: grid; grid-template-columns: 1fr auto; gap: 10px; align-items: center; }
.gw-row input { width: 100%; padding: 10px 12px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--ink); font: inherit; font-size: 0.9rem; }
.gw-row input:focus { outline: none; border-color: var(--brand-green); box-shadow: 0 0 0 3px color-mix(in oklab, var(--brand-green) 22%, transparent); }
.gw-detected { font-size: 0.84rem; color: var(--body); }
.gw-link { background: transparent; border: 0; padding: 0; color: var(--brand-green); text-decoration: underline; text-underline-offset: 3px; cursor: pointer; font: inherit; font-size: inherit; }
.gw-link:hover { color: var(--brand-green-bright); }
.gw-preview { font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; font-size: 0.74rem; letter-spacing: 0.08em; color: var(--muted); border-top: 1px dashed var(--line); padding-top: 10px; }
.gw-preview code { word-break: break-all; }
.settings-save-bar { display: flex; align-items: center; justify-content: flex-end; gap: 14px; padding: 12px 0 4px; position: sticky; bottom: 14px; background: color-mix(in oklab, var(--bg) 88%, transparent); -webkit-backdrop-filter: blur(10px); backdrop-filter: blur(10px); border-radius: 10px; padding-left: 16px; padding-right: 16px; border: 1px solid transparent; transition: border-color 180ms ease, box-shadow 180ms ease; }
.settings-save-bar.is-dirty { border-color: var(--line-strong); box-shadow: 0 10px 28px rgba(0,0,0,0.08); }
.settings-save-hint { color: var(--muted); font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; font-size: 0.72rem; letter-spacing: 0.16em; text-transform: uppercase; }
.settings-save-bar.is-dirty .settings-save-hint { color: var(--warn); }
.settings-save-bar .btn { min-width: 160px; justify-content: center; }
"####;
