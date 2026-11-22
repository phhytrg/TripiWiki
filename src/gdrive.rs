use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::handlers::AppState;

#[derive(Debug, Clone)]
pub struct DriveFile {
    pub id: String,
    pub mime_type: String,
}

#[derive(Debug, Clone)]
pub struct GDriveClient {
    http: reqwest::Client,
    folder_id: Option<String>,
    api_key: Option<String>,
    index: Arc<RwLock<HashMap<String, DriveFile>>>,
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
}

#[derive(Serialize)]
pub struct RefreshResult {
    pub enabled: bool,
    pub indexed_files: usize,
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
            index: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn is_folder_enabled(&self) -> bool {
        self.folder_id.is_some() && self.api_key.is_some()
    }

    /// Fetch the list of files inside `GDRIVE_FOLDER_ID` via Google Drive API v3.
    pub async fn refresh_index(&self) -> Result<usize, String> {
        let (Some(folder_id), Some(api_key)) = (&self.folder_id, &self.api_key) else {
            return Ok(0);
        };

        let mut map = HashMap::new();
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
                        "nextPageToken,files(id,name,mimeType)".to_string(),
                    ),
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
                map.insert(
                    f.name,
                    DriveFile {
                        id: f.id,
                        mime_type: f.mime_type,
                    },
                );
            }

            match parsed.next_page_token {
                Some(next) if !next.is_empty() => page_token = Some(next),
                _ => break,
            }
        }

        let count = map.len();
        *self.index.write().await = map;
        info!("Indexed {count} static media files from Google Drive folder");
        Ok(count)
    }

    async fn lookup(&self, filename: &str) -> Option<DriveFile> {
        self.index.read().await.get(filename).cloned()
    }

    async fn fetch_by_id(&self, file_id: &str, fallback_mime: Option<&str>) -> Result<Response, StatusCode> {
        let url = if let Some(ref api_key) = self.api_key {
            format!(
                "https://www.googleapis.com/drive/v3/files/{file_id}?alt=media&key={api_key}"
            )
        } else {
            // Direct public thumbnail/image endpoint (works without API key for public Drive files)
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

/// Serves `/static/images/{filename}` from Google Drive if configured & present,
/// falling back seamlessly to local `static/images/{filename}` on disk.
pub async fn image_handler(
    State(state): State<Arc<AppState>>,
    Path(filename): Path<String>,
) -> Response {
    // Reject path traversal
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    // 1. Try Google Drive folder first (if configured)
    if state.gdrive.is_folder_enabled() {
        let mut entry = state.gdrive.lookup(&filename).await;
        if entry.is_none() {
            // One-time refresh in case a new image was just uploaded to Drive
            if let Err(err) = state.gdrive.refresh_index().await {
                warn!("Google Drive refresh failed: {err}");
            } else {
                entry = state.gdrive.lookup(&filename).await;
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

    // 2. Fallback to local static/images/{filename}
    let local_path = PathBuf::from("static/images").join(&filename);
    match tokio::fs::read(&local_path).await {
        Ok(bytes) => {
            let mime = guess_mime(&filename);
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

/// Direct Google Drive file proxy by file ID: `/media/gdrive/{file_id}`.
/// Works even without `GDRIVE_API_KEY` for files shared as "Anyone with the link -> Viewer".
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

/// Refresh the Google Drive folder index without restarting the server (`POST /api/media/refresh`).
pub async fn refresh_media_handler(State(state): State<Arc<AppState>>) -> Response {
    if !state.gdrive.is_folder_enabled() {
        return Json(RefreshResult {
            enabled: false,
            indexed_files: 0,
        })
        .into_response();
    }

    match state.gdrive.refresh_index().await {
        Ok(count) => Json(RefreshResult {
            enabled: true,
            indexed_files: count,
        })
        .into_response(),
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": err })),
        )
            .into_response(),
    }
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
