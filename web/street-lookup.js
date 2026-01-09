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

function decodeInt24(view, offset) {
  let value =
    view.getUint8(offset) |
    (view.getUint8(offset + 1) << 8) |
    (view.getUint8(offset + 2) << 16);
  if (value & 0x800000) {
    value |= 0xff000000;
  }
  return [value, offset + 3];
}

function decodePrefixTable(view, offset, buffer) {
  let count;
  [count, offset] = decodeVarint(view, offset);
  const list = new Array(count);
  let prev = new Uint8Array(0);
  for (let i = 0; i < count; i++) {
    let prefixLen;
    [prefixLen, offset] = decodeVarint(view, offset);
    let suffixLen;
    [suffixLen, offset] = decodeVarint(view, offset);
    const suffixBytes = new Uint8Array(buffer, offset, suffixLen);
    offset += suffixLen;
    const bytes = new Uint8Array(prefixLen + suffixLen);
    if (prefixLen > 0) {
      bytes.set(prev.subarray(0, prefixLen), 0);
    }
    bytes.set(suffixBytes, prefixLen);
    list[i] = new TextDecoder("utf-8").decode(bytes);
    prev = bytes;
  }
  return [list, offset];
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
  if (version !== 3 && version !== 4 && version !== 5 && version !== 6 && version !== 7 && version !== 9 && version !== 10 && version !== 11 && version !== 12) {
    throw new Error(`Unsupported version ${version}`);
  }
  let scale;
  let offset;
  if (version === 5 || version === 6 || version === 7 || version === 9 || version === 10 || version === 11 || version === 12) {
    scale =
      view.getUint8(5) |
      (view.getUint8(6) << 8) |
      (view.getUint8(7) << 16);
    offset = 8;
  } else {
    scale = view.getInt32(5, true);
    offset = 9;
  }

  let nodeList = null;
  let cityList = null;
  if (version >= 9) {
    [nodeList, offset] = decodePrefixTable(view, offset, buffer);
    [cityList, offset] = decodePrefixTable(view, offset, buffer);
  } else {
    let placeNodeCount;
    [placeNodeCount, offset] = decodeVarint(view, offset);
    nodeList = new Array(placeNodeCount);
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
    cityList = new Array(cityCount);
    for (let i = 0; i < cityCount; i++) {
      let cityLen;
      [cityLen, offset] = decodeVarint(view, offset);
      const bytes = new Uint8Array(buffer, offset, cityLen);
      const city = new TextDecoder("utf-8").decode(bytes);
      offset += cityLen;
      cityList[i] = city;
    }
  }

  let locs = [];
  let locationsCount = 0;
  if (version <= 5) {
    let count;
    [count, offset] = decodeVarint(view, offset);
    locs = new Array(count);
    locationsCount = count;
    for (let i = 0; i < count; i++) {
      let lon;
      let lat;
      if (version === 5) {
        [lon, offset] = decodeInt24(view, offset);
        [lat, offset] = decodeInt24(view, offset);
      } else {
        lon = view.getInt32(offset, true);
        lat = view.getInt32(offset + 4, true);
        offset += 8;
      }
      let nodeIdx;
      [nodeIdx, offset] = decodeVarint(view, offset);
      let cityIdx;
      [cityIdx, offset] = decodeVarint(view, offset);
      locs[i] = [lon / scale, lat / scale, nodeIdx, cityIdx];
    }
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
  if (version >= 7) {
    let loudsBitCount;
    [loudsBitCount, offset] = decodeVarint(view, offset);
    const loudsByteCount = Math.ceil(loudsBitCount / 8);
    const loudsBytes = new Uint8Array(buffer, offset, loudsByteCount);
    offset += loudsByteCount;

    let edgeCount;
    [edgeCount, offset] = decodeVarint(view, offset);
    const edgeLabels = new Array(edgeCount);
    for (let i = 0; i < edgeCount; i++) {
      let labelLen;
      [labelLen, offset] = decodeVarint(view, offset);
      const bytes = new Uint8Array(buffer, offset, labelLen);
      edgeLabels[i] = new TextDecoder("utf-8").decode(bytes);
      offset += labelLen;
    }

    const valuesPerNode = new Array(nodeCount);
    let pendingKindRef = null;
    for (let i = 0; i < nodeCount; i++) {
      let valuesCount;
      [valuesCount, offset] = decodeVarint(view, offset);
      const values = [];
      for (let v = 0; v < valuesCount; v++) {
        let lon;
        let lat;
        [lon, offset] = decodeInt24(view, offset);
        [lat, offset] = decodeInt24(view, offset);
        let nodeIdx;
        [nodeIdx, offset] = decodeVarint(view, offset);
        let cityIdx;
        [cityIdx, offset] = decodeVarint(view, offset);
        if (version >= 12) {
          const byte = view.getUint8(offset);
          offset += 1;
          const kind = byte & 0x0f;
          const pop = (byte >> 4) & 0x0f;
          values.push([lon / scale, lat / scale, nodeIdx, cityIdx, kind, pop]);
        } else if (version >= 11) {
          const entryIndex = values.length;
          values.push([lon / scale, lat / scale, nodeIdx, cityIdx, 0]);
          if (!pendingKindRef) {
            pendingKindRef = { list: values, index: entryIndex };
          } else {
            const byte = view.getUint8(offset);
            offset += 1;
            pendingKindRef.list[pendingKindRef.index][4] = byte & 0x0f;
            values[entryIndex][4] = (byte >> 4) & 0x0f;
            pendingKindRef = null;
          }
        } else {
          let kind = 0;
          if (version >= 10) {
            kind = view.getUint8(offset);
            offset += 1;
          }
          values.push([lon / scale, lat / scale, nodeIdx, cityIdx, kind]);
        }
        locationsCount += 1;
      }
      valuesPerNode[i] = values;
    }
    if (version >= 11 && pendingKindRef) {
      const byte = view.getUint8(offset);
      offset += 1;
      pendingKindRef.list[pendingKindRef.index][4] = byte & 0x0f;
      pendingKindRef = null;
    }

    let currentNode = 0;
    let edgeIdx = 0;
    for (let bitIndex = 0; bitIndex < loudsBitCount; bitIndex++) {
      const byte = loudsBytes[bitIndex >> 3];
      const bit = (byte >> (bitIndex & 7)) & 1;
      if (bit === 1) {
        const childIdx = edgeIdx + 1;
        const label = edgeLabels[edgeIdx] || "";
        if (!nodes[currentNode]) {
          nodes[currentNode] = { edges: [], values: valuesPerNode[currentNode] || [] };
        }
        nodes[currentNode].edges.push({ label, child: childIdx });
        edgeIdx += 1;
      } else {
        if (!nodes[currentNode]) {
          nodes[currentNode] = { edges: [], values: valuesPerNode[currentNode] || [] };
        }
        currentNode += 1;
        if (currentNode >= nodeCount) {
          break;
        }
      }
    }
    for (let i = 0; i < nodeCount; i++) {
      if (!nodes[i]) {
        nodes[i] = { edges: [], values: valuesPerNode[i] || [] };
      }
    }
  } else {
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
        if (version === 6) {
          let lon;
          let lat;
          [lon, offset] = decodeInt24(view, offset);
          [lat, offset] = decodeInt24(view, offset);
          let nodeIdx;
          [nodeIdx, offset] = decodeVarint(view, offset);
          let cityIdx;
          [cityIdx, offset] = decodeVarint(view, offset);
          values.push([lon / scale, lat / scale, nodeIdx, cityIdx]);
          locationsCount += 1;
        } else {
          let value;
          [value, offset] = decodeVarint(view, offset);
          values.push(value);
        }
      }
      nodes[i] = { edges, values };
    }
  }

  return {
    locations: locs,
    locationsCount,
    placeNodes: nodeList,
    placeCities: cityList,
    nodes,
  };
}

