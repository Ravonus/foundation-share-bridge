//! Filesystem / filename sanitization helpers.

use std::path::Path;

use crate::util::url::parse_ipfs_path;

pub fn sanitize_file_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_matches('/');
    let file_name = trimmed.rsplit('/').next().unwrap_or(trimmed);
    let cleaned = file_name
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            value => value,
        })
        .collect::<String>();
    let sanitized = cleaned.trim().trim_matches('.').to_string();
    (!sanitized.is_empty()).then_some(sanitized)
}

pub fn ensure_leaf_file_extension(file_name: &str, extension: &str) -> String {
    if Path::new(file_name).extension().is_some() {
        file_name.to_string()
    } else {
        format!("{}.{}", file_name.trim_end_matches('.'), extension)
    }
}

pub fn sniff_leaf_file_extension(bytes: &[u8]) -> Option<&'static str> {
    let lower_prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).to_ascii_lowercase();
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else if bytes.starts_with(b"OggS") {
        Some("ogg")
    } else if bytes.starts_with(b"ID3") {
        Some("mp3")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        Some("wav")
    } else if bytes.get(4..8) == Some(b"ftyp") {
        Some("mp4")
    } else if bytes.starts_with(b"glTF") {
        Some("glb")
    } else if lower_prefix.contains("<svg") {
        Some("svg")
    } else if lower_prefix.contains("<html") || lower_prefix.contains("<!doctype html") {
        Some("html")
    } else if lower_prefix.trim_start().starts_with('{')
        || lower_prefix.trim_start().starts_with('[')
    {
        Some("json")
    } else {
        None
    }
}

pub fn preferred_file_name_from_relative_path(relative_path: &str) -> Option<String> {
    sanitize_file_name(relative_path)
}

pub fn leaf_name_from_ipfs_path(ipfs_path: &str) -> Option<String> {
    let (_, relative_path) = parse_ipfs_path(ipfs_path)?;
    preferred_file_name_from_relative_path(&relative_path)
}
