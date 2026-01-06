#!/usr/bin/env python3
import argparse
import json
from pathlib import Path
from typing import Dict, Iterable, List, Tuple

import csv


def find_default_csv(folder: Path) -> Path:
    csvs = sorted(folder.glob("*.csv"))
    if not csvs:
        raise FileNotFoundError("no .csv files found in current directory")
    if len(csvs) > 1:
        raise FileExistsError("multiple .csv files found; pass --input explicitly")
    return csvs[0]


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


def build_trie(input_path: Path) -> Tuple[List[Tuple[float, float, int]], List[str], Dict]:
    locations: List[Tuple[float, float, int]] = []
    location_index: Dict[Tuple[float, float], int] = {}
    cities: List[str] = []
    city_index: Dict[str, int] = {}
    trie: Dict = {}
    with input_path.open(newline="", encoding="utf-8") as f:
        reader = csv.DictReader(f)
        required = {"streetname", "center_lon", "center_lat", "city_resolved"}
        if not required.issubset(reader.fieldnames or []):
            missing = ", ".join(sorted(required - set(reader.fieldnames or [])))
            raise ValueError(f"missing required CSV columns: {missing}")

        for row in reader:
            name = (row.get("streetname") or "").strip()
            if not name:
                continue
            try:
                lon = float(row["center_lon"])
                lat = float(row["center_lat"])
            except (TypeError, ValueError):
                continue

            city = (row.get("city_resolved") or "").strip()
            if city not in city_index:
                city_index[city] = len(cities)
                cities.append(city)
            city_idx = city_index[city]

            loc_key = (lon, lat)
            index = location_index.get(loc_key)
            if index is None:
                index = len(locations)
                location_index[loc_key] = index
                locations.append((lon, lat, city_idx))
            insert_trie(trie, name, index)
    return locations, cities, trie


def encode_varint(value: int) -> bytes:
    out = bytearray()
    v = value
    while True:
        byte = v & 0x7F
        v >>= 7
        if v:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            break
    return bytes(out)


def build_nodes(trie: Dict) -> List[Dict]:
    nodes: List[Dict] = []

    def visit(node: Dict) -> int:
        idx = len(nodes)
        nodes.append({"edges": [], "values": []})

        values = node.get("$", [])
        nodes[idx]["values"] = list(values)

        edges = []
        for edge_label, child in node.items():
            if edge_label == "$":
                continue
            edges.append((edge_label, child))

        edges.sort(key=lambda item: item[0])
        for edge_label, child in edges:
            child_idx = visit(child)
            nodes[idx]["edges"].append((edge_label, child_idx))

        return idx

    visit(trie)
    return nodes


def pack_trie(
    locations: List[Tuple[float, float, int]],
    cities: List[str],
    trie: Dict,
    scale: int = 10_000_000,
) -> bytes:
    out = bytearray()
    out.extend(b"STRI")
    out.append(2)
    out.extend(scale.to_bytes(4, "little", signed=True))

    out.extend(encode_varint(len(cities)))
    for city in cities:
        city_bytes = city.encode("utf-8")
        out.extend(encode_varint(len(city_bytes)))
        out.extend(city_bytes)

    out.extend(encode_varint(len(locations)))
    for lon, lat, city_idx in locations:
        lon_i = int(round(lon * scale))
        lat_i = int(round(lat * scale))
        out.extend(lon_i.to_bytes(4, "little", signed=True))
        out.extend(lat_i.to_bytes(4, "little", signed=True))
        out.extend(encode_varint(city_idx))

    nodes = build_nodes(trie)
    out.extend(encode_varint(len(nodes)))
    for node in nodes:
        edges = node["edges"]
        out.extend(encode_varint(len(edges)))
        for label, child_idx in edges:
            label_bytes = label.encode("utf-8")
            out.extend(encode_varint(len(label_bytes)))
            out.extend(label_bytes)
            out.extend(encode_varint(child_idx))

        values = node["values"]
        out.extend(encode_varint(len(values)))
        for value in values:
            out.extend(encode_varint(value))

    return bytes(out)


def write_payload(payload: Dict, output_path: Path, output_format: str) -> None:
    if output_format == "json":
        output_path.write_text(json.dumps(payload, ensure_ascii=False), encoding="utf-8")
        return

    if output_format == "msgpack":
        try:
            import msgpack
        except ImportError as exc:
            raise SystemExit("msgpack is required for --format msgpack") from exc
        output_path.write_bytes(msgpack.packb(payload, use_bin_type=True))
        return

    if output_format == "packed":
        packed = pack_trie(payload["locations"], payload["cities"], payload["trie"])
        output_path.write_bytes(packed)
        return

    raise SystemExit(f"unknown format: {output_format}")


def parse_args(argv: Iterable[str] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build a street-name trie from a CSV keyed by name to indices in a location array."
    )
    parser.add_argument(
        "--input",
        type=Path,
        help="Path to a CSV file. Defaults to the only .csv in the current folder.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("street_trie.packed"),
        help="Output file path.",
    )
    parser.add_argument(
        "--format",
        choices=["json", "msgpack", "packed"],
        default="packed",
        help="Output format. Defaults to packed for compact binary output.",
    )
    return parser.parse_args(argv)


def main() -> None:
    args = parse_args()
    input_path = args.input if args.input else find_default_csv(Path.cwd())
    locations, cities, trie = build_trie(input_path)
    trie = compress_trie(trie)
    payload = {
        "locations": locations,
        "cities": cities,
        "trie": trie,
    }
    write_payload(payload, args.output, args.format)


if __name__ == "__main__":
    main()