function ensureArrayBuffer(view) {
  if (view.byteOffset === 0 && view.byteLength === view.buffer.byteLength) {
    return view.buffer;
  }
  return view.buffer.slice(view.byteOffset, view.byteOffset + view.byteLength);
}

function maybeGunzip(buffer) {
  const bytes = new Uint8Array(buffer);
  if (bytes.length >= 2 && bytes[0] === 0x1f && bytes[1] === 0x8b) {
    if (!window.pako || typeof window.pako.ungzip !== "function") {
      throw new Error("pako is required to decode gzipped shards");
    }
    return ensureArrayBuffer(window.pako.ungzip(bytes));
  }
  return buffer;
}

class StreetLookup {
  constructor(options) {
    const opts = options || {};
    this.maxResults = opts.maxResults ?? 80;
    this.shardPrefixLen = opts.shardPrefixLen ?? 3;
    this.shardBase = opts.shardBase ?? "street_trie";
    this.shardSuffix = opts.shardSuffix ?? ".packed.gz";
    this.shardRoot =
      opts.shardRoot ??
      "https://nielstron.github.io/local-street-db/build/shards";

    this.trie = null;
    this.locations = [];
    this.locationsCount = 0;
    this.placeNodes = [];
    this.placeCities = [];
    this.currentShardKey = null;
    this.lookupId = 0;
    this.shardCache = new Map();
    this.shardLoads = new Map();
    this.allowedKinds = this.normalizeKinds(opts.allowedKinds);
    this.debug =
      typeof window !== "undefined" &&
      window.location &&
      (window.location.hostname === "localhost" ||
        window.location.hostname === "127.0.0.1");
  }

