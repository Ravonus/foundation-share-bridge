//! Stateful IPFS-dependency walk — resolve probe paths from a CID, seed
//! file-name hints from on-chain metadata, and breadth-first scan linked
//! references that should be pinned alongside the top-level work.
//!
//! The pure parsing helpers (`parse_discovered_dependency`,
//! `extract_absolute_ipfs_reference_strings`, etc.) live in
//! `model::pin::metadata`; this module glues them to the live Kubo daemon.

use std::collections::{HashSet, VecDeque};

use crate::{
    AppState,
    model::{
        pin::{
            metadata::{
                build_work_dependency_input, collect_dependency_refs_from_json_value,
                enqueue_dependency_probe, extract_absolute_ipfs_reference_strings,
                is_dependency_probe_candidate, metadata_file_url, metadata_image_url,
                metadata_primary_media_url, push_unique_dependency,
            },
            types::WatchPinInput,
        },
        relay::RelayShareWorkPayload,
    },
    util::{file::preferred_file_name_from_relative_path, url::parse_ipfs_reference},
};

use super::kubo::{fetch_ipfs_json, fetch_ipfs_text, list_ipfs_links, resolve_single_child_path};

// A shallow breadth-first walk is enough to cover every real-world collection
// we've observed; deeper traversal risks pulling in sibling works and blows up
// the probe loop.
const MAX_DEPENDENCY_DISCOVERY_DEPTH: usize = 2;
const MAX_DEPENDENCY_SCAN_CIDS: usize = 24;

pub async fn resolve_dependency_probe_paths(
    state: &AppState,
    cid: &str,
    preferred_file_name: Option<&str>,
) -> Vec<String> {
    let root_path = format!("/ipfs/{}", cid.trim());
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    let mut push_candidate = |path: String| {
        if seen.insert(path.clone()) {
            candidates.push(path);
        }
    };

    if preferred_file_name.as_ref().is_some_and(|value| is_dependency_probe_candidate(value)) {
        push_candidate(root_path.clone());
    }

    match list_ipfs_links(state, &root_path).await {
        Ok(links) if links.is_empty() => {
            push_candidate(root_path);
        }
        Ok(links) => {
            for preferred_name in ["index.html", "metadata.json"] {
                if let Some(name) = links
                    .iter()
                    .filter_map(|link| link.get("Name").and_then(|value| value.as_str()))
                    .map(str::trim)
                    .find(|name| name.eq_ignore_ascii_case(preferred_name))
                {
                    push_candidate(format!("{root_path}/{name}"));
                }
            }

            for name in links
                .iter()
                .filter_map(|link| link.get("Name").and_then(|value| value.as_str()))
                .map(str::trim)
                .filter(|name| !name.is_empty() && is_dependency_probe_candidate(name))
            {
                push_candidate(format!("{root_path}/{name}"));
            }
        }
        Err(_) => {
            if preferred_file_name
                .as_ref()
                .is_some_and(|value| is_dependency_probe_candidate(value))
            {
                push_candidate(root_path);
            }
        }
    }

    candidates.truncate(6);
    candidates
}

pub async fn resolve_work_root_file_hints(
    state: &AppState,
    input: &RelayShareWorkPayload,
) -> (Option<String>, Option<String>) {
    // `Option::map_or_else` is awkward with the three-way (Some / token-id /
    // fallback) branch; leaving the nested `if let` keeps the intent readable.
    #[allow(clippy::option_if_let_else)]
    let metadata_hint = if let Some(metadata_cid) =
        input.metadata_cid.as_deref().filter(|cid| !cid.trim().is_empty())
    {
        if let Some(child) = resolve_single_child_path(state, metadata_cid, &[".json"]).await {
            preferred_file_name_from_relative_path(&child)
        } else if !input.token_id.trim().is_empty() {
            Some(format!("{}.json", input.token_id.trim()))
        } else {
            Some("metadata.json".to_string())
        }
    } else {
        None
    };

    let media_hint =
        if let Some(media_cid) = input.media_cid.as_deref().filter(|cid| !cid.trim().is_empty()) {
            let metadata = if let Some(metadata_cid) =
                input.metadata_cid.as_deref().filter(|cid| !cid.trim().is_empty())
            {
                load_work_metadata_record(state, metadata_cid, Some(&input.token_id)).await
            } else {
                None
            };

            let from_metadata = metadata.as_ref().and_then(|record| {
                [
                    metadata_primary_media_url(record),
                    metadata_file_url(record),
                    metadata_image_url(record),
                ]
                .into_iter()
                .flatten()
                .find_map(|raw| match parse_ipfs_reference(&raw) {
                    Some((candidate_cid, relative_path))
                        if candidate_cid.trim() == media_cid.trim() =>
                    {
                        preferred_file_name_from_relative_path(&relative_path)
                    }
                    _ => None,
                })
            });

            if from_metadata.is_some() {
                from_metadata
            } else {
                resolve_single_child_path(state, media_cid, &[])
                    .await
                    .and_then(|child| preferred_file_name_from_relative_path(&child))
            }
        } else {
            None
        };

    (metadata_hint, media_hint)
}

