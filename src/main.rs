mod content;
mod gdrive;
mod handlers;
mod weather;

use axum::{
    routing::{get, post},
    Router,
};
use std::path::Path;
use std::sync::Arc;
use tower_http::{compression::CompressionLayer, services::ServeDir};
use tracing::{info, warn};

use crate::content::WikiStore;
use crate::gdrive::{
    gdrive_id_handler, image_handler, media_status_handler, refresh_media_handler, GDriveClient,
};
use crate::handlers::{
    index_handler, place_detail_handler, places_geojson_handler, railway_geojson_handler,
    railway_handler, AppState,
};
use crate::weather::{
    climate_weather_handler, hubs_weather_handler, place_weather_handler, route_generate_handler,
    WeatherClient,
};

#[tokio::main]
async fn main() {
    load_dotenv();
    tracing_subscriber::fmt::init();

    let store = WikiStore::load_from_dir(Path::new("content"))
        .expect("Failed to load content directory");
    info!(
        "Loaded {} places across {} regions",
        store.places.len(),
        store.regions.len()
    );

    let gdrive = GDriveClient::from_env();
    if gdrive.is_folder_enabled() {
        if let Err(err) = gdrive.refresh_index().await {
            warn!("Initial Google Drive index failed (will retry on demand / fallback to local): {err}");
        }
    } else {
        info!("Google Drive folder not configured; serving images from local static/images/");
    }

    let weather = WeatherClient::new();
    let state = Arc::new(AppState {
        store,
        gdrive,
        weather,
    });

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/railway", get(railway_handler))
        .route("/places/{slug}", get(place_detail_handler))
        .route("/api/places.geojson", get(places_geojson_handler))
        .route("/api/railway.geojson", get(railway_geojson_handler))
        .route("/api/route/generate", get(route_generate_handler))
        .route("/api/weather/hubs", get(hubs_weather_handler))
        .route("/api/weather/place", get(place_weather_handler))
        .route("/api/weather/climate", get(climate_weather_handler))
        .route("/api/media/status", get(media_status_handler))
        .route("/api/media/refresh", post(refresh_media_handler))
        .route("/media/gdrive/{file_id}", get(gdrive_id_handler))
        .route("/static/images/{*path}", get(image_handler))
        .nest_service("/static", ServeDir::new("static"))
        .layer(CompressionLayer::new())
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind {addr}: {e}"));
    info!("TripiWiki running at http://{addr}");
    axum::serve(listener, app).await.expect("Server error");
}

/// Lightweight `.env` loader for local development (existing env vars take precedence).
fn load_dotenv() {
    let Ok(contents) = std::fs::read_to_string(".env") else {
        return;
    };
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, val)) = trimmed.split_once('=') {
            let key = key.trim();
            let val = val.trim().trim_matches('"').trim_matches('\'');
            if !key.is_empty() && !val.is_empty() && std::env::var_os(key).is_none() {
                std::env::set_var(key, val);
            }
        }
    }
}
