# local-street-db

Builds a compact, searchable street-name index from OpenStreetMap PBF extracts. The pipeline extracts named streets and a small set of named POIs, merges nearby segments, and packs the result into a binary trie used by the demo web app.

## What’s in this repo

- `extract/` Rust CLI that reads `.pbf` or `.osm` and writes a normalized CSV.
- `trie/` Python builder that compresses street names into a packed trie.
- `scripts/` orchestration and download helpers.
- `web/` Leaflet demo that loads sharded tries from `build/shards/`.

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
- `build/shards/`
- `build/street_trie.packed.tar.xz`

3) Run the demo

Serve the repo root with any static server and open `web/index.html`. The app loads shard files from `build/shards/`.
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

## Tests

```
uv run python -m unittest tests/test_download_country_pbfs.py
uv run python -m pytest trie/tests/test_build_street_trie.py
cargo test --manifest-path extract/Cargo.toml
```

## Data sources

This project expects OpenStreetMap PBF extracts (e.g. from https://download.openstreetmap.fr/extracts/).