// Breadth-first dependency crawl — the state machine spans queue seeding,
// metadata scan, root-CID filtering, and per-level probing; splitting it would
// fragment a single logical algorithm.
#[allow(clippy::too_many_lines)]
pub async fn discover_work_dependency_inputs(
    state: &AppState,
    input: &RelayShareWorkPayload,
    metadata_hint: Option<String>,
    media_hint: Option<String>,
) -> Vec<WatchPinInput> {
    let mut dependencies = Vec::new();
    let mut queued = HashSet::new();
    let mut scanned = HashSet::new();
    let mut queue = VecDeque::<(String, Option<String>, usize)>::new();

    let root_cids = [
        input
            .metadata_cid
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        input
            .media_cid
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
    ]
    .into_iter()
    .flatten()
    .collect::<HashSet<_>>();

    if let Some(metadata_cid) =
        input.metadata_cid.as_deref().map(str::trim).filter(|value| !value.is_empty())
    {
        enqueue_dependency_probe(
            &mut queued,
            &mut queue,
            metadata_cid.to_string(),
            metadata_hint.clone(),
            0,
        );
        if let Some(metadata) =
            load_work_metadata_record(state, metadata_cid, Some(&input.token_id)).await
        {
            collect_dependency_refs_from_json_value(&metadata, &mut dependencies);
        }
    }

    if let Some(media_cid) =
        input.media_cid.as_deref().map(str::trim).filter(|value| !value.is_empty())
    {
        enqueue_dependency_probe(
            &mut queued,
            &mut queue,
            media_cid.to_string(),
            media_hint.clone(),
            0,
        );
    }

    for dependency in dependencies.clone() {
        if !root_cids.contains(&dependency.cid) {
            enqueue_dependency_probe(
                &mut queued,
                &mut queue,
                dependency.cid,
                dependency.preferred_file_name,
                1,
            );
        }
    }

    while let Some((cid, preferred_file_name, depth)) = queue.pop_front() {
        if scanned.len() >= MAX_DEPENDENCY_SCAN_CIDS || !scanned.insert(cid.clone()) {
            continue;
        }

        let probe_paths =
            resolve_dependency_probe_paths(state, &cid, preferred_file_name.as_deref()).await;
        for probe_path in probe_paths {
            let Ok(Some(text)) = fetch_ipfs_text(state, &probe_path).await else {
                continue;
            };
            for dependency in extract_absolute_ipfs_reference_strings(&text) {
                if root_cids.contains(&dependency.cid) {
                    continue;
                }

                let is_new = push_unique_dependency(&mut dependencies, dependency.clone());
                if depth < MAX_DEPENDENCY_DISCOVERY_DEPTH && is_new {
                    enqueue_dependency_probe(
                        &mut queued,
                        &mut queue,
                        dependency.cid,
                        dependency.preferred_file_name,
                        depth + 1,
                    );
                }
            }
        }
    }

    dependencies
        .into_iter()
        .filter(|dependency| !root_cids.contains(&dependency.cid))
        .map(|dependency| build_work_dependency_input(input, dependency))
        .collect()
}

/// Loads the NFT metadata JSON for a work, tolerating the several layouts we
/// see in practice: `<cid>/<token_id>.json`, `<cid>/metadata.json`, a single
/// `.json` child, or the CID itself pointing at the record.
pub async fn load_work_metadata_record(
    state: &AppState,
    metadata_cid: &str,
    token_id: Option<&str>,
) -> Option<serde_json::Value> {
    let mut candidates = Vec::new();
    let cid = metadata_cid.trim();

    if let Some(token_id) = token_id.map(str::trim).filter(|value| !value.is_empty()) {
        candidates.push(format!("/ipfs/{cid}/{token_id}.json"));
        candidates.push(format!("/ipfs/{cid}/{token_id}"));
    }

    candidates.push(format!("/ipfs/{cid}/metadata.json"));
    if let Some(single_json_child) = resolve_single_child_path(state, cid, &[".json"]).await {
        candidates.push(format!("/ipfs/{cid}/{single_json_child}"));
    }
    candidates.push(format!("/ipfs/{cid}"));

    let mut seen = HashSet::new();
    for candidate in candidates {
        if !seen.insert(candidate.clone()) {
            continue;
        }
        if let Ok(Some(metadata)) = fetch_ipfs_json(state, &candidate).await {
            return Some(metadata);
        }
    }

    None
}
