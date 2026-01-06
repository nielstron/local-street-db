#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
pbfs_dir="$root_dir/pbfs"
build_dir="$root_dir/build"
csv_dir="$build_dir/csvs"
merged_csv="$build_dir/streets_merged.csv"
packed_trie="$build_dir/street_trie.packed"
tarball="$build_dir/streetdb-build.tar.gz"

mkdir -p "$csv_dir"

shopt -s nullglob
pbfs=("$pbfs_dir"/*.pbf)
shopt -u nullglob

if [ ${#pbfs[@]} -eq 0 ]; then
  echo "No .pbf files found in $pbfs_dir" >&2
  exit 1
fi

echo "Extracting CSVs from ${#pbfs[@]} PBF files..."
for pbf in "${pbfs[@]}"; do
  base="$(basename "$pbf")"
  base="${base%.osm.pbf}"
  base="${base%.pbf}"
  out_csv="$csv_dir/${base}.csv"
  echo "  -> $out_csv"
  cargo run --release --manifest-path "$root_dir/extract/Cargo.toml" -- \
    --input "$pbf" \
    --output "$out_csv"
done

echo "Merging CSVs into $merged_csv"
first=1
: > "$merged_csv"
for csv in "$csv_dir"/*.csv; do
  if [ $first -eq 1 ]; then
    cat "$csv" >> "$merged_csv"
    first=0
  else
    tail -n +2 "$csv" >> "$merged_csv"
  fi
done

echo "Building packed trie at $packed_trie"
uv run python "$root_dir/trie/build_street_trie.py" \
  --input "$merged_csv" \
  --output "$packed_trie" \
  --format packed

echo "Creating tarball at $tarball"
tar -C "$root_dir" -I 'gzip -9' -cf "$tarball" build

echo "Done."
