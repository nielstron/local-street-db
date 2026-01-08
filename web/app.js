const statusEl = document.getElementById("status");
const resultsEl = document.getElementById("results");
const searchInput = document.getElementById("search");
const filtersEl = document.getElementById("filters");

const isLocalhost =
  window.location.hostname === "localhost" ||
  window.location.hostname === "127.0.0.1";
const LOOKUP_CONFIG = {
  maxResults: 80,
  shardPrefixLen: 3,
  shardBase: "street_trie",
  shardSuffix: ".packed.gz",
  shardRoot: isLocalhost
    ? `${window.location.origin}/build/shards`
    : "https://nielstron.github.io/local-street-db/build/shards",
};
const streetLookup = new StreetLookup(LOOKUP_CONFIG);
const MAX_RESULTS = LOOKUP_CONFIG.maxResults;
const SHARD_PREFIX_LEN = LOOKUP_CONFIG.shardPrefixLen;
let map = null;
let markersLayer = null;
let countriesPromise = null;

const KIND_LABELS = {
  0: { label: "Street", emoji: "🛣️" },
  1: { label: "Airport", emoji: "✈️" },
  2: { label: "Train station", emoji: "🚆" },
  3: { label: "Bus stop", emoji: "🚌" },
  4: { label: "Ferry terminal", emoji: "⛴️" },
  5: { label: "University", emoji: "🎓" },
  6: { label: "Museum", emoji: "🏛️" },
  7: { label: "Civic building", emoji: "🏛️" },
  8: { label: "Sight", emoji: "📍" },
  9: { label: "City", emoji: "🏙️" },
  10: { label: "Country", emoji: "🌍" },
  15: { label: "Other", emoji: "•" },
};
const KIND_ORDER = [0, 9, 10, 1, 2, 3, 4, 5, 6, 7, 8, 15];

async function loadCountries() {
  if (countriesPromise) return countriesPromise;
  countriesPromise = fetch("countries.json").then(async (response) => {
    if (!response.ok) {
      throw new Error("Failed to load countries");
    }
    return response.json();
  });
  return countriesPromise;
}

async function lookupCountryCode(code) {
  const normalized = code.trim().toUpperCase();
  if (normalized.length !== 2) return null;
  const countries = await loadCountries();
  return countries.find((entry) => entry.code === normalized) || null;
}

function renderFilters() {
  if (!filtersEl) return;
  filtersEl.innerHTML = "";
  for (const kind of KIND_ORDER) {
    const info = KIND_LABELS[kind];
    const label = document.createElement("label");
    label.className = "filter-item";
    label.title = info.label;
    label.setAttribute("aria-label", info.label);
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.value = String(kind);
    checkbox.checked = true;
    checkbox.addEventListener("change", updateAllowedKinds);
    const emoji = document.createElement("span");
    emoji.className = "filter-emoji";
    emoji.textContent = info.emoji;
    emoji.title = info.label;
    label.append(checkbox, emoji);
    filtersEl.appendChild(label);
  }
}

function updateAllowedKinds() {
  if (!filtersEl) return;
  const checked = Array.from(
    filtersEl.querySelectorAll("input[type=checkbox]:checked")
  ).map((el) => Number(el.value));
  streetLookup.setAllowedKinds(checked);
  updateSearch();
}

function initMap() {
  map = L.map("map", { zoomControl: true });
  markersLayer = L.layerGroup().addTo(map);
  L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", {
    maxZoom: 19,
    attribution: "&copy; OpenStreetMap contributors",
  }).addTo(map);
}

function clearMarkers() {
  markersLayer.clearLayers();
}

