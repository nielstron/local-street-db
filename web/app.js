const statusEl = document.getElementById("status");
const resultsEl = document.getElementById("results");
const searchInput = document.getElementById("search");

const MAX_RESULTS = 80;
const SHARD_PREFIX_LEN = 3;
const SHARD_BASE = "street_trie";
const isLocalhost =
  window.location.hostname === "localhost" ||
  window.location.hostname === "127.0.0.1";
const SHARD_DIR = isLocalhost ? "/build/shards" : "./build/shards";
const SHARD_SUFFIX = ".packed";
let trie = null;
let locations = [];
let placeNodes = [];
let placeCities = [];
let map = null;
let markersLayer = null;
let currentShardKey = null;
let shardLoadId = 0;
const shardCache = new Map();
const shardLoads = new Map();

function normalizeSearchValue(value) {
  return value
    .normalize("NFKD")
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]/gu, "");
}

function shardKeyForPrefix(value) {
  const normalized = normalizeSearchValue(value.trim());
  if (!normalized) return null;
  const prefix = normalized.slice(0, SHARD_PREFIX_LEN);
  let key = "";
  for (const ch of prefix) {
    key += /[a-z0-9]/.test(ch) ? ch : "_";
  }
  while (key.length < SHARD_PREFIX_LEN) {
    key += "_";
  }
  return key;
}

function decodeVarint(view, offset) {
  let shift = 0;
  let value = 0;
  let byte = 0;
  do {
    byte = view.getUint8(offset++);
    value |= (byte & 0x7f) << shift;
    shift += 7;
  } while (byte & 0x80);
  return [value, offset];
}

