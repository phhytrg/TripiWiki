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
use tokio::sync::RwLock;

use crate::content::{CountryInfo, Place, RegionInfo, WikiStore};
use crate::gdrive::GDriveClient;
use crate::weather::WeatherClient;

pub struct AppState {
    pub store: RwLock<WikiStore>,
    pub gdrive: GDriveClient,
    pub weather: WeatherClient,
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

pub struct RegionSection<'a> {
    pub country_slug: String,
    pub country_name: String,
    pub region_slug: String,
    pub region_name: String,
    pub center_lat: f64,
    pub center_lng: f64,
    pub places: Vec<&'a Place>,
}

fn group_places_by_region<'a>(places: Vec<&'a Place>) -> Vec<RegionSection<'a>> {
    let mut sections: Vec<RegionSection<'a>> = Vec::new();
    for place in places {
        if let Some(sec) = sections
            .iter_mut()
            .find(|s| s.country_slug == place.meta.country && s.region_slug == place.meta.region)
        {
            sec.places.push(place);
        } else {
            sections.push(RegionSection {
                country_slug: place.meta.country.clone(),
                country_name: place.meta.country_name.clone(),
                region_slug: place.meta.region.clone(),
                region_name: place.meta.region_name.clone(),
                center_lat: place.meta.lat,
                center_lng: place.meta.lng,
                places: vec![place],
            });
        }
    }
    for sec in &mut sections {
        let n = sec.places.len() as f64;
        sec.center_lat = sec.places.iter().map(|p| p.meta.lat).sum::<f64>() / n;
        sec.center_lng = sec.places.iter().map(|p| p.meta.lng).sum::<f64>() / n;
    }
    // Sort province sections North -> South by average latitude
    sections.sort_by(|a, b| b.center_lat.total_cmp(&a.center_lat));
    sections
}

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate<'a> {
    pub countries: &'a [CountryInfo],
    pub regions: Vec<RegionInfo>,
    pub sections: Vec<RegionSection<'a>>,
    pub active_country: String,
    pub active_country_name: String,
    pub active_region: String,
    pub query: String,
    pub total_places: usize,
}

#[derive(Template)]
#[template(path = "place.html")]
pub struct PlaceTemplate<'a> {
    pub place: &'a Place,
    pub countries: &'a [CountryInfo],
    pub related: Vec<&'a Place>,
}

#[derive(Template)]
#[template(path = "partials/place_cards.html")]
pub struct PlaceCardsPartial<'a> {
    pub sections: Vec<RegionSection<'a>>,
}

#[derive(Template)]
#[template(path = "railway.html")]
pub struct RailwayTemplate<'a> {
    pub countries: &'a [CountryInfo],
    pub active_country: String,
}

#[derive(Deserialize, Default)]
pub struct SearchParams {
    pub country: Option<String>,
    pub region: Option<String>,
    pub q: Option<String>,
}

pub async fn index_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
    headers: HeaderMap,
) -> Response {
    let active_country = params
        .country
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "all".to_string());
    let active_region = params
        .region
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "all".to_string());
    let query = params.q.clone().unwrap_or_default();

    let store = state.store.read().await;
    let places = store.filter(Some(&active_country), Some(&active_region), Some(&query));
    let sections = group_places_by_region(places);

    if headers.contains_key("hx-request") {
        return HtmlTemplate(PlaceCardsPartial { sections }).into_response();
    }

    let regions = store.regions_for_country(Some(&active_country));
    let total_places = store.filter(Some(&active_country), None, None).len();
    let active_country_name = store
        .countries
        .iter()
        .find(|c| c.slug == active_country)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "All Destinations".to_string());

    HtmlTemplate(IndexTemplate {
        countries: &store.countries,
        regions,
        sections,
        active_country,
        active_country_name,
        active_region,
        query,
        total_places,
    })
    .into_response()
}

pub async fn place_detail_handler(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> Response {
    let store = state.store.read().await;
    let Some(place) = store.find_by_slug(&slug) else {
        return (StatusCode::NOT_FOUND, "Destination not found").into_response();
    };

    let related: Vec<&Place> = store
        .filter(Some(&place.meta.country), Some(&place.meta.region), None)
        .into_iter()
        .filter(|p| p.meta.slug != place.meta.slug)
        .take(3)
        .collect();

    HtmlTemplate(PlaceTemplate {
        place,
        countries: &store.countries,
        related,
    })
    .into_response()
}

pub async fn places_geojson_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> Json<Value> {
    let store = state.store.read().await;
    Json(store.places_geojson(params.country.as_deref(), params.region.as_deref()))
}

pub async fn railway_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> Response {
    let store = state.store.read().await;
    let active_country = params
        .country
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "all".to_string());
    HtmlTemplate(RailwayTemplate {
        countries: &store.countries,
        active_country,
    })
    .into_response()
}

pub async fn railway_geojson_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> Json<Value> {
    let store = state.store.read().await;
    Json(store.railway_geojson_for(params.country.as_deref()))
}
