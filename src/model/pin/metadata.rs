//! Pure metadata + dependency-discovery helpers — extracts display fields,
//! attribute lists, URL candidates, and media-kind hints from arbitrary
//! [`serde_json::Value`] payloads fetched from IPFS, and parses candidate
//! IPFS references out of JSON/text so the dependency-probe loop knows which
//! CIDs to walk next.

use std::collections::{HashSet, VecDeque};

use super::types::{DiscoveredDependency, PinMetadataField, PinMetadataView, WatchPinInput};
use crate::{
    model::relay::RelayShareWorkPayload,
    util::{
        data::{first_present_string, json_display_value, json_string, nested_json_value},
        file::preferred_file_name_from_relative_path,
        url::parse_ipfs_reference,
    },
};

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

pub fn parse_discovered_dependency(raw: &str) -> Option<DiscoveredDependency> {
    let (cid, relative_path) = parse_ipfs_reference(raw)?;
    Some(DiscoveredDependency {
        cid,
        preferred_file_name: preferred_file_name_from_relative_path(&relative_path),
    })
}

pub fn push_unique_dependency(
    dependencies: &mut Vec<DiscoveredDependency>,
    candidate: DiscoveredDependency,
) -> bool {
    if let Some(existing) = dependencies.iter_mut().find(|entry| entry.cid == candidate.cid) {
        if existing.preferred_file_name.is_none() {
            existing.preferred_file_name = candidate.preferred_file_name;
        }
        false
    } else {
        dependencies.push(candidate);
        true
    }
}

pub fn extract_absolute_ipfs_reference_strings(text: &str) -> Vec<DiscoveredDependency> {
    let mut dependencies = Vec::new();
    for token in text.split(|character: char| {
        character.is_whitespace()
            || matches!(
                character,
                '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '='
            )
    }) {
        let candidate = token.trim_matches(|character: char| {
            matches!(
                character,
                '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
            )
        });
        if let Some(dependency) = parse_discovered_dependency(candidate) {
            push_unique_dependency(&mut dependencies, dependency);
        }
    }
    dependencies
}

pub fn collect_dependency_refs_from_json_value(
    value: &serde_json::Value,
    dependencies: &mut Vec<DiscoveredDependency>,
) {
    match value {
        serde_json::Value::String(text) => {
            if let Some(dependency) = parse_discovered_dependency(text) {
                push_unique_dependency(dependencies, dependency);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_dependency_refs_from_json_value(item, dependencies);
            }
        }
        serde_json::Value::Object(entries) => {
            for item in entries.values() {
                collect_dependency_refs_from_json_value(item, dependencies);
            }
        }
        _ => {}
    }
}

pub fn is_dependency_probe_candidate(file_name: &str) -> bool {
    // Extension comparison is already case-insensitive because `lower` is
    // pre-lowercased above; the clippy lint that flags `ends_with(".html")` as
    // case-sensitive doesn't track that, so suppress it locally.
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    {
        let lower = file_name.trim().to_ascii_lowercase();
        lower.ends_with(".html")
            || lower.ends_with(".htm")
            || lower.ends_with(".gltf")
            || lower.ends_with(".json")
            || lower.ends_with(".svg")
            || lower.ends_with(".css")
            || lower.ends_with(".js")
            || lower.ends_with(".txt")
    }
}

pub fn build_work_dependency_input(
    input: &RelayShareWorkPayload,
    dependency: DiscoveredDependency,
) -> WatchPinInput {
    WatchPinInput {
        cid: dependency.cid,
        label: Some("dependency".to_string()),
        preferred_file_name: dependency.preferred_file_name,
        source_kind: "work".to_string(),
        title: Some(input.title.clone()),
        contract_address: Some(input.contract_address.clone()),
        token_id: Some(input.token_id.clone()),
        foundation_url: input.foundation_url.clone(),
        artist_username: input.artist_username.clone(),
        account_address: None,
        username: None,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn enqueue_dependency_probe(
    queued: &mut HashSet<String>,
    queue: &mut VecDeque<(String, Option<String>, usize)>,
    cid: String,
    preferred_file_name: Option<String>,
    depth: usize,
) {
    if queued.insert(cid.clone()) {
        queue.push_back((cid, preferred_file_name, depth));
    }
}
