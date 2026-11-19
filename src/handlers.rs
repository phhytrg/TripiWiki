use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::content::{Place, RegionInfo, WikiStore};

pub struct AppState {
    pub store: WikiStore,
}

pub struct HtmlTemplate<T>(pub T);

impl<T: Template> IntoResponse for HtmlTemplate<T> {
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Template rendering error: {err}"),
            )
                .into_response(),
        }
    }
}

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate<'a> {
    pub regions: &'a [RegionInfo],
    pub places: Vec<&'a Place>,
    pub active_region: String,
    pub query: String,
    pub total_places: usize,
}

#[derive(Template)]
#[template(path = "place.html")]
pub struct PlaceTemplate<'a> {
    pub place: &'a Place,
    pub related: Vec<&'a Place>,
}

#[derive(Template)]
#[template(path = "partials/place_cards.html")]
pub struct PlaceCardsPartial<'a> {
    pub places: Vec<&'a Place>,
}

#[derive(Template)]
#[template(path = "railway.html")]
pub struct RailwayTemplate;

#[derive(Deserialize, Default)]
pub struct SearchParams {
    pub region: Option<String>,
    pub q: Option<String>,
}

pub async fn index_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
    headers: HeaderMap,
) -> Response {
    let active_region = params
        .region
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "all".to_string());
    let query = params.q.clone().unwrap_or_default();

    let places = state.store.filter(Some(&active_region), Some(&query));

    if headers.contains_key("hx-request") {
        return HtmlTemplate(PlaceCardsPartial { places }).into_response();
    }

    HtmlTemplate(IndexTemplate {
        regions: &state.store.regions,
        places,
        active_region,
        query,
        total_places: state.store.places.len(),
    })
    .into_response()
}

pub async fn place_detail_handler(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> Response {
    let Some(place) = state.store.find_by_slug(&slug) else {
        return (StatusCode::NOT_FOUND, "Destination not found").into_response();
    };

    let related: Vec<&Place> = state
        .store
        .filter(Some(&place.meta.region), None)
        .into_iter()
        .filter(|p| p.meta.slug != place.meta.slug)
        .take(3)
        .collect();

    HtmlTemplate(PlaceTemplate { place, related }).into_response()
}

pub async fn places_geojson_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> Json<Value> {
    Json(state.store.places_geojson(params.region.as_deref()))
}

pub async fn routes_geojson_handler(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(state.store.routes_geojson.clone())
}

pub async fn railway_handler() -> Response {
    HtmlTemplate(RailwayTemplate).into_response()
}

pub async fn railway_geojson_handler(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(state.store.railway_geojson.clone())
}