function addMarkers(entries) {
  clearMarkers();
  const latLngs = [];
  let maxZoom = null;
  for (const entry of entries) {
    const loc = entry.location;
    if (!loc) continue;
    const [lon, lat] = loc;
    const kind = entry.kindByte ?? 0;
    if (kind === 10) {
      maxZoom = maxZoom === null ? 7 : Math.min(maxZoom, 7);
    } else if (kind === 9) {
      maxZoom = maxZoom === null ? 12 : Math.min(maxZoom, 12);
    } else if (maxZoom === null) {
      maxZoom = 18;
    }
    const marker = L.circleMarker([lat, lon], {
      radius: 6,
      color: "#2f5d62",
      fillColor: "#2f5d62",
      fillOpacity: 0.75,
    });
    markersLayer.addLayer(marker);
    latLngs.push([lat, lon]);
  }
  if (latLngs.length) {
    map.fitBounds(latLngs, { padding: [40, 40], maxZoom: maxZoom ?? 14 });
  }
}

function renderResults(entries) {
  resultsEl.innerHTML = "";
  const limitedEntries = entries.slice(0, MAX_RESULTS);
  if (!limitedEntries.length) {
    resultsEl.textContent = "No matches";
    return [];
  }

  for (const entry of limitedEntries) {
    const div = document.createElement("div");
    div.className = "result-item";
    const cityText = entry.placeLabel || "Unknown city";

    const mainEl = document.createElement("div");
    mainEl.className = "result-main";

    const rowEl = document.createElement("div");
    rowEl.className = "result-row";

    const nameEl = document.createElement("span");
    nameEl.className = "result-name";
    const kind = KIND_LABELS[entry.kindByte] || KIND_LABELS[15];
    nameEl.textContent = entry.display;

    const cityEl = document.createElement("span");
    cityEl.className = "result-city";
    cityEl.textContent = `${cityText} · ${kind.label}`;

    const emojiEl = document.createElement("span");
    emojiEl.className = "result-emoji";
    emojiEl.textContent = kind.emoji;

    rowEl.append(nameEl, emojiEl);
    mainEl.append(rowEl, cityEl);
    div.append(mainEl);
    div.addEventListener("click", () => addMarkers([entry]));
    resultsEl.appendChild(div);
  }

  return limitedEntries;
}

async function updateSearch() {
  const rawValue = searchInput.value;
  const value = rawValue.trim();
  statusEl.textContent = "Searching…";

  if (value.length === 2 && !value.includes(",")) {
    try {
      const match = await lookupCountryCode(value);
      if (!match) {
        resultsEl.textContent = "No matches";
        statusEl.textContent = `No country for code ${value.toUpperCase()}`;
        clearMarkers();
        return;
      }
      const entry = {
        display: match.name,
        placeLabel: match.code,
        kindByte: 10,
        location: [match.lon, match.lat, 0, 0, 10],
      };
      statusEl.textContent = `Country code ${match.code}`;
      const entries = renderResults([entry]);
      addMarkers(entries);
      return;
    } catch (err) {
      console.error(err);
      resultsEl.textContent = "No matches";
      statusEl.textContent = "Failed to load countries";
      clearMarkers();
      return;
    }
  }

  const lookupResult = await streetLookup.lookup(value);
  if (lookupResult.status === "stale") return;

  if (lookupResult.status === "empty") {
    resultsEl.textContent = "";
    statusEl.textContent = `Type ${SHARD_PREFIX_LEN}+ letters to search`;
    clearMarkers();
    return;
  }

  if (lookupResult.status === "short") {
    resultsEl.textContent = `Type at least ${SHARD_PREFIX_LEN} letters`;
    statusEl.textContent = "Waiting for prefix";
    clearMarkers();
    return;
  }

  if (lookupResult.status === "missing") {
    resultsEl.textContent = "No matches";
    statusEl.textContent = `No shard for ${lookupResult.shardKey}`;
    clearMarkers();
    return;
  }

  statusEl.textContent = `Loaded shard ${lookupResult.shardKey} (${lookupResult.locationsCount} locations)`;
  const entries = renderResults(lookupResult.results);
  addMarkers(entries.slice(0, 500));
}

renderFilters();
updateAllowedKinds();

initMap();
map.setView([0, 0], 2);
statusEl.textContent = `Type ${SHARD_PREFIX_LEN}+ letters to load a shard`;

searchInput.addEventListener("input", () => {
  updateSearch().catch((err) => {
    statusEl.textContent = "Failed to search";
    console.error(err);
  });
});
