use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine as _;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::content::{parse_markdown_place, Place, WikiStore};
use crate::handlers::AppState;

const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const GDOC_MIME: &str = "application/vnd.google-apps.document";
const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.readonly";
const DEFAULT_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";

/// True only during local development (never in `--release` or on Render).
fn is_local_debug() -> bool {
    cfg!(debug_assertions) && std::env::var("RENDER").is_err()
}

fn mask_secret(val: &Option<String>) -> String {
    match val {
        Some(s) if s.len() > 8 => format!("{}...{} (len={})", &s[..4], &s[s.len() - 4..], s.len()),
        Some(s) => format!("*** (len={})", s.len()),
        None => "<unset>".to_string(),
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ServiceAccountKey {
    client_email: String,
    private_key: String,
    #[serde(default = "default_token_uri")]
    token_uri: String,
}

fn default_token_uri() -> String {
    DEFAULT_TOKEN_URI.to_string()
}

#[derive(Serialize)]
struct JwtClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: u64,
    exp: u64,
}

#[derive(Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
}

fn default_expires_in() -> u64 {
    3600
}

#[derive(Debug, Clone)]
struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

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
enum DriveAuth {
    Bearer(String),
    ApiKey(String),
}

#[derive(Debug, Clone)]
pub struct GDriveClient {
    http: reqwest::Client,
    folder_id: Option<String>,
    places_folder_id: Option<String>,
    api_key: Option<String>,
    service_account: Option<ServiceAccountKey>,
    token_cache: Arc<RwLock<Option<CachedToken>>>,
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

fn load_service_account_from_env() -> Option<ServiceAccountKey> {
    // 1. Try GDRIVE_SERVICE_ACCOUNT_JSON (raw JSON or base64-encoded JSON — ideal for Render)
    if let Ok(raw) = std::env::var("GDRIVE_SERVICE_ACCOUNT_JSON") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            if let Ok(key) = serde_json::from_str::<ServiceAccountKey>(trimmed) {
                return Some(key);
            }
            if let Ok(decoded_bytes) = base64::engine::general_purpose::STANDARD.decode(trimmed) {
                if let Ok(decoded_str) = String::from_utf8(decoded_bytes) {
                    if let Ok(key) = serde_json::from_str::<ServiceAccountKey>(&decoded_str) {
                        return Some(key);
                    }
                }
            }
            warn!("GDRIVE_SERVICE_ACCOUNT_JSON is set but could not be parsed as valid Service Account JSON");
        }
    }

    // 2. Try GDRIVE_SERVICE_ACCOUNT_FILE (or default local path credentials/service-account.json)
    let file_path = std::env::var("GDRIVE_SERVICE_ACCOUNT_FILE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "credentials/service-account.json".to_string());

    if std::path::Path::new(&file_path).exists() {
        match std::fs::read_to_string(&file_path) {
            Ok(contents) => match serde_json::from_str::<ServiceAccountKey>(&contents) {
                Ok(key) => return Some(key),
                Err(err) => {
                    warn!("Failed to parse Service Account file '{file_path}': {err}");
                }
            },
            Err(err) => {
                warn!("Failed to read Service Account file '{file_path}': {err}");
            }
        }
    }

    None
}

impl GDriveClient {
    /// Reads Google Drive config from environment variables.
    /// Prefers Service Account OAuth (`GDRIVE_SERVICE_ACCOUNT_JSON` / `GDRIVE_SERVICE_ACCOUNT_FILE`),
    /// falling back to `GDRIVE_API_KEY` if no Service Account is configured.
    pub fn from_env() -> Self {
        let folder_id = std::env::var("GDRIVE_FOLDER_ID")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let places_folder_id = std::env::var("GDRIVE_PLACES_FOLDER_ID")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let api_key = std::env::var("GDRIVE_API_KEY")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let service_account = load_service_account_from_env();

        let http = reqwest::Client::builder()
            .user_agent("TripiWiki/1.0")
            .timeout(Duration::from_secs(12))
            .build()
            .expect("Failed to build HTTP client");

        if is_local_debug() {
            let auth_mode = if let Some(ref sa) = service_account {
                format!("ServiceAccount({})", sa.client_email)
            } else if api_key.is_some() {
                format!("ApiKey({})", mask_secret(&api_key))
            } else {
                "<none>".to_string()
            };
            info!(
                "[GDrive local debug] config: auth={}, GDRIVE_FOLDER_ID={}, GDRIVE_PLACES_FOLDER_ID={}",
                auth_mode,
                folder_id.as_deref().unwrap_or("<unset>"),
                places_folder_id.as_deref().unwrap_or("<unset>"),
            );
        }

        Self {
            http,
            folder_id,
            places_folder_id,
            api_key,
            service_account,
            token_cache: Arc::new(RwLock::new(None)),
            index: Arc::new(RwLock::new(DriveIndex::default())),
        }
    }

