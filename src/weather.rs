use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::warn;

use crate::handlers::AppState;

const CACHE_TTL: Duration = Duration::from_secs(600); // 10 minutes
const CLIMATE_CACHE_TTL: Duration = Duration::from_secs(86400); // 24 hours

#[derive(Clone)]
struct CachedEntry {
    fetched_at: Instant,
    payload: Value,
}

#[derive(Clone)]
pub struct WeatherClient {
    http: reqwest::Client,
    cache: Arc<RwLock<HashMap<String, CachedEntry>>>,
}

#[derive(Deserialize)]
pub struct PlaceWeatherQuery {
    pub lat: f64,
    pub lng: f64,
}

#[derive(Deserialize)]
pub struct RouteGenerateQuery {
    pub coords: String,
}

impl WeatherClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent("TripiWiki/1.0")
            .timeout(Duration::from_secs(12))
            .build()
            .expect("Failed to build weather HTTP client");

        Self {
            http,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn fetch_cached(&self, key: &str, url: &str, ttl: Duration) -> Result<Value, String> {
        {
            let guard = self.cache.read().await;
            if let Some(entry) = guard.get(key) {
                if entry.fetched_at.elapsed() < ttl {
                    return Ok(entry.payload.clone());
                }
            }
        }

        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| format!("Weather upstream error: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("Weather upstream status {}", resp.status()));
        }

        let payload: Value = resp
            .json()
            .await
            .map_err(|e| format!("Invalid weather JSON: {e}"))?;

        if payload.get("error").and_then(|v| v.as_bool()) == Some(true) {
            return Err(format!("Open-Meteo returned error: {payload}"));
        }

        let mut guard = self.cache.write().await;
        guard.insert(
            key.to_string(),
            CachedEntry {
                fetched_at: Instant::now(),
                payload: payload.clone(),
            },
        );
        Ok(payload)
    }
}

