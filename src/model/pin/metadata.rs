//! Pure metadata helpers — extracts display fields, attribute lists, URL
//! candidates, and media-kind hints from arbitrary [`serde_json::Value`]
//! payloads fetched from IPFS.

use std::collections::HashSet;

use super::types::{PinMetadataField, PinMetadataView};
use crate::util::data::{first_present_string, json_display_value, json_string, nested_json_value};

pub fn collect_url_candidates(value: Option<&serde_json::Value>) -> Vec<String> {
    let mut candidates = Vec::new();
    let entries = match value {
        Some(serde_json::Value::Array(items)) => items.iter().collect::<Vec<_>>(),
        Some(other) => vec![other],
        None => Vec::new(),
    };

    for entry in entries {
        let Some(record) = entry.as_object() else {
            continue;
        };

        for key in ["uri", "url", "src", "href", "animation_url", "animation", "image", "image_url"]
        {
            let candidate = json_string(record.get(key));
            if let Some(candidate) = candidate.filter(|value| !candidates.contains(value)) {
                candidates.push(candidate);
            }
        }
    }

    candidates
}

pub fn metadata_image_url(metadata: &serde_json::Value) -> Option<String> {
    first_present_string([
        json_string(metadata.get("image")),
        json_string(metadata.get("image_url")),
        json_string(nested_json_value(metadata, &["properties", "image"])),
        json_string(nested_json_value(metadata, &["properties", "image_url"])),
        json_string(nested_json_value(metadata, &["displayUri"])),
        json_string(nested_json_value(metadata, &["display_uri"])),
        json_string(nested_json_value(metadata, &["thumbnailUri"])),
        json_string(nested_json_value(metadata, &["thumbnail_uri"])),
    ])
}

pub fn metadata_file_url(metadata: &serde_json::Value) -> Option<String> {
    first_present_string(
        collect_url_candidates(nested_json_value(metadata, &["media", "files"]))
            .into_iter()
            .map(Some)
            .chain(
                collect_url_candidates(nested_json_value(metadata, &["properties", "files"]))
                    .into_iter()
                    .map(Some),
            )
            .chain(
                collect_url_candidates(nested_json_value(metadata, &["files"]))
                    .into_iter()
                    .map(Some),
            )
            .chain(
                collect_url_candidates(nested_json_value(metadata, &["formats"]))
                    .into_iter()
                    .map(Some),
            ),
    )
}

pub fn metadata_primary_media_url(metadata: &serde_json::Value) -> Option<String> {
    first_present_string([
        json_string(metadata.get("animation_url")),
        json_string(metadata.get("animation")),
        json_string(nested_json_value(metadata, &["media", "uri"])),
        json_string(nested_json_value(metadata, &["media", "url"])),
        json_string(nested_json_value(metadata, &["properties", "animation_url"])),
        json_string(nested_json_value(metadata, &["properties", "animation"])),
        json_string(nested_json_value(metadata, &["artifactUri"])),
        json_string(nested_json_value(metadata, &["artifact_uri"])),
        json_string(nested_json_value(metadata, &["content", "uri"])),
        json_string(nested_json_value(metadata, &["content", "url"])),
    ])
}

pub fn metadata_description(metadata: &serde_json::Value) -> Option<String> {
    first_present_string([
        json_string(metadata.get("description")),
        json_string(nested_json_value(metadata, &["properties", "description"])),
        json_string(nested_json_value(metadata, &["content", "description"])),
    ])
}

pub fn metadata_external_url(metadata: &serde_json::Value) -> Option<String> {
    first_present_string([
        json_string(metadata.get("external_url")),
        json_string(metadata.get("externalUrl")),
        json_string(nested_json_value(metadata, &["properties", "external_url"])),
        json_string(nested_json_value(metadata, &["properties", "externalUrl"])),
    ])
}

pub fn metadata_mime_type(metadata: &serde_json::Value) -> Option<String> {
    first_present_string([
        json_string(metadata.get("mimeType")),
        json_string(metadata.get("mime_type")),
        json_string(nested_json_value(metadata, &["content", "mimeType"])),
        json_string(nested_json_value(metadata, &["content", "mime_type"])),
        json_string(nested_json_value(metadata, &["properties", "mimeType"])),
        json_string(nested_json_value(metadata, &["properties", "mime_type"])),
    ])
}

