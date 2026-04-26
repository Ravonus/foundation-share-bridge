use anyhow::anyhow;
use axum::{Json, extract::Multipart, extract::State, response::Redirect};
use chrono::Utc;
use tracing::warn;

use crate::{
    AppError, AppState,
    model::{
        pin::{
            AddFilesResult, AddedFileEntry, WatchPinInput, client::sync::sync_cid_if_enabled,
            service::remember_watched_pin,
        },
        session::service::validate_session,
    },
    util::url::encode_query_component,
};

struct ParsedUpload {
    session_secret: Option<String>,
    label: Option<String>,
    files: Vec<(String, Vec<u8>)>,
    total_bytes: u64,
}

async fn parse_upload_multipart(mut multipart: Multipart) -> Result<ParsedUpload, AppError> {
    let mut session_secret: Option<String> = None;
    let mut label: Option<String> = None;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total_bytes: u64 = 0;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| AppError::bad_request(format!("Unable to read upload: {error}")))?
    {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "session_secret" => {
                let value = field.text().await.map_err(|error| {
                    AppError::bad_request(format!("Bad session_secret: {error}"))
                })?;
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    session_secret = Some(trimmed.to_string());
                }
            }
            "label" => {
                let value = field
                    .text()
                    .await
                    .map_err(|error| AppError::bad_request(format!("Bad label: {error}")))?;
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    label = Some(trimmed.to_string());
                }
            }
            "file" | "files" => {
                let filename = field
                    .file_name()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "file".to_string());
                let bytes = field.bytes().await.map_err(|error| {
                    AppError::bad_request(format!("Upload read failed: {error}"))
                })?;
                total_bytes = total_bytes.saturating_add(bytes.len() as u64);
                files.push((filename, bytes.to_vec()));
            }
            _ => {
                // Drain unknown fields so the body is fully consumed.
                let _ = field.bytes().await;
            }
        }
    }

    Ok(ParsedUpload { session_secret, label, files, total_bytes })
}

async fn ingest_uploaded_files(
    state: &AppState,
    label: Option<String>,
    mut files: Vec<(String, Vec<u8>)>,
    total_bytes: u64,
) -> Result<AddFilesResult, AppError> {
    if files.is_empty() {
        return Err(AppError::bad_request(
            "At least one file is required. Use form field name `file` or `files`.",
        ));
    }

    let wrap = files.len() > 1 || files.iter().any(|(name, _)| name.contains('/'));

    let mut form = reqwest::multipart::Form::new();
    for (filename, bytes) in files.drain(..) {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename)
            .mime_str("application/octet-stream")
            .map_err(|error| AppError::internal(anyhow!("Bad upload part: {error}")))?;
        form = form.part("file", part);
    }

    let endpoint = format!(
        "{}/api/v0/add?pin=true{}",
        state.ipfs_api_url.trim_end_matches('/'),
        if wrap { "&wrap-with-directory=true" } else { "" }
    );

    let mut request = state.http.post(endpoint);
    if let Some(header) = &state.ipfs_api_auth_header {
        request = request.header("Authorization", header);
    }

    let response = request
        .multipart(form)
        .send()
        .await
        .map_err(|error| AppError::internal(anyhow!("Failed to reach IPFS API: {error}")))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::internal(anyhow!(
            "IPFS add failed with status {}: {}",
            status,
            body
        )));
    }

    let body_text = response
        .text()
        .await
        .map_err(|error| AppError::internal(anyhow!("Unable to read IPFS response: {error}")))?;

    let mut entries: Vec<AddedFileEntry> = Vec::new();
    for line in body_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(trimmed).map_err(|error| {
            AppError::internal(anyhow!("IPFS returned malformed line: {error}"))
        })?;
        let name = value.get("Name").and_then(|value| value.as_str()).unwrap_or("").to_string();
        let cid = value.get("Hash").and_then(|value| value.as_str()).unwrap_or("").to_string();
        if cid.is_empty() {
            continue;
        }
        let size = value
            .get("Size")
            .and_then(|value| value.as_str())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        entries.push(AddedFileEntry { name, cid, size });
    }

    if entries.is_empty() {
        return Err(AppError::internal(anyhow!("IPFS add returned no entries")));
    }

    let root_cid = if wrap {
        entries
            .iter()
            .find(|entry| entry.name.is_empty())
            .map(|entry| entry.cid.clone())
            .unwrap_or_else(|| entries.last().map(|entry| entry.cid.clone()).unwrap_or_default())
    } else {
        entries.last().map(|entry| entry.cid.clone()).unwrap_or_default()
    };

    if root_cid.is_empty() {
        return Err(AppError::internal(anyhow!("IPFS add did not return a root CID")));
    }

    let file_count = entries.iter().filter(|entry| !entry.name.is_empty()).count();
    let file_count = if file_count == 0 { entries.len() } else { file_count };

    let derived_label = label.clone().or_else(|| {
        if wrap {
            entries.iter().find(|entry| entry.name.is_empty()).and_then(|entry| {
                entries.iter().find(|inner| !inner.name.is_empty()).map(|inner| {
                    inner.name.split('/').next().unwrap_or(entry.cid.as_str()).to_string()
                })
            })
        } else {
            entries.iter().find(|entry| !entry.name.is_empty()).map(|entry| entry.name.clone())
        }
    });

    let preferred_file_name = if !wrap {
        entries.iter().find(|entry| !entry.name.is_empty()).map(|entry| entry.name.clone())
    } else {
        None
    };

    remember_watched_pin(
        state,
        WatchPinInput {
            cid: root_cid.clone(),
            label: derived_label.clone(),
            preferred_file_name,
            source_kind: "upload".to_string(),
            title: None,
            contract_address: None,
            token_id: None,
            foundation_url: None,
            artist_username: None,
            account_address: None,
            username: None,
        },
        Some(root_cid.clone()),
        None,
        true,
    )
    .await?;

    if let Err(error) = sync_cid_if_enabled(state, &root_cid).await {
        warn!("sync after upload failed for {}: {}", root_cid, error);
    }

    Ok(AddFilesResult {
        root_cid: root_cid.clone(),
        label: derived_label,
        pinned: true,
        provider: "kubo",
        pin_reference: root_cid,
        requested_at: Utc::now(),
        file_count,
        total_bytes,
        wrapped: wrap,
        entries,
    })
}

pub async fn add_files(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Json<AddFilesResult>, AppError> {
    let parsed = parse_upload_multipart(multipart).await?;

    let secret = parsed
        .session_secret
        .as_deref()
        .ok_or_else(|| AppError::unauthorized("session_secret is required to upload files"))?;
    validate_session(&state, secret).await?;

    let result =
        ingest_uploaded_files(&state, parsed.label, parsed.files, parsed.total_bytes).await?;
    Ok(Json(result))
}

/// Form-based upload used by the bridge's own HTML UI. The server only binds
/// to loopback + CORS blocks cross-origin submissions, so skipping the
/// session check is fine — no remote caller can reach this route.
pub async fn add_files_form(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Redirect, AppError> {
    let parsed = parse_upload_multipart(multipart).await?;

    match ingest_uploaded_files(&state, parsed.label, parsed.files, parsed.total_bytes).await {
        Ok(result) => {
            Ok(Redirect::to(&format!("/?uploaded={}", encode_query_component(&result.root_cid),)))
        }
        Err(error) => {
            Ok(Redirect::to(&format!("/?error={}", encode_query_component(&error.message),)))
        }
    }
}
