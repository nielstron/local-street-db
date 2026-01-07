# local-street-db

#### Self-hosted, offline street + POI geocoding database and JavaScript lookup library for global address search.

[JS library docs](./web) - [Demo page](https://nielstron.github.io/local-street-db/) - [Artifact releases](https://github.com/nielstron/local-street-db/releases)

## What is this for?

`local-street-db` enables fast, offline geocoding of street names and POIs directly in the browser or any JS runtime.
Check out the [demo web page](https://nielstron.github.io/local-street-db/) and the [library documentation](./web) to integrate it into your application.
If you want private, self-hosted address search (no third-party geocoding APIs), this is designed for you.

Example use cases are
- Private address search inside a web app (no external API calls)
- Local-first or air-gapped deployments
- Geotagging photos or datasets by street name
- On-device maps or navigation prototypes

## What does this repo do?

This repo contains the code to generate the artifact [`street_trie.packed.tar.xz`](https://github.com/nielstron/local-street-db/releases) and the corresponding [`street-lookup`](./web) client library used by the demo web app.
The code in this repo builds a compact, searchable street-name index from OpenStreetMap PBF extracts, extracts named streets and named POIs, deduplicates aggressively, and packs the result into a binary trie used by the demo web app.

- `web/` Leaflet demo that loads the artifact for lookup.
- `extract/` Rust CLI that reads `.pbf` or `.osm` and writes a normalized CSV.
- `trie/` Python builder that compresses street names into a packed trie.
- `scripts/` orchestration and download helpers.


## Requirements

- Rust toolchain (for the extractor)
- Python with `uv`
- Optional: `msgpack` if you want msgpack output

## Quickstart

1) Download country PBFs

```
uv run python scripts/download_country_pbfs.py --out-dir pbfs
```

2) Build all assets (extract CSVs, merge, build packed trie shards, create tarball)

```
uv run python scripts/build_all.py
```

Outputs are written under `build/`, including:

- `build/streets_merged.csv`
- `build/street_trie.packed` (output base name for sharding)
- `build/shards/` (contains `*.packed.gz`)
- `build/street_trie.packed.tar.xz`

You can download the produced output from the releases page for a quickstart.

3) Run the demo

Serve the repo root with any static server and open `web/index.html`. The app loads gzipped shard files from `build/shards/`.
If you have a prebuilt bundle (`street_trie.packed.tar.xz`), extract it at the repo root so it creates `build/shards/`.

## Standalone usage

Extract a single PBF to CSV:

```
cargo run --release --manifest-path extract/Cargo.toml -- --input path/to/file.pbf --output street_polygons.csv
```

Build a trie from a CSV:

```
uv run python trie/build_street_trie.py --input street_polygons.csv --output street_trie.packed --format packed
```


## How it works (high level)

This project expects OpenStreetMap PBF extracts (e.g. from https://download.openstreetmap.fr/extracts/).

1) Download PBF extracts (OpenStreetMap).
2) Extract named streets and POIs to CSV.
3) Deduplicate and normalize entries.
4) Build a packed trie and shard it for fast, partial-prefix lookup.
5) Serve shards to the JS client for on-demand search.

## Contributing

Contributions are welcome! Head to the [Discussions](https://github.com/nielstron/local-street-db/discussions) to learn about what I consider interesting potential addtions and feel free to chime in with your use cases.

## Tests

```
uv run python -m unittest tests/test_build_all.py
uv run python -m unittest tests/test_download_country_pbfs.py
uv run python -m pytest trie/tests/test_build_street_trie.py
cargo test --manifest-path extract/Cargo.toml
```
