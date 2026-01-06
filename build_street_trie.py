#!/usr/bin/env python3
import argparse
import json
from pathlib import Path
from typing import Dict, Iterable, List, Tuple

import osmium

NAME_KEYS = {
    "name",
    "alt_name",
    "old_name",
    "official_name",
    "loc_name",
    "short_name",
}
NAME_PREFIXES = ("name:", "alt_name:", "old_name:", "official_name:", "loc_name:", "short_name:")


def split_names(value: str) -> List[str]:
    parts = [p.strip() for p in value.split(";")]
    return [p for p in parts if p]


def collect_names(tags: osmium.osm.TagList) -> List[str]:
    names: List[str] = []
    seen = set()

    def add(value: str) -> None:
        for item in split_names(value):
            if item not in seen:
                seen.add(item)
                names.append(item)

    for tag in tags:
        key = tag.k
        if key in NAME_KEYS or key.startswith(NAME_PREFIXES):
            if tag.v:
                add(tag.v)

    return names


def polygon_centroid(coords: List[Tuple[float, float]]) -> Tuple[float, float]:
    if coords[0] != coords[-1]:
        coords = coords + [coords[0]]
    if len(coords) < 4:
        raise ValueError("polygon must have at least 3 points")

    area = 0.0
    cx = 0.0
    cy = 0.0
    for i in range(len(coords) - 1):
        x0, y0 = coords[i]
        x1, y1 = coords[i + 1]
        cross = x0 * y1 - x1 * y0
        area += cross
        cx += (x0 + x1) * cross
        cy += (y0 + y1) * cross

    area *= 0.5
    if abs(area) < 1e-12:
        avg_x = sum(p[0] for p in coords[:-1]) / (len(coords) - 1)
        avg_y = sum(p[1] for p in coords[:-1]) / (len(coords) - 1)
        return avg_x, avg_y

    cx /= 6.0 * area
    cy /= 6.0 * area
    return cx, cy


def find_default_pbf(folder: Path) -> Path:
    pbfs = sorted(folder.glob("*.pbf"))
    if not pbfs:
        raise FileNotFoundError("no .pbf files found in current directory")
    if len(pbfs) > 1:
        raise FileExistsError("multiple .pbf files found; pass --input explicitly")
    return pbfs[0]


def insert_trie(trie: Dict, key: str, index: int) -> None:
    node = trie
    for ch in key:
        node = node.setdefault(ch, {})
    node.setdefault("$", []).append(index)


def compress_trie(trie: Dict) -> Dict:
    terminal = trie.get("$")
    compressed: Dict = {}

    for key, child in trie.items():
        if key == "$":
            continue

        compressed_child = compress_trie(child)
        merged_key = key

        while True:
            child_keys = [k for k in compressed_child.keys() if k != "$"]
            if "$" not in compressed_child and len(child_keys) == 1:
                only_key = child_keys[0]
                merged_key += only_key
                compressed_child = compressed_child[only_key]
                continue
            break

        compressed[merged_key] = compressed_child

    if terminal is not None:
        compressed["$"] = terminal

    return compressed


class StreetTrieBuilder(osmium.SimpleHandler):
    def __init__(self, locations: List[Tuple[float, float]], trie: Dict):
        super().__init__()
        self.locations = locations
        self.trie = trie

    def way(self, w):
        if "highway" not in w.tags:
            return

        names = collect_names(w.tags)
        if not names:
            return

        if not w.is_closed():
            return

        coords = []
        for node in w.nodes:
            if not node.location.valid():
                return
            coords.append((node.location.lon, node.location.lat))

        if len(coords) < 4:
            return

        try:
            center_lon, center_lat = polygon_centroid(coords)
        except ValueError:
            return

        index = len(self.locations)
        self.locations.append((center_lon, center_lat))

        for name in names:
            insert_trie(self.trie, name, index)


def build_trie(input_path: Path) -> Tuple[List[Tuple[float, float]], Dict]:
    locations: List[Tuple[float, float]] = []
    trie: Dict = {}
    handler = StreetTrieBuilder(locations, trie)
    handler.apply_file(str(input_path), locations=True)
    return locations, trie


def parse_args(argv: Iterable[str] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build a street-name trie keyed by name to indices in a location array."
    )
    parser.add_argument(
        "--input",
        type=Path,
        help="Path to a .pbf file. Defaults to the only .pbf in the current folder.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("street_trie.json"),
        help="Output JSON path.",
    )
    return parser.parse_args(argv)


def main() -> None:
    args = parse_args()
    input_path = args.input if args.input else find_default_pbf(Path.cwd())
    locations, trie = build_trie(input_path)
    trie = compress_trie(trie)
    payload = {
        "locations": locations,
        "trie": trie,
    }
    args.output.write_text(json.dumps(payload, ensure_ascii=False), encoding="utf-8")


if __name__ == "__main__":
    main()
