(async function initLaosMap() {
  const mapEl = document.getElementById("laos-map");
  if (!mapEl) return;

  const activeRegion = mapEl.dataset.activeRegion || "all";
  const map = L.map("laos-map").setView([18.5, 103.5], 6);

  L.tileLayer(
    "https://server.arcgisonline.com/ArcGIS/rest/services/World_Street_Map/MapServer/tile/{z}/{y}/{x}",
    {
      attribution: "Tiles &copy; Esri",
      maxZoom: 19,
    }
  ).addTo(map);

  const placesGroup = L.layerGroup().addTo(map);
  const routesGroup = L.layerGroup().addTo(map);
  const railwayLinesGroup = L.layerGroup().addTo(map);
  const stationsGroup = L.layerGroup().addTo(map);
  const markersBySlug = new Map();

  const isMobile = window.matchMedia("(max-width: 640px)").matches;

  L.control
    .layers(
      null,
      {
        '<span class="legend-dot legend-pin"></span> Destinations': placesGroup,
        '<span class="legend-line legend-dashed"></span> Travel Routes': routesGroup,
        '<span class="legend-line legend-rail"></span> Railway Lines': railwayLinesGroup,
        '<span class="legend-dot legend-station"></span> Train Stations': stationsGroup,
      },
      { collapsed: isMobile, position: "topright" }
    )
    .addTo(map);

  // Load dashed route polylines
  try {
    const routesRes = await fetch("/api/routes.geojson");
    const routesData = await routesRes.json();
    L.geoJSON(routesData, {
      style: (feature) => ({
        color: feature.properties?.color || "#0f766e",
        weight: 3,
        dashArray: "6 4",
        opacity: 0.85,
      }),
      onEachFeature: (feature, layer) => {
        if (feature.properties?.name) {
          layer.bindPopup(
            `<strong>${feature.properties.name}</strong><br>${feature.properties.description || ""}`
          );
        }
      },
    }).addTo(routesGroup);
  } catch (err) {
    console.warn("Could not load routes:", err);
  }

  // Load railway lines & stations into separate toggleable layers
  try {
    const railRes = await fetch("/api/railway.geojson");
    const railData = await railRes.json();
    const features = railData.features || [];

    const lineFeatures = features.filter((f) => f.geometry?.type !== "Point");
    const stationFeatures = features.filter((f) => f.geometry?.type === "Point");

    L.geoJSON(
      { type: "FeatureCollection", features: lineFeatures },
      {
        style: (feature) => ({
          color: feature.properties?.color || "#2563eb",
          weight: 3,
          opacity: 0.9,
        }),
        onEachFeature: (feature, layer) => {
          const p = feature.properties || {};
          if (p.name) {
            layer.bindPopup(`<strong>🚆 ${p.name}</strong><br>${p.description || ""}`);
          }
        },
      }
    ).addTo(railwayLinesGroup);

    L.geoJSON(
      { type: "FeatureCollection", features: stationFeatures },
      {
        pointToLayer: (_feature, latlng) =>
          L.circleMarker(latlng, {
            radius: 5,
            fillColor: "#2563eb",
            color: "#ffffff",
            weight: 1.5,
            fillOpacity: 1,
          }),
        onEachFeature: (feature, layer) => {
          const p = feature.properties || {};
          layer.bindPopup(`<strong>🚆 ${p.name}</strong><br>${p.transfer || ""}`);
        },
      }
    ).addTo(stationsGroup);
  } catch (err) {
    console.warn("Could not load railway:", err);
  }

  // Load place markers
  try {
    const url =
      activeRegion && activeRegion !== "all"
        ? `/api/places.geojson?region=${encodeURIComponent(activeRegion)}`
        : "/api/places.geojson";
    const placesRes = await fetch(url);
    const placesData = await placesRes.json();

    const layer = L.geoJSON(placesData, {
      onEachFeature: (feature, marker) => {
        const p = feature.properties;
        markersBySlug.set(p.slug, marker);
        marker.bindPopup(`
          <div class="map-popup">
            <span class="popup-region">${p.region_name}</span>
            <h4><a href="${p.url}">${p.title}</a></h4>
            <p>${p.summary}</p>
            <a href="${p.url}" class="popup-link">Read guide &rarr;</a>
          </div>
        `);
      },
    }).addTo(placesGroup);

    if (placesData.features && placesData.features.length > 0) {
      map.fitBounds(layer.getBounds(), { padding: [40, 40], maxZoom: 11 });
    }
  } catch (err) {
    console.warn("Could not load places:", err);
  }

  // First tap/click on a card -> focus it on the map;
  // double-click (or second tap on an already-selected card on touch devices) -> open its guide page
  const isTouchDevice = window.matchMedia("(pointer: coarse)").matches;

  document.addEventListener("click", (event) => {
    if (event.target.closest(".carousel-dot")) return;
    const card = event.target.closest(".place-card");
    if (!card) return;
    if (event.metaKey || event.ctrlKey) return;

    const slug = card.dataset.slug;
    const alreadySelected = card.classList.contains("selected");

    if (isTouchDevice && alreadySelected && slug) {
      window.location.href = `/places/${encodeURIComponent(slug)}`;
      return;
    }

    event.preventDefault();

    document
      .querySelectorAll(".place-card.selected")
      .forEach((el) => el.classList.remove("selected"));
    card.classList.add("selected");

    const lat = parseFloat(card.dataset.lat);
    const lng = parseFloat(card.dataset.lng);

    if (!map.hasLayer(placesGroup)) {
      placesGroup.addTo(map);
    }

    const marker = markersBySlug.get(slug);
    if (!Number.isNaN(lat) && !Number.isNaN(lng)) {
      map.panTo([lat, lng], { animate: true, duration: 0.5 });
    }
    if (marker) {
      marker.openPopup();
    }

    mapEl.scrollIntoView({ behavior: "smooth", block: "nearest" });
  });

  document.addEventListener("dblclick", (event) => {
    if (event.target.closest(".carousel-dot")) return;
    const card = event.target.closest(".place-card");
    if (!card || !card.dataset.slug) return;

    window.location.href = `/places/${encodeURIComponent(card.dataset.slug)}`;
  });
})();