  normalizeKinds(value) {
    if (value == null) {
      return null;
    }
    const set = new Set();
    for (const item of value) {
      const num = Number(item);
      if (!Number.isNaN(num)) {
        set.add(num);
      }
    }
    return set;
  }

  setAllowedKinds(value) {
    this.allowedKinds = this.normalizeKinds(value);
  }

  normalize(value) {
    return value
      .normalize("NFKD")
      .replace(/\p{M}/gu, "")
      .toLowerCase()
      .replace(/[^\p{L}\p{N}]/gu, "");
  }

  getShardKey(value) {
    const normalized = this.normalize(value.trim());
    if (!normalized) return null;
    const prefix = normalized.slice(0, this.shardPrefixLen);
    let key = "";
    for (const ch of prefix) {
      key += /[a-z0-9]/.test(ch) ? ch : "_";
    }
    while (key.length < this.shardPrefixLen) {
      key += "_";
    }
    return key;
  }

  async loadShard(shardKey) {
    if (this.shardCache.has(shardKey)) {
      return this.shardCache.get(shardKey);
    }
    if (this.shardLoads.has(shardKey)) {
      return this.shardLoads.get(shardKey);
    }
    const url = `${this.shardRoot}/${this.shardBase}.shard_${shardKey}${this.shardSuffix}`;
    if (this.debug) {
      console.log("[street-lookup] load shard", { shardKey, url });
    }
    const loadPromise = fetch(url).then(async (response) => {
      if (!response.ok) {
        if (this.debug) {
          console.warn("[street-lookup] shard fetch failed", {
            shardKey,
            status: response.status,
          });
        }
        throw new Error(`Missing shard ${shardKey}`);
      }
      const buffer = await response.arrayBuffer();
      const decoded = decodePackedTrie(maybeGunzip(buffer));
      if (this.debug) {
        console.log("[street-lookup] shard decoded", {
          shardKey,
          locations: decoded.locations?.length ?? 0,
          nodes: decoded.trie?.nodes?.length ?? 0,
        });
      }
      this.shardCache.set(shardKey, decoded);
      this.shardLoads.delete(shardKey);
      return decoded;
    });
    this.shardLoads.set(shardKey, loadPromise);
    return loadPromise;
  }

