const statusEl = document.getElementById("status");
const resultsEl = document.getElementById("results");
const searchInput = document.getElementById("search");

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
  for (const entry of entries) {
    const loc = entry.location;
    if (!loc) continue;
    const [lon, lat] = loc;
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
    map.fitBounds(latLngs, { padding: [40, 40] });
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

    const nameEl = document.createElement("span");
    nameEl.className = "result-name";
    nameEl.textContent = entry.display;

    const cityEl = document.createElement("span");
    cityEl.className = "result-city";
    cityEl.textContent = cityText;

    mainEl.append(nameEl, cityEl);
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

initMap();
map.setView([0, 0], 2);
statusEl.textContent = `Type ${SHARD_PREFIX_LEN}+ letters to load a shard`;

searchInput.addEventListener("input", () => {
  updateSearch().catch((err) => {
    statusEl.textContent = "Failed to search";
    console.error(err);
  });
});
