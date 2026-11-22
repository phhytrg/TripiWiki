use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::handlers::AppState;

const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

#[derive(Debug, Clone)]
pub struct DriveFile {
    pub id: String,
    pub mime_type: String,
    pub modified_time: Option<String>,
}

#[derive(Debug, Default)]
struct DriveIndex {
    files: HashMap<String, DriveFile>,
    duplicates: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GDriveClient {
    http: reqwest::Client,
    folder_id: Option<String>,
    api_key: Option<String>,
    index: Arc<RwLock<DriveIndex>>,
}

#[derive(Deserialize)]
struct DriveListResponse {
    files: Vec<DriveApiFile>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct DriveApiFile {
    id: String,
    name: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
    #[serde(rename = "modifiedTime")]
    modified_time: Option<String>,
}

#[derive(Serialize)]
pub struct RefreshResult {
    pub enabled: bool,
    pub indexed_files: usize,
    pub duplicates: Vec<String>,
}

impl GDriveClient {
    /// Reads `GDRIVE_FOLDER_ID` and `GDRIVE_API_KEY` from environment variables.
    /// If either is unset, Google Drive folder proxying is disabled and local `static/images/` is used.
    pub fn from_env() -> Self {
        let folder_id = std::env::var("GDRIVE_FOLDER_ID")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let api_key = std::env::var("GDRIVE_API_KEY")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let http = reqwest::Client::builder()
            .user_agent("TripiWiki/1.0")
            .build()
            .expect("Failed to build HTTP client");

        Self {
            http,
            folder_id,
            api_key,
            index: Arc::new(RwLock::new(DriveIndex::default())),
        }
    }

    pub fn is_folder_enabled(&self) -> bool {
        self.folder_id.is_some() && self.api_key.is_some()
    }

    /// Recursively indexes `GDRIVE_FOLDER_ID` and all subfolders via Google Drive API v3.
    /// Uses `orderBy=modifiedTime asc` so if duplicate filenames exist in the same folder,
    /// the most recently modified file always wins, and duplicates are recorded for visibility.
    pub async fn refresh_index(&self) -> Result<(usize, Vec<String>), String> {
        let (Some(root_folder_id), Some(api_key)) = (&self.folder_id, &self.api_key) else {
            return Ok((0, Vec::new()));
        };

        let mut files: HashMap<String, DriveFile> = HashMap::new();
        let mut duplicates: Vec<String> = Vec::new();
        let mut queue: VecDeque<(String, String)> = VecDeque::new();
        queue.push_back((root_folder_id.clone(), String::new()));

        while let Some((folder_id, prefix)) = queue.pop_front() {
            let mut page_token: Option<String> = None;

            loop {
                let mut req = self
                    .http
                    .get("https://www.googleapis.com/drive/v3/files")
                    .query(&[
                        (
                            "q",
                            format!("'{folder_id}' in parents and trashed = false"),
                        ),
                        (
                            "fields",
                            "nextPageToken,files(id,name,mimeType,modifiedTime)".to_string(),
                        ),
                        ("orderBy", "modifiedTime asc".to_string()),
                        ("pageSize", "1000".to_string()),
                        ("key", api_key.clone()),
                    ]);

                if let Some(ref token) = page_token {
                    req = req.query(&[("pageToken", token.as_str())]);
                }

                let resp = req
                    .send()
                    .await
                    .map_err(|e| format!("Google Drive API request failed: {e}"))?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(format!("Google Drive API error {status}: {body}"));
                }

                let parsed: DriveListResponse = resp
                    .json()
                    .await
                    .map_err(|e| format!("Failed to parse Google Drive response: {e}"))?;

                for f in parsed.files {
                    let rel_path = if prefix.is_empty() {
                        f.name.clone()
                    } else {
                        format!("{prefix}/{}", f.name)
                    };

                    if f.mime_type == FOLDER_MIME {
                        queue.push_back((f.id, rel_path));
                        continue;
                    }

                    if files.contains_key(&rel_path) {
                        warn!(
                            "Duplicate filename in Google Drive: '{rel_path}' (using newest modifiedTime {:?})",
                            f.modified_time
                        );
                        if !duplicates.contains(&rel_path) {
                            duplicates.push(rel_path.clone());
                        }
                    }

                    files.insert(
                        rel_path,
                        DriveFile {
                            id: f.id,
                            mime_type: f.mime_type,
                            modified_time: f.modified_time,
                        },
                    );
                }

                match parsed.next_page_token {
                    Some(next) if !next.is_empty() => page_token = Some(next),
                    _ => break,
                }
            }
        }

        duplicates.sort();
        let count = files.len();
        let dup_clone = duplicates.clone();
        *self.index.write().await = DriveIndex { files, duplicates };
        info!(
            "Indexed {count} static media files from Google Drive ({} duplicate names resolved by newest)",
            dup_clone.len()
        );
        Ok((count, dup_clone))
    }