pub fn build_metadata_summary_fields(
    metadata: &serde_json::Value,
    image_raw: Option<&str>,
    media_raw: Option<&str>,
) -> Vec<PinMetadataField> {
    let mut fields = Vec::new();
    let mut seen = HashSet::new();

    let mut push_field = |label: &str, value: Option<String>| {
        let Some(value) = value.filter(|entry| !entry.trim().is_empty()) else {
            return;
        };
        let dedupe_key = format!("{}:{}", label.to_ascii_lowercase(), value);
        if seen.insert(dedupe_key) {
            fields.push(PinMetadataField { label: label.to_string(), value });
        }
    };

    push_field("Metadata title", json_string(metadata.get("name")));
    push_field("External URL", metadata_external_url(metadata));
    push_field("Preview image", image_raw.map(ToOwned::to_owned));
    push_field(
        "Primary media",
        media_raw.filter(|entry| Some(*entry) != image_raw).map(ToOwned::to_owned),
    );
    push_field("Mime type", metadata_mime_type(metadata));

    fields
}

pub fn build_metadata_attribute_fields(metadata: &serde_json::Value) -> Vec<PinMetadataField> {
    let mut attributes = Vec::new();
    let mut seen = HashSet::new();

    for candidate in [
        metadata.get("attributes"),
        nested_json_value(metadata, &["properties", "attributes"]),
        metadata.get("traits"),
    ]
    .into_iter()
    .flatten()
    {
        let Some(entries) = candidate.as_array() else {
            continue;
        };

        for (index, entry) in entries.iter().enumerate() {
            let Some(record) = entry.as_object() else {
                continue;
            };

            let label = first_present_string([
                json_string(record.get("trait_type")),
                json_string(record.get("type")),
                json_string(record.get("name")),
                json_string(record.get("key")),
            ])
            .unwrap_or_else(|| format!("Trait {}", index + 1));

            let value = first_present_string([
                json_display_value(record.get("value")),
                json_display_value(record.get("display_value")),
                json_display_value(record.get("trait_value")),
            ]);
            let Some(value) = value.filter(|entry| !entry.trim().is_empty()) else {
                continue;
            };

            let dedupe_key = format!("{}:{}", label.to_ascii_lowercase(), value);
            if seen.insert(dedupe_key) {
                attributes.push(PinMetadataField { label, value });
            }
        }
    }

    attributes
}

pub fn build_metadata_view(
    metadata: &serde_json::Value,
    image_raw: Option<&str>,
    media_raw: Option<&str>,
) -> Option<PinMetadataView> {
    const MAX_METADATA_JSON_CHARS: usize = 24_000;

    let mut raw_json = serde_json::to_string_pretty(metadata).ok()?;
    let raw_json_truncated = if raw_json.chars().count() > MAX_METADATA_JSON_CHARS {
        raw_json = raw_json.chars().take(MAX_METADATA_JSON_CHARS).collect();
        raw_json.push_str("\n…");
        true
    } else {
        false
    };

    let description = metadata_description(metadata);
    let fields = build_metadata_summary_fields(metadata, image_raw, media_raw);
    let attributes = build_metadata_attribute_fields(metadata);

    if description.is_none() && fields.is_empty() && attributes.is_empty() && raw_json.is_empty() {
        return None;
    }

    Some(PinMetadataView { description, fields, attributes, raw_json, raw_json_truncated })
}

pub fn detect_media_kind_from_text(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let markers: [(&str, &[&str]); 6] = [
        ("VIDEO", &[".mp4", ".mov", ".webm", "video"]),
        ("AUDIO", &[".mp3", ".wav", ".ogg", ".aac", "audio"]),
        ("IMAGE", &[".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", "image"]),
        ("HTML", &[".html", "text/html"]),
        ("MODEL", &[".glb", ".gltf", ".usdz", "model", "model/gltf", "model/vnd.usdz", "3d"]),
        ("JSON", &[".json", "application/json", "text/json"]),
    ];

    markers.iter().find_map(|(kind, entries)| {
        entries.iter().any(|marker| lower.contains(marker)).then(|| (*kind).to_string())
    })
}
