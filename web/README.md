# Street Lookup Library

`street-lookup.js` is a small client-side library that loads packed street trie shards on demand and performs lookups with a single async call.

## Include

Load `pako` first (for gzipped shards), then `street-lookup.js`, then your app code:

```html
<script src="https://unpkg.com/pako@2.1.0/dist/pako.min.js"></script>
<script src="street-lookup.js"></script>
<script src="app.js"></script>
```

## Usage

```js
const lookup = new StreetLookup({
  maxResults: 80,
  shardPrefixLen: 3,
  shardBase: "street_trie",
  shardSuffix: ".packed.gz",
  shardRoot: "https://nielstron.github.io/local-street-db/build/shards",
});

const result = await lookup.lookup("Main");

if (result.status === "ready") {
  for (const entry of result.results) {
    console.log(entry.display, entry.placeLabel, entry.location);
  }
}
```

## Lookup API

`lookup(query)` is the only call you need. It selects the shard, fetches it, caches it, and returns matches.

Returned object:

- `status`
  - `"ready"`: results are available.
  - `"empty"`: no query provided.
  - `"short"`: query length is below `shardPrefixLen`.
  - `"missing"`: shard not found for the prefix.
  - `"stale"`: a newer lookup started while this one was loading.
- `results` (array, when `status === "ready"`)
  - `display`: street name text.
  - `index`: location index inside the shard.
  - `location`: `[lon, lat, placeNodeIndex, placeCityIndex]` or `null`.
  - `placeLabel`: human-friendly `"Node, City"` label or `"Unknown city"`.
- `shardKey`: shard prefix key used.
- `locationsCount`: number of locations in the loaded shard.
- `loaded`: `true` if this call fetched a new shard.

## Shard Root

`shardRoot` controls where shards are fetched from. It defaults to:

```
https://nielstron.github.io/local-street-db/build/shards
```

If you host shards elsewhere, pass a different `shardRoot` in the constructor.