    async fn lookup(&self, rel_path: &str) -> Option<DriveFile> {
        self.index.read().await.files.get(rel_path).cloned()
    }

    async fn fetch_by_id(
        &self,
        file_id: &str,
        fallback_mime: Option<&str>,
    ) -> Result<Response, StatusCode> {
        let url = if let Some(ref api_key) = self.api_key {
            format!(
                "https://www.googleapis.com/drive/v3/files/{file_id}?alt=media&key={api_key}"
            )
        } else {
            // Works without API key for public Drive files ("Anyone with the link -> Viewer")
            format!("https://lh3.googleusercontent.com/d/{file_id}=w1400")
        };

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?;

        if !resp.status().is_success() {
            return Err(StatusCode::NOT_FOUND);
        }

        let content_type = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .or_else(|| fallback_mime.map(str::to_string))
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let bytes = resp.bytes().await.map_err(|_| StatusCode::BAD_GATEWAY)?;

        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CACHE_CONTROL, "public, max-age=86400")
            .body(Body::from(bytes))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    }
}

/// Validates a relative media path (supports subfolders like `vientiane/01.jpg`, blocks traversal).
fn is_safe_rel_path(rel_path: &str) -> bool {
    if rel_path.is_empty() || rel_path.starts_with('/') || rel_path.contains('\\') {
        return false;
    }
    rel_path
        .split('/')
        .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// Serves `/static/images/{*path}` (including subfolders like `/static/images/vientiane/01.jpg`)
/// from Google Drive if configured & present, falling back to local `static/images/{path}` on disk.
pub async fn image_handler(
    State(state): State<Arc<AppState>>,
    Path(raw_path): Path<String>,
) -> Response {
    let rel_path = raw_path.trim_start_matches('/');
    if !is_safe_rel_path(rel_path) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    // 1. Try Google Drive folder/subfolders first (if configured)
    if state.gdrive.is_folder_enabled() {
        let mut entry = state.gdrive.lookup(rel_path).await;
        if entry.is_none() {
            if let Err(err) = state.gdrive.refresh_index().await {
                warn!("Google Drive refresh failed: {err}");
            } else {
                entry = state.gdrive.lookup(rel_path).await;
            }
        }
        if let Some(file) = entry {
            if let Ok(resp) = state
                .gdrive
                .fetch_by_id(&file.id, Some(&file.mime_type))
                .await
            {
                return resp;
            }
        }
    }

    // 2. Fallback to local static/images/{rel_path}
    let local_path = PathBuf::from("static/images").join(rel_path);
    match tokio::fs::read(&local_path).await {
        Ok(bytes) => {
            let mime = guess_mime(rel_path);
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .header(header::CACHE_CONTROL, "public, max-age=86400")
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Direct Google Drive file proxy by File ID: `/media/gdrive/{file_id}`.
/// Globally unique (zero conflicts) and works even without `GDRIVE_API_KEY`.
pub async fn gdrive_id_handler(
    State(state): State<Arc<AppState>>,
    Path(file_id): Path<String>,
) -> Response {
    if !file_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return StatusCode::BAD_REQUEST.into_response();
    }

    match state.gdrive.fetch_by_id(&file_id, None).await {
        Ok(resp) => resp,
        Err(status) => status.into_response(),
    }
}

/// Refresh the Google Drive folder index without restarting (`POST /api/media/refresh`).
pub async fn refresh_media_handler(State(state): State<Arc<AppState>>) -> Response {
    if !state.gdrive.is_folder_enabled() {
        return Json(RefreshResult {
            enabled: false,
            indexed_files: 0,
            duplicates: Vec::new(),
        })
        .into_response();
    }

    match state.gdrive.refresh_index().await {
        Ok((count, duplicates)) => Json(RefreshResult {
            enabled: true,
            indexed_files: count,
            duplicates,
        })
        .into_response(),
        Err(err) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": err }))).into_response(),
    }
}

/// Inspect Google Drive media status & duplicate warnings (`GET /api/media/status`).
pub async fn media_status_handler(State(state): State<Arc<AppState>>) -> Response {
    let index = state.gdrive.index.read().await;
    let mut files: Vec<_> = index
        .files
        .iter()
        .map(|(path, f)| {
            json!({
                "path": path,
                "id": f.id,
                "modified_time": f.modified_time,
            })
        })
        .collect();
    files.sort_by(|a, b| {
        a.get("path")
            .and_then(|v| v.as_str())
            .cmp(&b.get("path").and_then(|v| v.as_str()))
    });

    Json(json!({
        "enabled": state.gdrive.is_folder_enabled(),
        "indexed_files": files.len(),
        "duplicates": index.duplicates,
        "files": files,
    }))
    .into_response()
}

fn guess_mime(filename: &str) -> &'static str {
    let lower = filename.to_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
}
