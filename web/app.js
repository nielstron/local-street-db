const statusEl = document.getElementById("status");
const resultsEl = document.getElementById("results");
const searchInput = document.getElementById("search");

const MAX_RESULTS = 80;
let trie = null;
let locations = [];
let placeNodes = [];
let placeCities = [];
let map = null;
let markersLayer = null;

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
  const results = [];

  function dfs(nodeIndex, built, remaining) {
    const node = trie.nodes[nodeIndex];
    if (!node) return;

    if (remaining.length === 0) {
      if (node.values.length) {
        for (const idx of node.values) {
          results.push({ display: built, index: idx });
          if (results.length >= MAX_RESULTS) return;
        }
      }
      for (const edge of node.edges) {
        if (results.length >= MAX_RESULTS) return;
        dfs(edge.child, built + edge.label, remaining);
      }
      return;
    }

    for (const edge of node.edges) {
      const edgeLower = edge.label.toLowerCase();
      if (remaining.startsWith(edgeLower)) {
        dfs(edge.child, built + edge.label, remaining.slice(edgeLower.length));
      } else if (edgeLower.startsWith(remaining)) {
        dfs(edge.child, built + edge.label, "");
      }
    }
  }

  dfs(0, "", prefix);
  return results;
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

function updateSearch() {
  if (!trie) return;
  const value = searchInput.value.trim();
  if (!value) {
    resultsEl.textContent = "";
    clearMarkers();
    return;
  }
  const matches = collectMatches(value.toLowerCase());
  const indices = renderResults(matches);
  addMarkers(indices.slice(0, 500));
}

async function loadTrie() {
  statusEl.textContent = "Loading trie…";
  const response = await fetch("./street_trie.packed");
  const buffer = await response.arrayBuffer();
  const decoded = decodePackedTrie(buffer);
  trie = decoded;
  locations = decoded.locations;
  placeNodes = decoded.placeNodes;
  placeCities = decoded.placeCities;
  statusEl.textContent = `Loaded ${locations.length} locations`;

  if (locations.length) {
    map.setView([locations[0][1], locations[0][0]], 12);
  } else {
    map.setView([0, 0], 2);
  }
}

initMap();
loadTrie().catch((err) => {
  statusEl.textContent = "Failed to load trie";
  console.error(err);
});

searchInput.addEventListener("input", updateSearch);
