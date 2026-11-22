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

  async function fetchJson(url) {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    return res.json();
  }

  // 1. Home page: batched weather forecast strip for hubs (country-aware, North -> South)
  const stripEl = document.getElementById("laos-weather-strip");
  if (stripEl) {
    const activeCountry = stripEl.dataset.activeCountry || "all";
    const hubsByCountry = {
      laos: [
        { name: "Nong Khiaw", url: "/?country=laos&region=nong-khiaw" },
        { name: "Luang Prabang", url: "/?country=laos&region=luang-prabang" },
        { name: "Vang Vieng", url: "/?country=laos&region=vang-vieng" },
        { name: "Vientiane", url: "/?country=laos&region=vientiane" },
        { name: "Thakhek", url: "/?country=laos&region=khammouane" },
        { name: "Pakse / Bolaven", url: "/?country=laos&region=champasak" },
      ],
      malaysia: [
        { name: "Mt. Kinabalu", url: "/?country=malaysia&region=sabah" },
        { name: "Kota Kinabalu", url: "/?country=malaysia&region=sabah" },
        { name: "Sandakan / Sepilok", url: "/?country=malaysia&region=sabah" },
        { name: "Miri · Brunei", url: "/?country=malaysia&region=sarawak" },
        { name: "Mulu Caves", url: "/?country=malaysia&region=sarawak" },
        { name: "Kuching", url: "/?country=malaysia&region=sarawak" },
      ],
      all: [
        { name: "Luang Prabang", url: "/?country=laos&region=luang-prabang" },
        { name: "Vang Vieng", url: "/?country=laos&region=vang-vieng" },
        { name: "Vientiane", url: "/?country=laos&region=vientiane" },
        { name: "Kota Kinabalu", url: "/?country=malaysia&region=sabah" },
        { name: "Miri · Brunei", url: "/?country=malaysia&region=sarawak" },
        { name: "Kuching", url: "/?country=malaysia&region=sarawak" },
      ],
    };

    const hubs = hubsByCountry[activeCountry] || hubsByCountry.all;

    fetchJson(`/api/weather/hubs?country=${encodeURIComponent(activeCountry)}`)
      .then((data) => {
        const list = Array.isArray(data) ? data : [data];
        const html = hubs
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
              <a href="${hub.url}" class="weather-chip" title="${label}">
                <span class="weather-chip-city">${hub.name}</span>
                <span class="weather-chip-main">${icon} <strong>${temp}°C</strong></span>
                <span class="weather-chip-sub">${lo}° / ${hi}° ${rainText}</span>
              </a>
            `;
          })
          .join("");

        if (!html.trim()) throw new Error("Empty weather payload");
        stripEl.innerHTML = html;
      })
      .catch((err) => {
        console.error("Weather hubs error:", err);
        stripEl.innerHTML = `<span class="weather-error">Live weather unavailable right now.</span>`;
      });
  }

  // 2. Destination detail page: current weather + 5-day forecast
  const placeEl = document.getElementById("place-weather");
  if (placeEl) {
    const lat = placeEl.dataset.lat;
    const lng = placeEl.dataset.lng;
    const url = `/api/weather/place?lat=${encodeURIComponent(lat)}&lng=${encodeURIComponent(lng)}`;

    fetchJson(url)
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
      .catch((err) => {
        console.error("Place weather error:", err);
        placeEl.innerHTML = `<p class="weather-error">Could not load live forecast.</p>`;
      });
  }

  // 3. Destination detail page: Full-Year Climate Chart (Jan–Dec Temperature + Precipitation)
  const climateEl = document.getElementById("place-climate-chart");
  if (climateEl) {
    const lat = climateEl.dataset.lat;
    const lng = climateEl.dataset.lng;
    const url = `/api/weather/climate?lat=${encodeURIComponent(lat)}&lng=${encodeURIComponent(lng)}`;

    fetchJson(url)
      .then((data) => {
        const months = data.months || [];
        if (months.length !== 12) throw new Error("Expected 12 months");

        const W = 640;
        const H = 240;
        const padL = 38;
        const padR = 44;
        const padT = 16;
        const padB = 30;
        const plotW = W - padL - padR;
        const plotH = H - padT - padB;

        const maxRain = Math.max(100, ...months.map((m) => m.rain_mm));
        const maxTemp = Math.ceil(Math.max(35, ...months.map((m) => m.avg_high)) / 5) * 5;
        const minTemp = Math.floor(Math.min(10, ...months.map((m) => m.avg_low)) / 5) * 5;

        const stepX = plotW / 12;
        const xFor = (i) => padL + stepX * i + stepX / 2;
        const yTemp = (t) => padT + plotH - ((t - minTemp) / (maxTemp - minTemp)) * plotH;
        const yRain = (r) => padT + plotH - (r / maxRain) * plotH;

        // Horizontal grid lines (temperature ticks)
        const gridLines = [];
        for (let t = minTemp; t <= maxTemp; t += 10) {
          const y = yTemp(t);
          gridLines.push(
            `<line x1="${padL}" y1="${y}" x2="${W - padR}" y2="${y}" stroke="#e7e5e4" stroke-dasharray="3 3" />`,
            `<text x="${padL - 6}" y="${y + 4}" text-anchor="end" font-size="10" fill="#78716c">${t}°</text>`
          );
        }

        // Right axis rainfall ticks
        const rainTicks = [0, Math.round(maxRain / 2), Math.round(maxRain)].map((r) => {
          const y = yRain(r);
          return `<text x="${W - padR + 6}" y="${y + 4}" text-anchor="start" font-size="10" fill="#0284c7">${r}mm</text>`;
        });

        // Precipitation bars
        const barW = stepX * 0.52;
        const bars = months
          .map((m, i) => {
            const x = xFor(i) - barW / 2;
            const y = yRain(m.rain_mm);
            const h = padT + plotH - y;
            return `
              <rect x="${x.toFixed(1)}" y="${y.toFixed(1)}" width="${barW.toFixed(1)}" height="${Math.max(1, h).toFixed(1)}" rx="3" fill="#bae6fd">
                <title>${m.month}: ${m.rain_mm} mm rain · High ${m.avg_high}°C / Low ${m.avg_low}°C</title>
              </rect>
            `;
          })
          .join("");

        const highPoints = months.map((m, i) => `${xFor(i).toFixed(1)},${yTemp(m.avg_high).toFixed(1)}`).join(" ");
        const lowPoints = months.map((m, i) => `${xFor(i).toFixed(1)},${yTemp(m.avg_low).toFixed(1)}`).join(" ");

        const dots = months
          .map((m, i) => {
            const x = xFor(i).toFixed(1);
            const yh = yTemp(m.avg_high).toFixed(1);
            const yl = yTemp(m.avg_low).toFixed(1);
            return `
              <circle cx="${x}" cy="${yh}" r="3.5" fill="#ea580c"><title>${m.month} Avg High: ${m.avg_high}°C</title></circle>
              <circle cx="${x}" cy="${yl}" r="3.5" fill="#0f766e"><title>${m.month} Avg Low: ${m.avg_low}°C</title></circle>
              <text x="${x}" y="${H - 10}" text-anchor="middle" font-size="11" fill="#57534e">${m.month}</text>
            `;
          })
          .join("");

        const hottest = months.reduce((a, b) => (b.avg_high > a.avg_high ? b : a));
        const coolest = months.reduce((a, b) => (b.avg_low < a.avg_low ? b : a));
        const wettest = months.reduce((a, b) => (b.rain_mm > a.rain_mm ? b : a));

        climateEl.innerHTML = `
          <div class="climate-svg-wrap">
            <svg viewBox="0 0 ${W} ${H}" class="climate-svg" role="img" aria-label="Full-year monthly temperature and precipitation chart">
              ${gridLines.join("")}
              ${rainTicks.join("")}
              ${bars}
              <polyline fill="none" stroke="#ea580c" stroke-width="2.5" points="${highPoints}" />
              <polyline fill="none" stroke="#0f766e" stroke-width="2.5" points="${lowPoints}" />
              ${dots}
            </svg>
          </div>
          <div class="climate-highlights">
            <span>🔥 Warmest: <strong>${hottest.month}</strong> (${hottest.avg_high}°C)</span>
            <span>❄️ Coolest: <strong>${coolest.month}</strong> (${coolest.avg_low}°C)</span>
            <span>🌧️ Wettest: <strong>${wettest.month}</strong> (${wettest.rain_mm} mm)</span>
          </div>
        `;
      })
      .catch((err) => {
        console.error("Climate chart error:", err);
        climateEl.innerHTML = `<p class="weather-error">Could not load full-year climate data.</p>`;
      });
  }
})();
