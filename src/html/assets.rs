//! Inline SVG assets embedded in server-rendered HTML.

#[allow(clippy::needless_raw_string_hashes)]
pub const LOGO_MARK_SVG: &str = r##"<svg class="brand-mark" role="img" aria-label="Agorix mark" width="28" height="28" viewBox="0 0 64 64"><g fill="none" stroke="currentColor" stroke-width="3.2" stroke-linecap="square" style="color: var(--ink); opacity: 0.78"><path d="M6 18V6h12"/><path d="M58 18V6H46"/><path d="M6 46v12h12"/><path d="M58 46v12H46"/></g><path d="M32 16 C 32 24, 40 32, 48 32 C 40 32, 32 40, 32 48 C 32 40, 24 32, 16 32 C 24 32, 32 24, 32 16 Z" fill="var(--brand-green)"/></svg>"##;
