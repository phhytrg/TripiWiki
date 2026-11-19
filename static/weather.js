(function initWeather() {
  const WMO = {
    0: ["☀️", "Clear sky"],
    1: ["🌤️", "Mainly clear"],
    2: ["⛅", "Partly cloudy"],
    3: ["☁️", "Overcast"],
    45: ["🌫️", "Fog"],
    48: ["🌫️", "Rime fog"],
    51: ["🌦️", "Light drizzle"],
    53: ["🌦️", "Drizzle"],
    55: ["🌧️", "Dense drizzle"],
    61: ["🌦️", "Light rain"],
    63: ["🌧️", "Moderate rain"],
    65: ["🌧️", "Heavy rain"],
    80: ["🌦️", "Rain showers"],
    81: ["🌧️", "Moderate showers"],
    82: ["⛈️", "Heavy showers"],
    95: ["⛈️", "Thunderstorm"],
    96: ["⛈️", "Storm & hail"],
    99: ["⛈️", "Heavy storm"],
  };

  function describeCode(code) {
    return WMO[code] || ["🌤️", "Fair"];
  }

  function formatDay(dateStr, index) {
    if (index === 0) return "Today";
    const d = new Date(dateStr + "T00:00:00");
    return d.toLocaleDateString("en-US", { weekday: "short" });
  }

  // 1. Home page: batched live weather strip for major Laos hubs (North -> South)
  const stripEl = document.getElementById("laos-weather-strip");
  if (stripEl) {
    const hubs = [
      { name: "Nong Khiaw", region: "nong-khiaw", lat: 20.5718, lng: 102.6134 },
      { name: "Luang Prabang", region: "luang-prabang", lat: 19.8895, lng: 102.1347 },
      { name: "Vang Vieng", region: "vang-vieng", lat: 18.9233, lng: 102.4114 },
      { name: "Vientiane", region: "vientiane", lat: 17.9757, lng: 102.6365 },
      { name: "Thakhek", region: "khammouane", lat: 17.4089, lng: 104.8306 },
      { name: "Pakse / Bolaven", region: "champasak", lat: 15.1823, lng: 106.1268 },
    ];

    const lats = hubs.map((h) => h.lat).join(",");
    const lngs = hubs.map((h) => h.lng).join(",");
    const url =
      `https://api.open-meteo.com/v1/forecast?latitude=${lats}&longitude=${lngs}` +
      `&current=temperature_2m,weather_code&daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max` +
      `&timezone=Asia%2FBangkok&forecast_days=1`;

    fetch(url)
      .then((res) => res.json())
      .then((data) => {
        const list = Array.isArray(data) ? data : [data];
        stripEl.innerHTML = hubs
          .map((hub, idx) => {
            const item = list[idx];
            if (!item || !item.current) return "";
            const [icon, label] = describeCode(item.current.weather_code);
            const temp = Math.round(item.current.temperature_2m);
            const hi = Math.round(item.daily?.temperature_2m_max?.[0] ?? temp);
            const lo = Math.round(item.daily?.temperature_2m_min?.[0] ?? temp);
            const rain = item.daily?.precipitation_probability_max?.[0];
            const rainText = rain != null ? `💧 ${rain}%` : "";
            return `
              <a href="/?region=${hub.region}" class="weather-chip" title="${label}">
                <span class="weather-chip-city">${hub.name}</span>
                <span class="weather-chip-main">${icon} <strong>${temp}°C</strong></span>
                <span class="weather-chip-sub">${lo}° / ${hi}° ${rainText}</span>
              </a>
            `;
          })
          .join("");
      })
      .catch(() => {
        stripEl.innerHTML = `<span class="weather-error">Live weather unavailable right now.</span>`;
      });
  }

  // 2. Destination detail page: current weather + 5-day forecast
  const placeEl = document.getElementById("place-weather");
  if (placeEl) {
    const lat = placeEl.dataset.lat;
    const lng = placeEl.dataset.lng;
    const url =
      `https://api.open-meteo.com/v1/forecast?latitude=${encodeURIComponent(lat)}&longitude=${encodeURIComponent(lng)}` +
      `&current=temperature_2m,relative_humidity_2m,weather_code,wind_speed_10m` +
      `&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max` +
      `&timezone=Asia%2FBangkok&forecast_days=5`;

    fetch(url)
      .then((res) => res.json())
      .then((data) => {
        if (!data.current || !data.daily) {
          throw new Error("Incomplete weather response");
        }
        const [icon, label] = describeCode(data.current.weather_code);
        const temp = Math.round(data.current.temperature_2m);
        const humidity = Math.round(data.current.relative_humidity_2m);
        const wind = Math.round(data.current.wind_speed_10m);

        const rows = data.daily.time
          .map((dateStr, i) => {
            const [dIcon, dLabel] = describeCode(data.daily.weather_code[i]);
            const hi = Math.round(data.daily.temperature_2m_max[i]);
            const lo = Math.round(data.daily.temperature_2m_min[i]);
            const rain = data.daily.precipitation_probability_max?.[i] ?? 0;
            return `
              <div class="forecast-row" title="${dLabel}">
                <span class="forecast-day">${formatDay(dateStr, i)}</span>
                <span class="forecast-icon">${dIcon}</span>
                <span class="forecast-rain">💧 ${rain}%</span>
                <span class="forecast-temps"><strong>${hi}°</strong> / ${lo}°</span>
              </div>
            `;
          })
          .join("");

        placeEl.innerHTML = `
          <div class="weather-current">
            <div class="weather-now-left">
              <span class="weather-big-icon">${icon}</span>
              <div>
                <div class="weather-big-temp">${temp}°C</div>
                <div class="weather-cond">${label}</div>
              </div>
            </div>
            <div class="weather-meta">
              <div>Humidity: ${humidity}%</div>
              <div>Wind: ${wind} km/h</div>
            </div>
          </div>
          <div class="forecast-list">${rows}</div>
        `;
      })
      .catch(() => {
        placeEl.innerHTML = `<p class="weather-error">Could not load live forecast.</p>`;
      });
  }
})();
