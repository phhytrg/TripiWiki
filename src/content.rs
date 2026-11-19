use pulldown_cmark::{html, Options, Parser};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaceFrontmatter {
    pub title: String,
    pub slug: String,
    pub region: String,
    pub region_name: String,
    pub category: String,
    pub lat: f64,
    pub lng: f64,
    pub duration: String,
    pub best_time: String,
    pub summary: String,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub images: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl PlaceFrontmatter {
    /// Returns all gallery images for this place (falling back to `image` if `images` is empty).
    pub fn gallery(&self) -> Vec<&str> {
        if !self.images.is_empty() {
            self.images.iter().map(String::as_str).collect()
        } else if let Some(ref single) = self.image {
            vec![single.as_str()]
        } else {
            Vec::new()
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Place {
    pub meta: PlaceFrontmatter,
    pub html_body: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegionInfo {
    pub slug: String,
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct WikiStore {
    pub places: Vec<Place>,
    pub regions: Vec<RegionInfo>,
    pub routes_geojson: Value,
    pub railway_geojson: Value,
}

impl WikiStore {
    /// Load all markdown places and GeoJSON routes from the `content/` directory.
    pub fn load_from_dir(content_dir: &Path) -> Result<Self, String> {
        let places = load_places(&content_dir.join("places"))?;
        let regions = build_region_index(&places);
        let routes_geojson = load_routes_geojson(&content_dir.join("routes"))?;
        let railway_geojson = load_single_geojson(&content_dir.join("railway/lcr-railway.geojson"))?;

        Ok(Self {
            places,
            regions,
            routes_geojson,
            railway_geojson,
        })
    }

    /// Filter places by optional region slug and case-insensitive search query.
    pub fn filter(&self, region: Option<&str>, query: Option<&str>) -> Vec<&Place> {
        let region_filter = region.map(str::trim).filter(|s| !s.is_empty() && *s != "all");
        let q = query
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty());

        self.places
            .iter()
            .filter(|place| {
                if let Some(reg) = region_filter {
                    if place.meta.region != reg {
                        return false;
                    }
                }
                if let Some(ref needle) = q {
                    let in_title = place.meta.title.to_lowercase().contains(needle);
                    let in_summary = place.meta.summary.to_lowercase().contains(needle);
                    let in_region = place.meta.region_name.to_lowercase().contains(needle);
                    let in_tags = place
                        .meta
                        .tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(needle));
                    return in_title || in_summary || in_region || in_tags;
                }
                true
            })
            .collect()
    }

    pub fn find_by_slug(&self, slug: &str) -> Option<&Place> {
        self.places.iter().find(|p| p.meta.slug == slug)
    }

    /// Build a GeoJSON FeatureCollection for all places (used by the interactive map).
    pub fn places_geojson(&self, region: Option<&str>) -> Value {
        let features: Vec<Value> = self
            .filter(region, None)
            .into_iter()
            .map(|p| {
                json!({
                    "type": "Feature",
                    "geometry": {
                        "type": "Point",
                        "coordinates": [p.meta.lng, p.meta.lat]
                    },
                    "properties": {
                        "slug": p.meta.slug,
                        "title": p.meta.title,
                        "region": p.meta.region,
                        "region_name": p.meta.region_name,
                        "category": p.meta.category,
                        "summary": p.meta.summary,
                        "url": format!("/places/{}", p.meta.slug)
                    }
                })
            })
            .collect();

        json!({
            "type": "FeatureCollection",
            "features": features
        })
    }
}

fn load_places(dir: &Path) -> Result<Vec<Place>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut places = Vec::new();
    let entries = fs::read_dir(dir).map_err(|e| format!("Failed to read {}: {e}", dir.display()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("md") {
            let raw = fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
            let place = parse_markdown_place(&raw)
                .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
            places.push(place);
        }
    }

    places.sort_by(|a, b| {
        b.meta
            .lat
            .total_cmp(&a.meta.lat)
            .then_with(|| a.meta.title.to_lowercase().cmp(&b.meta.title.to_lowercase()))
    });
    Ok(places)
}

fn parse_markdown_place(raw: &str) -> Result<Place, String> {
    let trimmed = raw.trim_start();
    let rest = trimmed
        .strip_prefix("+++")
        .ok_or_else(|| "Missing opening +++ TOML frontmatter delimiter".to_string())?;

    let (frontmatter_str, markdown_body) = rest
        .split_once("+++")
        .ok_or_else(|| "Missing closing +++ TOML frontmatter delimiter".to_string())?;

    let meta: PlaceFrontmatter =
        toml::from_str(frontmatter_str.trim()).map_err(|e| format!("Invalid TOML frontmatter: {e}"))?;

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let parser = Parser::new_ext(markdown_body.trim(), options);
    let mut html_body = String::new();
    html::push_html(&mut html_body, parser);

    Ok(Place { meta, html_body })
}

fn build_region_index(places: &[Place]) -> Vec<RegionInfo> {
    let mut map: std::collections::HashMap<String, (String, usize, f64)> =
        std::collections::HashMap::new();
    for p in places {
        let entry = map
            .entry(p.meta.region.clone())
            .or_insert_with(|| (p.meta.region_name.clone(), 0, 0.0));
        entry.1 += 1;
        entry.2 += p.meta.lat;
    }
    let mut regions: Vec<(RegionInfo, f64)> = map
        .into_iter()
        .map(|(slug, (name, count, lat_sum))| {
            let avg_lat = lat_sum / count as f64;
            (RegionInfo { slug, name, count }, avg_lat)
        })
        .collect();
    // Sort regions North to South (highest average latitude first)
    regions.sort_by(|a, b| b.1.total_cmp(&a.1));
    regions.into_iter().map(|(info, _)| info).collect()
}

fn load_routes_geojson(dir: &Path) -> Result<Value, String> {
    let mut all_features: Vec<Value> = Vec::new();
    if dir.exists() {
        let entries =
            fs::read_dir(dir).map_err(|e| format!("Failed to read {}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("geojson") {
                let raw = fs::read_to_string(&path)
                    .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
                let val: Value = serde_json::from_str(&raw)
                    .map_err(|e| format!("Invalid GeoJSON in {}: {e}", path.display()))?;
                if let Some(features) = val.get("features").and_then(|f| f.as_array()) {
                    all_features.extend(features.iter().cloned());
                }
            }
        }
    }

    Ok(json!({
        "type": "FeatureCollection",
        "features": all_features
    }))
}

fn load_single_geojson(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({
            "type": "FeatureCollection",
            "features": []
        }));
    }
    let raw =
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("Invalid GeoJSON in {}: {e}", path.display()))
}
