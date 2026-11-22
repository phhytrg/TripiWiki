mod content;
mod handlers;

use axum::{routing::get, Router};
use std::path::Path;
use std::sync::Arc;
use tower_http::{compression::CompressionLayer, services::ServeDir};
use tracing::info;

use crate::content::WikiStore;
use crate::handlers::{
    index_handler, place_detail_handler, places_geojson_handler, railway_geojson_handler,
    railway_handler, routes_geojson_handler, AppState,
};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let store = WikiStore::load_from_dir(Path::new("content"))
        .expect("Failed to load content directory");
    info!(
        "Loaded {} places across {} regions",
        store.places.len(),
        store.regions.len()
    );

    let state = Arc::new(AppState { store });

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/railway", get(railway_handler))
        .route("/places/{slug}", get(place_detail_handler))
        .route("/api/places.geojson", get(places_geojson_handler))
        .route("/api/routes.geojson", get(routes_geojson_handler))
        .route("/api/railway.geojson", get(railway_geojson_handler))
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
