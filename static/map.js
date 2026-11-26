(async function initLaosMap() {
  const mapEl = document.getElementById("laos-map");
  if (!mapEl) return;

  const activeCountry = mapEl.dataset.activeCountry || "all";
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
  const customRouteGroup = L.layerGroup().addTo(map);
  const railwayLinesGroup = L.layerGroup().addTo(map);
  const stationsGroup = L.layerGroup().addTo(map);
  const markersBySlug = new Map();

  const isMobile = window.matchMedia("(max-width: 640px)").matches;

  L.control
    .layers(
      null,
      {
        '<span class="legend-dot legend-pin"></span> Destinations': placesGroup,
        '<span class="legend-line legend-custom"></span> My Trip Route': customRouteGroup,
        '<span class="legend-line legend-rail"></span> Railway Lines': railwayLinesGroup,
        '<span class="legend-dot legend-station"></span> Train Stations': stationsGroup,
      },
      { collapsed: isMobile, position: "topright" }
    )
    .addTo(map);

  // Speed-tier styling (scales cleanly to any number of countries without running out of colors)
  function railStyleBySpeed(kmh) {
    if (kmh >= 250) return { color: "#dc2626", weight: 4.5, dashArray: null, opacity: 0.92 };
    if (kmh >= 140) return { color: "#2563eb", weight: 3.5, dashArray: null, opacity: 0.9 };
    if (kmh >= 90) return { color: "#7c3aed", weight: 2.8, dashArray: null, opacity: 0.85 };
    return { color: "#059669", weight: 2.5, dashArray: "6 4", opacity: 0.88 };
  }

  // Load real OSM railway lines & stations (scoped to active country if selected)
  try {
    const railUrl =
      activeCountry && activeCountry !== "all"
        ? `/api/railway.geojson?country=${encodeURIComponent(activeCountry)}`
        : "/api/railway.geojson";
    const railRes = await fetch(railUrl);
    const railData = await railRes.json();
    const features = railData.features || [];

    const lineFeatures = features.filter((f) => f.geometry?.type !== "Point");
    const stationFeatures = features.filter((f) => f.geometry?.type === "Point");

    L.geoJSON(
      { type: "FeatureCollection", features: lineFeatures },
      {
        style: (feature) => railStyleBySpeed(feature.properties?.max_speed_kmh || 100),
        onEachFeature: (feature, layer) => {
          const p = feature.properties || {};
          const baseStyle = railStyleBySpeed(p.max_speed_kmh || 100);
          if (p.name) {
            const spd = p.speed ? `<br><strong>⚡ Speed:</strong> ${p.speed}` : "";
            layer.bindPopup(`<strong>🚆 ${p.name}</strong>${spd}<br>${p.description || ""}`);
            layer.bindTooltip(`🚆 ${p.name}${p.speed ? ` · ${p.speed}` : ""}`, { sticky: true });
          }
          layer.on("mouseover", () => layer.setStyle({ weight: baseStyle.weight + 2, opacity: 1 }));
          layer.on("mouseout", () => layer.setStyle(baseStyle));
        },
      }
    ).addTo(railwayLinesGroup);

    L.geoJSON(
      { type: "FeatureCollection", features: stationFeatures },
      {
        pointToLayer: (feature, latlng) => {
          const st = railStyleBySpeed(feature.properties?.max_speed_kmh || 100);
          return L.circleMarker(latlng, {
            radius: 5,
            fillColor: st.color,
            color: "#ffffff",
            weight: 1.5,
            fillOpacity: 1,
          });
        },
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
    const params = new URLSearchParams();
    if (activeCountry && activeCountry !== "all") params.set("country", activeCountry);
    if (activeRegion && activeRegion !== "all") params.set("region", activeRegion);
    const qs = params.toString();
    const url = qs ? `/api/places.geojson?${qs}` : "/api/places.geojson";

    const placesRes = await fetch(url);
    const placesData = await placesRes.json();

    const layer = L.geoJSON(placesData, {
      onEachFeature: (feature, marker) => {
        const p = feature.properties;
        const [lng, lat] = feature.geometry.coordinates;
        const elevBadge = p.elevation_m ? ` · ⛰️ ${p.elevation_m} m` : "";
        const imgHtml = p.image
          ? `<a href="${p.url}" class="popup-img-wrap"><img src="${p.image}" alt="${p.title.replace(/"/g, "&quot;")}" class="popup-img" loading="lazy" /></a>`
          : "";
        markersBySlug.set(p.slug, marker);
        marker.bindPopup(
          `
          <div class="map-popup">
            ${imgHtml}
            <span class="popup-region">${p.country_name} · ${p.region_name}${elevBadge}</span>
            <h4><a href="${p.url}">${p.title}</a></h4>
            <p class="popup-summary">${p.summary}</p>
            <div class="popup-actions">
              <a href="${p.url}" class="popup-link">Read guide &rarr;</a>
              <button
                type="button"
                class="route-toggle-btn popup-route-btn"
                data-slug="${p.slug}"
                data-title="${p.title.replace(/"/g, "&quot;")}"
                data-lat="${lat}"
                data-lng="${lng}"
              >+ Route</button>
            </div>
          </div>
        `,
          {
            maxWidth: 270,
            autoPanPaddingTopLeft: L.point(16, 16),
            autoPanPaddingBottomRight: L.point(16, 16),
          }
        );
      },
    }).addTo(placesGroup);

    if (placesData.features && placesData.features.length > 0) {
      map.fitBounds(layer.getBounds(), { padding: [40, 40], maxZoom: 11 });
    }
  } catch (err) {
    console.warn("Could not load places:", err);
  }

  // --- Interactive Trip Route Builder ---
  const selectedStops = [];
  const stopsEl = document.getElementById("route-stops");
  const summaryEl = document.getElementById("route-summary");
  const sortNsBtn = document.getElementById("route-sort-ns");
  const clearBtn = document.getElementById("route-clear");

  function formatDuration(minutes) {
    if (minutes < 60) return `${minutes} min`;
    const h = Math.floor(minutes / 60);
    const m = minutes % 60;
    return m > 0 ? `${h}h ${m}m` : `${h}h`;
  }

  function syncRouteButtons() {
    const slugs = new Set(selectedStops.map((s) => s.slug));
    document.querySelectorAll(".route-toggle-btn").forEach((btn) => {
      const inRoute = slugs.has(btn.dataset.slug);
      btn.classList.toggle("active", inRoute);
      btn.textContent = inRoute ? "✓ In Route" : "+ Route";
    });
  }

  async function renderCustomRoute() {
    syncRouteButtons();
    customRouteGroup.clearLayers();

    if (stopsEl) {
      stopsEl.innerHTML = selectedStops
        .map(
          (stop, idx) => `
          <span class="route-stop-chip">
            <span class="route-stop-num">${idx + 1}</span>
            ${stop.title}
            <button type="button" class="route-stop-remove" data-remove-slug="${stop.slug}" aria-label="Remove ${stop.title}">&times;</button>
          </span>
        `
        )
        .join("");
    }

    if (sortNsBtn) sortNsBtn.disabled = selectedStops.length < 2;
    if (clearBtn) clearBtn.disabled = selectedStops.length === 0;

    // Draw numbered markers for selected stops
    selectedStops.forEach((stop, idx) => {
      L.marker([stop.lat, stop.lng], {
        icon: L.divIcon({
          className: "route-pin-badge",
          html: `<span>${idx + 1}</span>`,
          iconSize: [26, 26],
          iconAnchor: [13, 13],
        }),
        zIndexOffset: 500,
      }).addTo(customRouteGroup);
    });

    if (selectedStops.length === 0) {
      if (summaryEl) {
        summaryEl.innerHTML =
          "Click <strong>+ Route</strong> on 2 or more places to generate a custom driving route.";
      }
      return;
    }

    if (selectedStops.length === 1) {
      if (summaryEl) {
        summaryEl.textContent = "1 stop selected — pick at least 1 more place to generate a route.";
      }
      return;
    }

    if (summaryEl) {
      summaryEl.textContent = "Generating route...";
    }

    const coords = selectedStops.map((s) => `${s.lng},${s.lat}`).join(";");
    try {
      const res = await fetch(`/api/route/generate?coords=${encodeURIComponent(coords)}`);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();

      const routeLayer = L.geoJSON(data.geometry, {
        style: {
          color: "#ea580c",
          weight: 4,
          opacity: 0.9,
          dashArray: data.mode === "direct" ? "8 6" : null,
        },
      }).addTo(customRouteGroup);

      if (!map.hasLayer(customRouteGroup)) {
        customRouteGroup.addTo(map);
      }

      map.fitBounds(routeLayer.getBounds(), { padding: [45, 45], maxZoom: 12 });

      if (summaryEl) {
        const modeLabel = data.mode === "driving" ? "Road route" : "Estimated direct route";
        summaryEl.innerHTML = `<strong>${selectedStops.length} stops</strong> · ${modeLabel}: <strong>${data.distance_km} km</strong> (~${formatDuration(data.duration_min)})`;
      }
    } catch (err) {
      console.error("Route generation failed:", err);
      if (summaryEl) {
        summaryEl.textContent = "Could not generate route right now.";
      }
    }
  }

  function toggleStop(stop) {
    const idx = selectedStops.findIndex((s) => s.slug === stop.slug);
    if (idx >= 0) {
      selectedStops.splice(idx, 1);
    } else {
      selectedStops.push(stop);
    }
    renderCustomRoute();
  }

  if (sortNsBtn) {
    sortNsBtn.addEventListener("click", () => {
      selectedStops.sort((a, b) => b.lat - a.lat);
      renderCustomRoute();
    });
  }

  if (clearBtn) {
    clearBtn.addEventListener("click", () => {
      selectedStops.length = 0;
      renderCustomRoute();
    });
  }

  // Keep button active states synced after HTMX search/filter swaps
  document.body.addEventListener("htmx:afterSwap", () => {
    syncRouteButtons();
    initProvinceMiniMaps();
  });

  // --- Province Section Locator Mini-Maps (lazy-loaded via IntersectionObserver) ---
  const countryCenters = {
    laos: { lat: 18.2, lng: 103.8, zoom: 4 },
    malaysia: { lat: 4.0, lng: 114.2, zoom: 4 },
  };

  function mountMiniMap(el) {
    if (el.dataset.mapInit === "1") return;
    el.dataset.mapInit = "1";

    const lat = parseFloat(el.dataset.lat);
    const lng = parseFloat(el.dataset.lng);
    if (Number.isNaN(lat) || Number.isNaN(lng)) return;

    const ctx = countryCenters[el.dataset.country] || { lat, lng, zoom: 5 };

    const mini = L.map(el, {
      center: [ctx.lat, ctx.lng],
      zoom: ctx.zoom,
      zoomControl: false,
      attributionControl: false,
      dragging: false,
      scrollWheelZoom: false,
      doubleClickZoom: false,
      boxZoom: false,
      keyboard: false,
      touchZoom: false,
    });

    L.tileLayer(
      "https://server.arcgisonline.com/ArcGIS/rest/services/World_Street_Map/MapServer/tile/{z}/{y}/{x}",
      { maxZoom: 12 }
    ).addTo(mini);

    // Outer halo + crisp location dot showing where this province sits in the country
    L.circleMarker([lat, lng], {
      radius: 11,
      color: "#0f766e",
      weight: 1.5,
      fillColor: "#14b8a6",
      fillOpacity: 0.28,
      interactive: false,
    }).addTo(mini);

    L.circleMarker([lat, lng], {
      radius: 4.5,
      color: "#ffffff",
      weight: 1.5,
      fillColor: "#ea580c",
      fillOpacity: 1,
      interactive: false,
    }).addTo(mini);

    el.addEventListener("click", () => {
      map.panTo([lat, lng], { animate: true, duration: 0.5 });
      mapEl.scrollIntoView({ behavior: "smooth", block: "nearest" });
    });
  }

  let miniMapObserver = null;
  function initProvinceMiniMaps() {
    const els = document.querySelectorAll(".province-minimap:not([data-map-init='1'])");
    if (!els.length) return;

    if ("IntersectionObserver" in window) {
      if (!miniMapObserver) {
        miniMapObserver = new IntersectionObserver(
          (entries) => {
            entries.forEach((entry) => {
              if (entry.isIntersecting) {
                mountMiniMap(entry.target);
                miniMapObserver.unobserve(entry.target);
              }
            });
          },
          { rootMargin: "160px" }
        );
      }
      els.forEach((el) => miniMapObserver.observe(el));
    } else {
      els.forEach(mountMiniMap);
    }
  }

  initProvinceMiniMaps();

  // Clicking "📍 Map" locates the place on the map; clicking the card opens its detail page
  document.addEventListener("click", (event) => {
    // 1. Handle "+ Route" button clicks (on cards or inside map popups)
    const routeBtn = event.target.closest(".route-toggle-btn");
    if (routeBtn) {
      event.preventDefault();
      event.stopPropagation();
      toggleStop({
        slug: routeBtn.dataset.slug,
        title: routeBtn.dataset.title,
        lat: parseFloat(routeBtn.dataset.lat),
        lng: parseFloat(routeBtn.dataset.lng),
      });
      return;
    }

    // 2. Handle "📍 Map" locate button on cards
    const locateBtn = event.target.closest(".locate-map-btn");
    if (locateBtn) {
      event.preventDefault();
      event.stopPropagation();

      const card = locateBtn.closest(".place-card");
      document
        .querySelectorAll(".place-card.selected")
        .forEach((el) => el.classList.remove("selected"));
      if (card) card.classList.add("selected");

      const slug = locateBtn.dataset.slug;
      const lat = parseFloat(locateBtn.dataset.lat);
      const lng = parseFloat(locateBtn.dataset.lng);

      if (!map.hasLayer(placesGroup)) {
        placesGroup.addTo(map);
      }

      const marker = markersBySlug.get(slug);
      // Scroll smoothly so the entire map is visible right below the sticky header
      const headerHeight = document.querySelector(".site-header")?.offsetHeight || 64;
      const targetY = Math.max(0, mapEl.getBoundingClientRect().top + window.scrollY - headerHeight - 12);
      window.scrollTo({ top: targetY, behavior: "smooth" });

      if (!Number.isNaN(lat) && !Number.isNaN(lng)) {
        if (marker) {
          map.once("moveend", () => marker.openPopup());
        }
        map.flyTo([lat, lng], Math.max(map.getZoom(), 12), { duration: 0.65 });
      } else if (marker) {
        marker.openPopup();
      }
      return;
    }

    // 3. Handle removing a stop chip
    const removeBtn = event.target.closest(".route-stop-remove");
    if (removeBtn) {
      const slug = removeBtn.dataset.removeSlug;
      const idx = selectedStops.findIndex((s) => s.slug === slug);
      if (idx >= 0) {
        selectedStops.splice(idx, 1);
        renderCustomRoute();
      }
      return;
    }

    // 4. Clicking anywhere else on a card navigates straight to its detail page
    if (
      event.target.closest(".carousel-dot") ||
      event.target.closest(".card-tag") ||
      event.target.closest(".img-retry-btn") ||
      event.target.closest("a")
    ) {
      return;
    }
    const card = event.target.closest(".place-card");
    if (!card || !card.dataset.slug) return;
    if (event.metaKey || event.ctrlKey) {
      window.open(`/places/${encodeURIComponent(card.dataset.slug)}`, "_blank");
      return;
    }

    window.location.href = `/places/${encodeURIComponent(card.dataset.slug)}`;
  });
})();
