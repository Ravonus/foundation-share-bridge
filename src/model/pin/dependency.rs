//! Pure IPFS-dependency discovery helpers — parse candidate references out of
//! JSON/text, deduplicate them, and assemble the input lists consumed by the
//! dependency-probe loop.

use std::collections::{HashSet, VecDeque};

use super::types::{DiscoveredDependency, WatchPinInput};
use crate::model::relay::RelayShareWorkPayload;
use crate::util::{file::preferred_file_name_from_relative_path, url::parse_ipfs_reference};

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