    fn has_auth(&self) -> bool {
        self.service_account.is_some() || self.api_key.is_some()
    }

    pub fn auth_mode(&self) -> &'static str {
        if self.service_account.is_some() {
            "service_account"
        } else if self.api_key.is_some() {
            "api_key"
        } else {
            "none"
        }
    }

    pub fn is_folder_enabled(&self) -> bool {
        self.folder_id.is_some() && self.has_auth()
    }

    pub fn is_places_enabled(&self) -> bool {
        self.places_folder_id.is_some() && self.has_auth()
    }

    /// Obtain a valid OAuth 2.0 Bearer token (cached for ~55 min) or fallback to API key.
    async fn resolve_auth(&self) -> Result<DriveAuth, String> {
        if let Some(ref sa) = self.service_account {
            // 1. Check cached token
            {
                let guard = self.token_cache.read().await;
                if let Some(ref cached) = *guard {
                    if Instant::now() + Duration::from_secs(120) < cached.expires_at {
                        return Ok(DriveAuth::Bearer(cached.access_token.clone()));
                    }
                }
            }

            // 2. Mint a signed RS256 JWT and exchange for an access_token
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| format!("System clock error: {e}"))?
                .as_secs();

            let claims = JwtClaims {
                iss: &sa.client_email,
                scope: DRIVE_SCOPE,
                aud: &sa.token_uri,
                iat: now,
                exp: now + 3600,
            };

            let encoding_key = EncodingKey::from_rsa_pem(sa.private_key.as_bytes())
                .map_err(|e| format!("Invalid Service Account RSA private_key: {e}"))?;

            let jwt = encode(&Header::new(Algorithm::RS256), &claims, &encoding_key)
                .map_err(|e| format!("Failed to sign Service Account JWT: {e}"))?;

            let resp = self
                .http
                .post(&sa.token_uri)
                .form(&[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                    ("assertion", jwt.as_str()),
                ])
                .send()
                .await
                .map_err(|e| format!("Google OAuth token request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Google OAuth token error {status}: {body}"));
            }

            let tok: OAuthTokenResponse = resp
                .json()
                .await
                .map_err(|e| format!("Failed to parse OAuth token response: {e}"))?;

            let expires_at = Instant::now() + Duration::from_secs(tok.expires_in);
            *self.token_cache.write().await = Some(CachedToken {
                access_token: tok.access_token.clone(),
                expires_at,
            });

            if is_local_debug() {
                info!(
                    "[GDrive local debug] minted fresh OAuth access token for {} (expires in {}s)",
                    sa.client_email, tok.expires_in
                );
            }

            return Ok(DriveAuth::Bearer(tok.access_token));
        }

        if let Some(ref key) = self.api_key {
            return Ok(DriveAuth::ApiKey(key.clone()));
        }

        Err("No Google Drive credentials configured".to_string())
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder, auth: &DriveAuth) -> reqwest::RequestBuilder {
        match auth {
            DriveAuth::Bearer(token) => req.bearer_auth(token),
            DriveAuth::ApiKey(key) => req.query(&[("key", key.as_str())]),
        }
    }

    /// Recursively lists files in a Google Drive folder (`orderBy=modifiedTime asc` -> newest wins).
    async fn list_folder_recursive(
        &self,
        root_folder_id: &str,
    ) -> Result<(HashMap<String, DriveFile>, Vec<String>), String> {
        let auth = self.resolve_auth().await?;
        let mut files: HashMap<String, DriveFile> = HashMap::new();
        let mut duplicates: Vec<String> = Vec::new();
        let mut queue: VecDeque<(String, String)> = VecDeque::new();
        queue.push_back((root_folder_id.to_string(), String::new()));

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
                    ]);
                req = self.apply_auth(req, &auth);

                if let Some(ref token) = page_token {
                    req = req.query(&[("pageToken", token.as_str())]);
                }

                if is_local_debug() {
                    info!(
                        "[GDrive local debug] listing folder_id='{}' prefix='{}' page_token={:?}",
                        folder_id, prefix, page_token
                    );
                }

                let resp = req
                    .send()
                    .await
                    .map_err(|e| format!("Google Drive API request failed: {e}"))?;

                let status = resp.status();
                if is_local_debug() {
                    info!(
                        "[GDrive local debug] folder '{}' -> HTTP {}",
                        folder_id, status
                    );
                }

                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(format!("Google Drive API error {status}: {body}"));
                }

                let parsed: DriveListResponse = resp
                    .json()
                    .await
                    .map_err(|e| format!("Failed to parse Google Drive response: {e}"))?;

                if is_local_debug() {
                    info!(
                        "[GDrive local debug] folder '{}' returned {} items",
                        folder_id,
                        parsed.files.len()
                    );
                }

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
        Ok((files, duplicates))
    }

    /// Recursively indexes `GDRIVE_FOLDER_ID` (images) via Google Drive API v3.
    pub async fn refresh_index(&self) -> Result<(usize, Vec<String>), String> {
        let Some(root_folder_id) = self.folder_id.as_deref() else {
            return Ok((0, Vec::new()));
        };
        if !self.has_auth() {
            return Ok((0, Vec::new()));
        }

        let (files, duplicates) = self.list_folder_recursive(root_folder_id).await?;
        let count = files.len();
        let dup_clone = duplicates.clone();
        *self.index.write().await = DriveIndex { files, duplicates };
        info!(
            "Indexed {count} static media files from Google Drive ({} duplicate names resolved by newest)",
            dup_clone.len()
        );
        Ok((count, dup_clone))
    }

    /// Recursively fetches and parses all Markdown (`.md`) place guides from `GDRIVE_PLACES_FOLDER_ID`.
    pub async fn fetch_places(&self) -> Result<(Vec<Place>, Vec<String>), String> {
        let Some(places_folder_id) = self.places_folder_id.as_deref() else {
            return Err("GDRIVE_PLACES_FOLDER_ID not configured".to_string());
        };
        let auth = self.resolve_auth().await?;
        let (files, duplicates) = self.list_folder_recursive(places_folder_id).await?;
        let mut places = Vec::new();
        let mut downloaded = 0usize;

        for (rel_path, file) in files {
            let is_md = rel_path.to_lowercase().ends_with(".md");
            let is_gdoc = file.mime_type == GDOC_MIME;
            if !is_md && !is_gdoc {
                continue;
            }

            // Only pace when using unauthenticated API key; Service Account OAuth handles normal throughput cleanly
            if matches!(auth, DriveAuth::ApiKey(_)) && downloaded > 0 {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            downloaded += 1;

            let req = if is_gdoc {
                let url = format!(
                    "https://www.googleapis.com/drive/v3/files/{}/export",
                    file.id
                );
                self.apply_auth(self.http.get(&url).query(&[("mimeType", "text/plain")]), &auth)
            } else {
                let url = format!("https://www.googleapis.com/drive/v3/files/{}", file.id);
                self.apply_auth(self.http.get(&url).query(&[("alt", "media")]), &auth)
            };

            match req.send().await {
                Ok(resp) if resp.status().is_success() => match resp.text().await {
                    Ok(raw) => {
                        let clean = raw.trim_start_matches('\u{feff}');
                        match parse_markdown_place(clean) {
                            Ok(place) => {
                                if is_local_debug() {
                                    info!(
                                        "[GDrive local debug] parsed place '{}' -> slug='{}'",
                                        rel_path, place.meta.slug
                                    );
                                }
                                places.push(place);
                            }
                            Err(err) => warn!("Skipping invalid Drive place '{rel_path}': {err}"),
                        }
                    }
                    Err(err) => warn!("Failed reading Drive place '{rel_path}': {err}"),
                },
                Ok(resp) => {
                    let status = resp.status();
                    let err_body = if is_local_debug() {
                        resp.text().await.unwrap_or_default()
                    } else {
                        String::new()
                    };
                    warn!("Failed downloading Drive place '{rel_path}': HTTP {status} {err_body}");
                }
                Err(err) => warn!("Request failed for Drive place '{rel_path}': {err}"),
            }
        }

        info!(
            "Loaded {} Markdown places from Google Drive ({} duplicates resolved by newest)",
            places.len(),
            duplicates.len()
        );
        Ok((places, duplicates))
    }

    async fn lookup(&self, rel_path: &str) -> Option<DriveFile> {
        self.index.read().await.files.get(rel_path).cloned()
    }

    async fn fetch_by_id(
        &self,
        file_id: &str,
        fallback_mime: Option<&str>,
    ) -> Result<Response, StatusCode> {
        let req = match self.resolve_auth().await {
            Ok(auth) => {
                let url = format!("https://www.googleapis.com/drive/v3/files/{file_id}");
                self.apply_auth(self.http.get(&url).query(&[("alt", "media")]), &auth)
            }
            Err(_) => {
                // Public Drive thumbnail fallback when no credentials are configured
                let url = format!("https://lh3.googleusercontent.com/d/{file_id}=w1400");
                self.http.get(&url)
            }
        };

        let resp = req
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .map_err(|err| {
                warn!("Drive image fetch timeout/error for {file_id}: {err}");
                if err.is_timeout() {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::BAD_GATEWAY
                }
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            warn!("Drive image fetch HTTP {status} for {file_id}");
            return Err(if status == StatusCode::NOT_FOUND {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_GATEWAY
            });
        }

        let content_type = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .or_else(|| fallback_mime.map(str::to_string))
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let bytes = resp.bytes().await.map_err(|err| {
            warn!("Drive image body read failed for {file_id}: {err}");
            StatusCode::BAD_GATEWAY
        })?;

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

/// Reload all place Markdown guides from Google Drive (or local fallback) without restarting (`POST /api/content/refresh`).
pub async fn refresh_content_handler(State(state): State<Arc<AppState>>) -> Response {
    if !state.gdrive.is_places_enabled() {
        match WikiStore::load_from_dir(std::path::Path::new("content")) {
            Ok(fresh) => {
                let count = fresh.places.len();
                *state.store.write().await = fresh;
                return Json(json!({
                    "source": "local",
                    "loaded_places": count,
                }))
                .into_response();
            }
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": err })),
                )
                    .into_response();
            }
        }
    }

    match state.gdrive.fetch_places().await {
        Ok((places, duplicates)) => {
            let railway = state.store.read().await.railway_geojson.clone();
            let fresh = WikiStore::from_places(places, railway);
            let count = fresh.places.len();
            let countries = fresh.countries.len();
            let regions = fresh.regions.len();
            *state.store.write().await = fresh;
            Json(json!({
                "source": "gdrive",
                "loaded_places": count,
                "countries": countries,
                "regions": regions,
                "duplicates": duplicates,
            }))
            .into_response()
        }
        Err(err) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": err }))).into_response(),
    }
}

/// Inspect Google Drive media & places sync status (`GET /api/media/status`).
pub async fn media_status_handler(State(state): State<Arc<AppState>>) -> Response {
    let index = state.gdrive.index.read().await;
    let store = state.store.read().await;
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
        "auth_mode": state.gdrive.auth_mode(),
        "enabled": state.gdrive.is_folder_enabled(),
        "indexed_files": files.len(),
        "duplicates": index.duplicates,
        "places_gdrive_enabled": state.gdrive.is_places_enabled(),
        "loaded_places": store.places.len(),
        "countries": store.countries.len(),
        "regions": store.regions.len(),
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