function decodePackedTrie(buffer) {
  const view = new DataView(buffer);
  const magic = String.fromCharCode(
    view.getUint8(0),
    view.getUint8(1),
    view.getUint8(2),
    view.getUint8(3)
  );
  if (magic !== "STRI") {
    throw new Error("Invalid trie file");
  }
  const version = view.getUint8(4);
  if (version !== 3 && version !== 4) {
    throw new Error(`Unsupported version ${version}`);
  }
  const scale = view.getInt32(5, true);
  let offset = 9;

  let placeNodeCount;
  [placeNodeCount, offset] = decodeVarint(view, offset);
  const nodeList = new Array(placeNodeCount);
  for (let i = 0; i < placeNodeCount; i++) {
    let nodeLen;
    [nodeLen, offset] = decodeVarint(view, offset);
    const bytes = new Uint8Array(buffer, offset, nodeLen);
    const node = new TextDecoder("utf-8").decode(bytes);
    offset += nodeLen;
    nodeList[i] = node;
  }

  let cityCount;
  [cityCount, offset] = decodeVarint(view, offset);
  const cityList = new Array(cityCount);
  for (let i = 0; i < cityCount; i++) {
    let cityLen;
    [cityLen, offset] = decodeVarint(view, offset);
    const bytes = new Uint8Array(buffer, offset, cityLen);
    const city = new TextDecoder("utf-8").decode(bytes);
    offset += cityLen;
    cityList[i] = city;
  }

  let count;
  [count, offset] = decodeVarint(view, offset);
  const locs = new Array(count);
  for (let i = 0; i < count; i++) {
    const lon = view.getInt32(offset, true);
    const lat = view.getInt32(offset + 4, true);
    offset += 8;
    let nodeIdx;
    [nodeIdx, offset] = decodeVarint(view, offset);
    let cityIdx;
    [cityIdx, offset] = decodeVarint(view, offset);
    locs[i] = [lon / scale, lat / scale, nodeIdx, cityIdx];
  }

  let labelTable = null;
  if (version === 4) {
    let labelCount;
    [labelCount, offset] = decodeVarint(view, offset);
    labelTable = new Array(labelCount);
    for (let i = 0; i < labelCount; i++) {
      let labelLen;
      [labelLen, offset] = decodeVarint(view, offset);
      const bytes = new Uint8Array(buffer, offset, labelLen);
      const label = new TextDecoder("utf-8").decode(bytes);
      offset += labelLen;
      labelTable[i] = label;
    }
  }

  let nodeCount;
  [nodeCount, offset] = decodeVarint(view, offset);
  const nodes = new Array(nodeCount);
  for (let i = 0; i < nodeCount; i++) {
    let edgeCount;
    [edgeCount, offset] = decodeVarint(view, offset);
    const edges = [];
    for (let e = 0; e < edgeCount; e++) {
      let label;
      if (version === 4) {
        let labelIdx;
        [labelIdx, offset] = decodeVarint(view, offset);
        label = labelTable[labelIdx] || "";
      } else {
        let labelLen;
        [labelLen, offset] = decodeVarint(view, offset);
        const bytes = new Uint8Array(buffer, offset, labelLen);
        label = new TextDecoder("utf-8").decode(bytes);
        offset += labelLen;
      }
      let childIdx;
      [childIdx, offset] = decodeVarint(view, offset);
      edges.push({ label, child: childIdx });
    }

    let valuesCount;
    [valuesCount, offset] = decodeVarint(view, offset);
    const values = [];
    for (let v = 0; v < valuesCount; v++) {
      let value;
      [value, offset] = decodeVarint(view, offset);
      values.push(value);
    }
    nodes[i] = { edges, values };
  }

  return { locations: locs, placeNodes: nodeList, placeCities: cityList, nodes };
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

function addMarkers(indices) {
  clearMarkers();
  const latLngs = [];
  for (const idx of indices) {
    const loc = locations[idx];
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
  if (latLngs.length > 0) {
    map.fitBounds(latLngs, { padding: [40, 40] });
  }
}

function collectMatches(prefix) {
  const normalizedPrefix = normalizeSearchValue(prefix);
  const results = [];
  if (!normalizedPrefix) return results;
  let bestNode = 0;
  let bestBuilt = "";
  let bestConsumed = 0;

  function collectFrom(nodeIndex, built) {
    const node = trie.nodes[nodeIndex];
    if (!node) return;

    if (node.values.length) {
      for (const idx of node.values) {
        results.push({ display: built, index: idx });
        if (results.length >= MAX_RESULTS) return;
      }
    }
    for (const edge of node.edges) {
      if (results.length >= MAX_RESULTS) return;
      collectFrom(edge.child, built + edge.label);
    }
  }

  function dfs(nodeIndex, built, remaining, consumed) {
    const node = trie.nodes[nodeIndex];
    if (!node) return;

    if (consumed > bestConsumed) {
      bestConsumed = consumed;
      bestNode = nodeIndex;
      bestBuilt = built;
    }

    if (remaining.length === 0) {
      collectFrom(nodeIndex, built);
      return;
    }

    for (const edge of node.edges) {
      const edgeNormalized = normalizeSearchValue(edge.label);
      if (!edgeNormalized) {
        dfs(edge.child, built + edge.label, remaining, consumed);
        continue;
      }
      if (remaining.startsWith(edgeNormalized)) {
        dfs(
          edge.child,
          built + edge.label,
          remaining.slice(edgeNormalized.length),
          consumed + edgeNormalized.length
        );
      } else if (edgeNormalized.startsWith(remaining)) {
        dfs(
          edge.child,
          built + edge.label,
          "",
          consumed + remaining.length
        );
      }
    }
  }

  dfs(0, "", normalizedPrefix, 0);
  if (!results.length && bestConsumed > 0) {
    collectFrom(bestNode, bestBuilt);
  }
  return results;
}

async function loadShard(shardKey) {
  if (shardCache.has(shardKey)) {
    return shardCache.get(shardKey);
  }
  if (shardLoads.has(shardKey)) {
    return shardLoads.get(shardKey);
  }
  const url = `${SHARD_DIR}/${SHARD_BASE}.shard_${shardKey}${SHARD_SUFFIX}`;
  const loadPromise = fetch(url).then(async (response) => {
    if (!response.ok) {
      throw new Error(`Missing shard ${shardKey}`);
    }
    const buffer = await response.arrayBuffer();
    const decoded = decodePackedTrie(buffer);
    shardCache.set(shardKey, decoded);
    shardLoads.delete(shardKey);
    return decoded;
  });
  shardLoads.set(shardKey, loadPromise);
  return loadPromise;
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
    const loc = locations[entry.index];
    let cityText = "Unknown city";
    if (loc) {
      const nodeName = placeNodes[loc[2]] || "";
      const cityName = placeCities[loc[3]] || "";
      if (nodeName && cityName) {
        cityText = `${nodeName}, ${cityName}`;
      } else {
        cityText = nodeName || cityName || "Unknown city";
      }
    }

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
    div.addEventListener("click", () => addMarkers([entry.index]));
    resultsEl.appendChild(div);
  }

  return limitedEntries.map((entry) => entry.index);
}

async function updateSearch() {
  const rawValue = searchInput.value;
  const value = rawValue.trim();
  const normalizedValue = normalizeSearchValue(value);
  if (!normalizedValue) {
    resultsEl.textContent = "";
    statusEl.textContent = "Type 3+ letters to search";
    clearMarkers();
    return;
  }
  if (normalizedValue.length < SHARD_PREFIX_LEN) {
    resultsEl.textContent = "Type at least 3 letters";
    statusEl.textContent = "Waiting for prefix";
    clearMarkers();
    return;
  }
  const shardKey = shardKeyForPrefix(value);
  if (!shardKey) {
    resultsEl.textContent = "Type at least 3 letters";
    statusEl.textContent = "Waiting for prefix";
    clearMarkers();
    return;
  }

  if (shardKey !== currentShardKey) {
    const loadId = ++shardLoadId;
    statusEl.textContent = `Loading shard ${shardKey}…`;
    try {
      const decoded = await loadShard(shardKey);
      if (loadId !== shardLoadId) return;
      trie = decoded;
      locations = decoded.locations;
      placeNodes = decoded.placeNodes;
      placeCities = decoded.placeCities;
      currentShardKey = shardKey;
      statusEl.textContent = `Loaded shard ${shardKey} (${locations.length} locations)`;
    } catch (err) {
      if (loadId !== shardLoadId) return;
      trie = null;
      locations = [];
      placeNodes = [];
      placeCities = [];
      currentShardKey = shardKey;
      resultsEl.textContent = "No matches";
      statusEl.textContent = `No shard for ${shardKey}`;
      clearMarkers();
      return;
    }
  }

  if (!trie) return;
  const matches = collectMatches(value);
  const indices = renderResults(matches);
  addMarkers(indices.slice(0, 500));
}

initMap();
map.setView([0, 0], 2);
statusEl.textContent = "Type 3+ letters to load a shard";

searchInput.addEventListener("input", () => {
  updateSearch().catch((err) => {
    statusEl.textContent = "Failed to search";
    console.error(err);
  });
});