pub async fn hubs_weather_handler(State(state): State<Arc<AppState>>) -> Response {
    let url = "https://api.open-meteo.com/v1/forecast\
        ?latitude=20.5718,19.8895,18.9233,17.9757,17.4089,15.1823\
        &longitude=102.6134,102.1347,102.4114,102.6365,104.8306,106.1268\
        &current=temperature_2m,weather_code\
        &daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max\
        &timezone=Asia%2FBangkok&forecast_days=1";

    match state.weather.fetch_cached("hubs", url, CACHE_TTL).await {
        Ok(val) => (
            [(header::CACHE_CONTROL, "public, max-age=300")],
            Json(val),
        )
            .into_response(),
        Err(err) => {
            warn!("{err}");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

pub async fn place_weather_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<PlaceWeatherQuery>,
) -> Response {
    if !q.lat.is_finite() || !q.lng.is_finite() || q.lat.abs() > 90.0 || q.lng.abs() > 180.0 {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let key = format!("place:{:.2},{:.2}", q.lat, q.lng);
    let url = format!(
        "https://api.open-meteo.com/v1/forecast\
        ?latitude={:.4}&longitude={:.4}\
        &current=temperature_2m,relative_humidity_2m,weather_code,wind_speed_10m\
        &daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max\
        &timezone=Asia%2FBangkok&forecast_days=5",
        q.lat, q.lng
    );

    match state.weather.fetch_cached(&key, &url, CACHE_TTL).await {
        Ok(val) => (
            [(header::CACHE_CONTROL, "public, max-age=300")],
            Json(val),
        )
            .into_response(),
        Err(err) => {
            warn!("{err}");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

/// Full-year monthly climate summary (Jan–Dec avg high/low °C & total precipitation mm)
/// computed from Open-Meteo's full-year historical archive and cached for 24 hours.
pub async fn climate_weather_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<PlaceWeatherQuery>,
) -> Response {
    if !q.lat.is_finite() || !q.lng.is_finite() || q.lat.abs() > 90.0 || q.lng.abs() > 180.0 {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let key = format!("climate:{:.2},{:.2}", q.lat, q.lng);
    {
        let guard = state.weather.cache.read().await;
        if let Some(entry) = guard.get(&key) {
            if entry.fetched_at.elapsed() < CLIMATE_CACHE_TTL {
                return (
                    [(header::CACHE_CONTROL, "public, max-age=86400")],
                    Json(entry.payload.clone()),
                )
                    .into_response();
            }
        }
    }

    let url = format!(
        "https://archive-api.open-meteo.com/v1/archive\
        ?latitude={:.4}&longitude={:.4}\
        &start_date=2025-01-01&end_date=2025-12-31\
        &daily=temperature_2m_max,temperature_2m_min,precipitation_sum\
        &timezone=auto",
        q.lat, q.lng
    );

    let resp = match state.weather.http.get(&url).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            warn!("Climate archive status {}", r.status());
            return StatusCode::BAD_GATEWAY.into_response();
        }
        Err(e) => {
            warn!("Climate archive error: {e}");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let raw: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            warn!("Climate archive JSON error: {e}");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let Some(daily) = raw.get("daily") else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let times = daily["time"].as_array();
    let highs = daily["temperature_2m_max"].as_array();
    let lows = daily["temperature_2m_min"].as_array();
    let rains = daily["precipitation_sum"].as_array();

    let (Some(times), Some(highs), Some(lows), Some(rains)) = (times, highs, lows, rains) else {
        return StatusCode::BAD_GATEWAY.into_response();
    };

    let mut high_sum = [0.0_f64; 12];
    let mut low_sum = [0.0_f64; 12];
    let mut rain_sum = [0.0_f64; 12];
    let mut counts = [0_u32; 12];

    for i in 0..times.len() {
        let Some(date_str) = times[i].as_str() else {
            continue;
        };
        let Some(m_idx) = date_str
            .get(5..7)
            .and_then(|s| s.parse::<usize>().ok())
            .and_then(|m| m.checked_sub(1))
        else {
            continue;
        };
        if m_idx >= 12 {
            continue;
        }
        if let (Some(hi), Some(lo)) = (highs.get(i).and_then(|v| v.as_f64()), lows.get(i).and_then(|v| v.as_f64())) {
            high_sum[m_idx] += hi;
            low_sum[m_idx] += lo;
            counts[m_idx] += 1;
        }
        if let Some(r) = rains.get(i).and_then(|v| v.as_f64()) {
            rain_sum[m_idx] += r;
        }
    }

    const MONTH_NAMES: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let months: Vec<Value> = (0..12)
        .map(|i| {
            let c = counts[i].max(1) as f64;
            serde_json::json!({
                "month": MONTH_NAMES[i],
                "avg_high": (high_sum[i] / c * 10.0).round() / 10.0,
                "avg_low": (low_sum[i] / c * 10.0).round() / 10.0,
                "rain_mm": rain_sum[i].round(),
            })
        })
        .collect();

    let payload = serde_json::json!({
        "year": 2025,
        "months": months,
    });

    state.weather.cache.write().await.insert(
        key,
        CachedEntry {
            fetched_at: Instant::now(),
            payload: payload.clone(),
        },
    );

    (
        [(header::CACHE_CONTROL, "public, max-age=86400")],
        Json(payload),
    )
        .into_response()
}

/// Generates a real driving route via OSRM between user-selected coordinates (`lng,lat;lng,lat;...`),
/// falling back gracefully to direct segments + haversine distance if OSRM cannot route.
pub async fn route_generate_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<RouteGenerateQuery>,
) -> Response {
    let mut parsed_points: Vec<(f64, f64)> = Vec::new();
    for pair in q.coords.split(';') {
        let Some((lng_s, lat_s)) = pair.split_once(',') else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        let (Ok(lng), Ok(lat)) = (lng_s.trim().parse::<f64>(), lat_s.trim().parse::<f64>()) else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        if !lng.is_finite() || !lat.is_finite() || lat.abs() > 90.0 || lng.abs() > 180.0 {
            return StatusCode::BAD_REQUEST.into_response();
        }
        parsed_points.push((lng, lat));
    }

    if parsed_points.len() < 2 || parsed_points.len() > 20 {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let coord_str: String = parsed_points
        .iter()
        .map(|(lng, lat)| format!("{lng:.5},{lat:.5}"))
        .collect::<Vec<_>>()
        .join(";");

    let osrm_url = format!(
        "https://router.project-osrm.org/route/v1/driving/{coord_str}?overview=full&geometries=geojson"
    );

    if let Ok(resp) = state.weather.http.get(&osrm_url).send().await {
        if resp.status().is_success() {
            if let Ok(val) = resp.json::<Value>().await {
                if let Some(route) = val["routes"].as_array().and_then(|r| r.first()) {
                    let distance_km =
                        (route["distance"].as_f64().unwrap_or(0.0) / 100.0).round() / 10.0;
                    let duration_min =
                        (route["duration"].as_f64().unwrap_or(0.0) / 60.0).round() as u64;
                    return Json(serde_json::json!({
                        "mode": "driving",
                        "distance_km": distance_km,
                        "duration_min": duration_min,
                        "geometry": route["geometry"],
                    }))
                    .into_response();
                }
            }
        }
    }

    // Fallback: direct polyline + haversine distance
    let mut dist_km = 0.0;
    for w in parsed_points.windows(2) {
        dist_km += haversine_km(w[0].1, w[0].0, w[1].1, w[1].0);
    }
    let coords_json: Vec<Value> = parsed_points
        .iter()
        .map(|(lng, lat)| serde_json::json!([lng, lat]))
        .collect();

    Json(serde_json::json!({
        "mode": "direct",
        "distance_km": (dist_km * 10.0).round() / 10.0,
        "duration_min": ((dist_km / 50.0) * 60.0).round() as u64,
        "geometry": {
            "type": "LineString",
            "coordinates": coords_json,
        }
    }))
    .into_response()
}

fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6371.0;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    2.0 * r * a.sqrt().asin()
}