  collectMatches(prefix, cityFilter) {
    const normalizedPrefix = this.normalize(prefix);
    const results = [];
    if (!normalizedPrefix) return results;
    const normalizedCity = cityFilter ? this.normalize(cityFilter) : "";
    let bestNode = 0;
    let bestBuilt = "";
    let bestConsumed = 0;

    const isAllowedKind = (kind) => {
      if (!this.allowedKinds) return true;
      return this.allowedKinds.has(kind);
    };

    const getLocationForValue = (value) => {
      if (Array.isArray(value)) {
        return value;
      }
      const loc = this.locations[value];
      if (!loc) return null;
      return loc;
    };

    const getKindForValue = (value) => {
      const loc = getLocationForValue(value);
      if (!loc) return 0;
      return loc[4] ?? 0;
    };

    const matchesCityFilter = (value) => {
      if (!normalizedCity) return true;
      const loc = getLocationForValue(value);
      if (!loc) return false;
      const nodeName = this.placeNodes[loc[2]] || "";
      const cityName = this.placeCities[loc[3]] || "";
      const nodeNorm = this.normalize(nodeName);
      const cityNorm = this.normalize(cityName);
      return nodeNorm.includes(normalizedCity) || cityNorm.includes(normalizedCity);
    };

    const collectFrom = (nodeIndex, built) => {
      const node = this.trie.nodes[nodeIndex];
      if (!node) return;

      if (node.values.length) {
        for (const value of node.values) {
          const kind = getKindForValue(value);
          if (!isAllowedKind(kind)) {
            continue;
          }
          if (!matchesCityFilter(value)) {
            continue;
          }
          if (Array.isArray(value)) {
            results.push({ display: built, location: value });
          } else {
            results.push({ display: built, index: value });
          }
          if (results.length >= this.maxResults) return;
        }
      }
      for (const edge of node.edges) {
        if (results.length >= this.maxResults) return;
        collectFrom(edge.child, built + edge.label);
      }
    };

    const dfs = (nodeIndex, built, remaining, consumed) => {
      const node = this.trie.nodes[nodeIndex];
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
        const edgeNormalized = this.normalize(edge.label);
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
          dfs(edge.child, built + edge.label, "", consumed + remaining.length);
        }
      }
    };

    dfs(0, "", normalizedPrefix, 0);
    if (!results.length && bestConsumed > 0) {
      collectFrom(bestNode, bestBuilt);
    }
    return results;
  }

  async lookup(query) {
    let streetQuery = query;
    let cityQuery = "";
    const commaIndex = query.indexOf(",");
    if (commaIndex >= 0) {
      streetQuery = query.slice(0, commaIndex).trim();
      cityQuery = query.slice(commaIndex + 1).trim();
    }
    const normalized = this.normalize(streetQuery.trim());
    if (this.debug) {
      console.log("[street-lookup] lookup", {
        query,
        streetQuery,
        cityQuery,
        normalized,
      });
    }
    if (!normalized) {
      return {
        status: "empty",
        minLength: this.shardPrefixLen,
        results: [],
      };
    }
    if (normalized.length < this.shardPrefixLen) {
      if (this.debug) {
        console.log("[street-lookup] query too short", {
          normalizedLength: normalized.length,
          minLength: this.shardPrefixLen,
        });
      }
      return {
        status: "short",
        minLength: this.shardPrefixLen,
        results: [],
      };
    }

    const shardKey = this.getShardKey(streetQuery);
    if (!shardKey) {
      if (this.debug) {
        console.log("[street-lookup] no shard key", { streetQuery });
      }
      return {
        status: "short",
        minLength: this.shardPrefixLen,
        results: [],
      };
    }

    const lookupId = ++this.lookupId;
    let loaded = false;
    if (shardKey !== this.currentShardKey || !this.trie) {
      try {
        const decoded = await this.loadShard(shardKey);
        if (lookupId !== this.lookupId) {
          return { status: "stale", shardKey, results: [] };
        }
        this.trie = decoded;
        this.locations = decoded.locations;
        this.locationsCount = decoded.locationsCount ?? decoded.locations.length;
        this.placeNodes = decoded.placeNodes;
        this.placeCities = decoded.placeCities;
        this.currentShardKey = shardKey;
        loaded = true;
      } catch (err) {
        if (lookupId !== this.lookupId) {
          return { status: "stale", shardKey, results: [] };
        }
        if (this.debug) {
          console.warn("[street-lookup] shard load error", {
            shardKey,
            error: String(err),
          });
        }
        this.trie = null;
        this.locations = [];
        this.locationsCount = 0;
        this.placeNodes = [];
        this.placeCities = [];
        this.currentShardKey = shardKey;
        return { status: "missing", shardKey, results: [] };
      }
    }

    if (!this.trie) {
      if (this.debug) {
        console.warn("[street-lookup] missing trie", { shardKey });
      }
      return { status: "missing", shardKey, results: [] };
    }

    const matches = this.collectMatches(streetQuery, cityQuery);
    if (this.debug) {
      console.log("[street-lookup] matches", {
        shardKey,
        total: matches.length,
      });
    }
    const results = matches.map((entry) => {
      const location = entry.location ?? this.locations[entry.index] ?? null;
      const loc = location ?? [];
      const normalizedDisplay = this.normalize(entry.display ?? "");
      return {
        ...entry,
        location,
        placeLabel: this.buildPlaceLabel(entry.location, entry.index),
        kindByte: loc[4] ?? 0,
        populationK: loc[5] ?? 0,
        exactMatch: normalizedDisplay === normalized,
      };
    });

    const kindGroup = (kindByte) => {
      if (kindByte === 9) return 0; // city
      if (kindByte === 0) return 1; // street
      return 2;
    };
    results.sort((a, b) => {
      const exactDiff = (a.exactMatch ? 0 : 1) - (b.exactMatch ? 0 : 1);
      if (exactDiff !== 0) return exactDiff;
      const groupDiff = kindGroup(a.kindByte ?? 0) - kindGroup(b.kindByte ?? 0);
      if (groupDiff !== 0) return groupDiff;
      const popDiff = (b.populationK ?? 0) - (a.populationK ?? 0);
      if (popDiff !== 0) return popDiff;
      const kindDiff = (a.kindByte ?? 0) - (b.kindByte ?? 0);
      if (kindDiff !== 0) return kindDiff;
      const aLabel = String(a.display ?? "");
      const bLabel = String(b.display ?? "");
      const lenDiff = aLabel.length - bLabel.length;
      if (lenDiff !== 0) return lenDiff;
      return aLabel.localeCompare(bLabel);
    });
    if (this.debug) {
      const exactCount = results.reduce(
        (count, entry) => count + (entry.exactMatch ? 1 : 0),
        0
      );
      console.log("[street-lookup] sorted matches", {
        exact: exactCount,
        other: results.length - exactCount,
      });
    }

    return {
      status: "ready",
      shardKey,
      loaded,
      locationsCount: this.locationsCount,
      results,
    };
  }

  buildPlaceLabel(location, index) {
    const loc = location ?? this.locations[index];
    if (!loc) return "Unknown city";
    const nodeName = this.placeNodes[loc[2]] || "";
    const cityName = this.placeCities[loc[3]] || "";
    if (nodeName && cityName) {
      return `${nodeName}, ${cityName}`;
    }
    return nodeName || cityName || "Unknown city";
  }
}

window.StreetLookup = StreetLookup;
