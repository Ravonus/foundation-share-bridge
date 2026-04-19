//! String-escaping and tag-sanitization helpers for the various text formats
//! the bridge emits: HTML pages, CSV exports, platform notifications, and
//! user-supplied custom tags.

pub fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Validate that a string looks like a CID: non-empty, reasonable length,
/// alphanumeric only. Stricter than multibase (which allows a few symbols),
/// but that's fine — every CID produced by modern Kubo matches this.
pub fn is_valid_cid(raw: &str) -> bool {
    let trimmed = raw.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 128
        && trimmed.chars().all(|c| c.is_ascii_alphanumeric())
}

pub fn sanitize_custom_tag(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 48 {
        return None;
    }
    let cleaned: String = trimmed.chars().filter(|c| !c.is_control()).collect();
    if cleaned.is_empty() { None } else { Some(cleaned) }
}

#[cfg(target_os = "macos")]
pub fn escape_notification_text(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " ")
}

#[cfg(target_os = "windows")]
pub fn escape_xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Defang user-supplied text for `notify-send` on Linux. libnotify does not
/// interpret body markup unless the `body-markup` hint is set, and `notify-send`
/// passes args as argv (not a shell), so there is no shell-injection surface.
/// This helper still strips the three markup-relevant characters and any
/// control characters so a pathological title renders cleanly on themes that
/// do enable markup.
#[cfg(target_os = "linux")]
pub fn escape_linux_notification_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .filter(|c| !matches!(c, '<' | '>' | '&'))
        .collect()
}
