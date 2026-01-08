#!/usr/bin/env python3
import argparse
import json
import unicodedata
from pathlib import Path
from typing import Dict, Iterable, List, Tuple

import csv

TERMINAL_KEY = "\0"
KIND_TO_BYTE = {
    "street": 0,
    "airport": 1,
    "train_station": 2,
    "bus_stop": 3,
    "ferry_terminal": 4,
    "university": 5,
    "museum": 6,
    "civic_building": 7,
    "sight": 8,
    "city": 9,
    "country": 10,
}
DEFAULT_KIND_BYTE = 15
CITY_KIND_BYTE = KIND_TO_BYTE["city"]
COUNTRY_KIND_BYTE = KIND_TO_BYTE["country"]
DEFAULT_COUNTRIES_PATH = Path(__file__).with_name("countries.csv")


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
    node.setdefault(TERMINAL_KEY, []).append(index)


def add_location_entry(
    trie: Dict,
    name: str,
    lon: float,
    lat: float,
    node_idx: int,
    city_idx: int,
    kind_byte: int,
    locations: List[Tuple[float, float, int, int, int]],
    location_index: Dict[Tuple[float, float, int, int, int], int],
) -> None:
    if not name:
        return
    loc_key = (lon, lat, node_idx, city_idx, kind_byte)
    index = location_index.get(loc_key)
    if index is None:
        index = len(locations)
        location_index[loc_key] = index
        locations.append((lon, lat, node_idx, city_idx, kind_byte))
    insert_trie(trie, name, index)


def load_countries(
    countries_path: Path | None,
) -> List[Tuple[str, str, float, float]]:
    if countries_path is None:
        return []
    if not countries_path.exists():
        raise FileNotFoundError(f"countries file not found: {countries_path}")
    countries: List[Tuple[str, str, float, float]] = []
    with countries_path.open(newline="", encoding="utf-8") as f:
        reader = csv.DictReader(f)
        required = {"country", "name", "latitude", "longitude"}
        if not required.issubset(reader.fieldnames or []):
            missing = ", ".join(sorted(required - set(reader.fieldnames or [])))
            raise ValueError(f"missing required countries CSV columns: {missing}")
        for row in reader:
            code = (row.get("country") or "").strip().upper()
            name = (row.get("name") or "").strip()
            if not name:
                continue
            try:
                lat = float(row["latitude"])
                lon = float(row["longitude"])
            except (TypeError, ValueError):
                continue
            countries.append((code, name, lon, lat))
    return countries


def add_countries_to_trie(
    trie: Dict,
    countries: List[Tuple[str, str, float, float]],
    node_names: List[str],
    node_index: Dict[str, int],
    city_names: List[str],
    city_index: Dict[str, int],
    locations: List[Tuple[float, float, int, int, int]],
    location_index: Dict[Tuple[float, float, int, int, int], int],
) -> None:
    for code, name, lon, lat in countries:
        if code and code not in node_index:
            node_index[code] = len(node_names)
            node_names.append(code)
        node_idx = node_index.get(code, 0)
        if name not in city_index:
            city_index[name] = len(city_names)
            city_names.append(name)
        city_idx = city_index[name]
        add_location_entry(
            trie,
            name,
            lon,
            lat,
            node_idx,
            city_idx,
            COUNTRY_KIND_BYTE,
            locations,
            location_index,
        )
        if code:
            add_location_entry(
                trie,
                code,
                lon,
                lat,
                node_idx,
                city_idx,
                COUNTRY_KIND_BYTE,
                locations,
                location_index,
            )


def compress_trie(trie: Dict) -> Dict:
    terminal = trie.get(TERMINAL_KEY)
    compressed: Dict = {}

    for key, child in trie.items():
        if key == TERMINAL_KEY:
            continue

        compressed_child = compress_trie(child)
        merged_key = key

        while True:
            child_keys = [k for k in compressed_child.keys() if k != TERMINAL_KEY]
            if TERMINAL_KEY not in compressed_child and len(child_keys) == 1:
                only_key = child_keys[0]
                merged_key += only_key
                compressed_child = compressed_child[only_key]
                continue
            break

        compressed[merged_key] = compressed_child

    if terminal is not None:
        compressed[TERMINAL_KEY] = terminal

    return compressed


def build_trie(
    input_path: Path,
    countries_path: Path | None = DEFAULT_COUNTRIES_PATH,
) -> Tuple[List[Tuple[float, float, int, int, int]], List[str], List[str], Dict]:
    locations: List[Tuple[float, float, int, int, int]] = []
    location_index: Dict[Tuple[float, float, int, int, int], int] = {}
    node_names: List[str] = [""]
    node_index: Dict[str, int] = {"": 0}
    city_names: List[str] = [""]
    city_index: Dict[str, int] = {"": 0}
    trie: Dict = {}
    with input_path.open(newline="", encoding="utf-8") as f:
        reader = csv.DictReader(f)
        required = {
            "streetname",
            "kind",
            "center_lon",
            "center_lat",
            "city_place_node",
            "city_place_city",
        }
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

            node = (row.get("city_place_node") or "").strip()
            node_new = node not in node_index
            if node_new:
                node_index[node] = len(node_names)
                node_names.append(node)
            node_idx = node_index[node]

            city = (row.get("city_place_city") or "").strip()
            city_new = city not in city_index
            if city_new:
                city_index[city] = len(city_names)
                city_names.append(city)
            city_idx = city_index[city]

            kind_str = (row.get("kind") or "").strip().lower()
            kind_byte = KIND_TO_BYTE.get(kind_str, DEFAULT_KIND_BYTE)
            add_location_entry(
                trie,
                name,
                lon,
                lat,
                node_idx,
                city_idx,
                kind_byte,
                locations,
                location_index,
            )


    countries = load_countries(countries_path)
    add_countries_to_trie(
        trie,
        countries,
        node_names,
        node_index,
        city_names,
        city_index,
        locations,
        location_index,
    )
    return locations, node_names, city_names, trie


def shard_key_for_name(name: str, shard_len: int) -> str | None:
    if shard_len <= 0:
        return None
    normalized_name = normalize_name(name)
    if not normalized_name:
        return None
    prefix = normalized_name[:shard_len]
    normalized = []
    for ch in prefix:
        normalized.append(ch if ch.isascii() and ch.isalnum() else "_")
    while len(normalized) < shard_len:
        normalized.append("_")
    return "".join(normalized)


def normalize_name(name: str) -> str:
    decomposed = unicodedata.normalize("NFKD", name)
    stripped = "".join(
        ch for ch in decomposed if unicodedata.category(ch) != "Mn"
    )
    lowered = stripped.lower()
    return "".join(ch for ch in lowered if ch.isalnum())


def build_trie_shards(
    input_path: Path,
    shard_len: int,
    countries_path: Path | None = DEFAULT_COUNTRIES_PATH,
) -> Dict[str, Dict]:
    shards: Dict[str, Dict] = {}
    with input_path.open(newline="", encoding="utf-8") as f:
        reader = csv.DictReader(f)
        required = {
            "streetname",
            "kind",
            "center_lon",
            "center_lat",
            "city_place_node",
            "city_place_city",
        }
        if not required.issubset(reader.fieldnames or []):
            missing = ", ".join(sorted(required - set(reader.fieldnames or [])))
            raise ValueError(f"missing required CSV columns: {missing}")

        for row in reader:
            name = (row.get("streetname") or "").strip()
            if not name:
                continue
            shard_key = shard_key_for_name(name, shard_len)
            if shard_key is None:
                continue
            try:
                lon = float(row["center_lon"])
                lat = float(row["center_lat"])
            except (TypeError, ValueError):
                continue

            shard = shards.get(shard_key)
            if shard is None:
                shard = {
                    "locations": [],
                    "location_index": {},
                    "node_names": [""],
                    "node_index": {"": 0},
                    "city_names": [""],
                    "city_index": {"": 0},
                    "trie": {},
                }
                shards[shard_key] = shard

            node = (row.get("city_place_node") or "").strip()
            if node not in shard["node_index"]:
                shard["node_index"][node] = len(shard["node_names"])
                shard["node_names"].append(node)
            node_idx = shard["node_index"][node]

            city = (row.get("city_place_city") or "").strip()
            if city not in shard["city_index"]:
                shard["city_index"][city] = len(shard["city_names"])
                shard["city_names"].append(city)
            city_idx = shard["city_index"][city]

            kind_str = (row.get("kind") or "").strip().lower()
            kind_byte = KIND_TO_BYTE.get(kind_str, DEFAULT_KIND_BYTE)
            add_location_entry(
                shard["trie"],
                name,
                lon,
                lat,
                node_idx,
                city_idx,
                kind_byte,
                shard["locations"],
                shard["location_index"],
            )


    countries = load_countries(countries_path)
    for code, name, lon, lat in countries:
        shard_key = shard_key_for_name(name, shard_len)
        if shard_key is None:
            continue
        shard = shards.get(shard_key)
        if shard is None:
            shard = {
                "locations": [],
                "location_index": {},
                "node_names": [""],
                "node_index": {"": 0},
                "city_names": [""],
                "city_index": {"": 0},
                "trie": {},
            }
            shards[shard_key] = shard
        city_index = shard["city_index"]
        city_names = shard["city_names"]
        node_index = shard["node_index"]
        node_names = shard["node_names"]
        if code and code not in node_index:
            node_index[code] = len(node_names)
            node_names.append(code)
        node_idx = node_index.get(code, 0)
        if name not in city_index:
            city_index[name] = len(city_names)
            city_names.append(name)
        city_idx = city_index[name]
        add_location_entry(
            shard["trie"],
            name,
            lon,
            lat,
            node_idx,
            city_idx,
            COUNTRY_KIND_BYTE,
            shard["locations"],
            shard["location_index"],
        )
        if code:
            add_location_entry(
                shard["trie"],
                code,
                lon,
                lat,
                node_idx,
                city_idx,
                COUNTRY_KIND_BYTE,
                shard["locations"],
                shard["location_index"],
            )

    for shard in shards.values():
        shard.pop("location_index", None)
        shard.pop("node_index", None)
        shard.pop("city_index", None)
    return shards


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


def reindex_names(names: List[str]) -> Tuple[List[str], List[int]]:
    indexed = list(enumerate(names))
    indexed.sort(key=lambda item: item[1])
    new_names = [name for _, name in indexed]
    old_to_new = [0] * len(names)
    for new_idx, (old_idx, _) in enumerate(indexed):
        old_to_new[old_idx] = new_idx
    return new_names, old_to_new


def encode_prefix_table(names: List[str]) -> bytes:
    out = bytearray()
    prev_bytes = b""
    out.extend(encode_varint(len(names)))
    for name in names:
        name_bytes = name.encode("utf-8")
        prefix_len = 0
        max_prefix = min(len(prev_bytes), len(name_bytes))
        while prefix_len < max_prefix and prev_bytes[prefix_len] == name_bytes[prefix_len]:
            prefix_len += 1
        suffix = name_bytes[prefix_len:]
        out.extend(encode_varint(prefix_len))
        out.extend(encode_varint(len(suffix)))
        out.extend(suffix)
        prev_bytes = name_bytes
    return bytes(out)


def build_nodes(trie: Dict) -> List[Dict]:
    nodes: List[Dict] = []

    def visit(node: Dict) -> int:
        idx = len(nodes)
        nodes.append({"edges": [], "values": []})

        values = node.get(TERMINAL_KEY, [])
        nodes[idx]["values"] = list(values)

        edges = []
        for edge_label, child in node.items():
            if edge_label == TERMINAL_KEY:
                continue
            edges.append((edge_label, child))

        edges.sort(key=lambda item: item[0])
        for edge_label, child in edges:
            child_idx = visit(child)
            nodes[idx]["edges"].append((edge_label, child_idx))

        return idx

    visit(trie)
    return nodes


def build_louds(trie: Dict) -> Tuple[int, int, int, bytes, List[str], List[List[int]]]:
    queue: List[Dict] = [trie]
    louds_bits: List[int] = []
    edge_labels: List[str] = []
    values_per_node: List[List[int]] = []

    while queue:
        node = queue.pop(0)
        values = node.get(TERMINAL_KEY, [])
        values_per_node.append(list(values))

        edges = []
        for edge_label, child in node.items():
            if edge_label == TERMINAL_KEY:
                continue
            edges.append((edge_label, child))
        edges.sort(key=lambda item: item[0])

        for edge_label, child in edges:
            edge_labels.append(edge_label)
            queue.append(child)
            louds_bits.append(1)
        louds_bits.append(0)

    node_count = len(values_per_node)
    edge_count = len(edge_labels)
    bit_count = len(louds_bits)
    louds_bytes = bytearray((bit_count + 7) // 8)
    for i, bit in enumerate(louds_bits):
        if bit:
            louds_bytes[i >> 3] |= 1 << (i & 7)

    return node_count, edge_count, bit_count, bytes(louds_bytes), edge_labels, values_per_node


def pack_trie(
    locations: List[Tuple[float, float, int, int, int]],
    node_names: List[str],
    city_names: List[str],
    trie: Dict,
    scale: int = 10_000,
) -> bytes:
    out = bytearray()
    out.extend(b"STRI")
    out.append(11)
    if scale < 0 or scale > 0xFFFFFF:
        raise ValueError("scale must fit in 3 bytes")
    out.extend(scale.to_bytes(3, "little", signed=False))

    re_nodes, node_index = reindex_names(node_names)
    re_cities, city_index = reindex_names(city_names)
    out.extend(encode_prefix_table(re_nodes))
    out.extend(encode_prefix_table(re_cities))

    remapped_locations: List[Tuple[float, float, int, int, int]] = []
    for lon, lat, node_idx, city_idx, kind_byte in locations:
        remapped_locations.append(
            (lon, lat, node_index[node_idx], city_index[city_idx], kind_byte)
        )

    node_count, edge_count, bit_count, louds_bytes, edge_labels, values_per_node = build_louds(trie)
    out.extend(encode_varint(node_count))
    out.extend(encode_varint(bit_count))
    out.extend(louds_bytes)
    out.extend(encode_varint(edge_count))
    for label in edge_labels:
        label_bytes = label.encode("utf-8")
        out.extend(encode_varint(len(label_bytes)))
        out.extend(label_bytes)

    pending_kind: int | None = None

    def write_kind_nibble(kind_byte: int) -> None:
        nonlocal pending_kind
        nibble = kind_byte & 0x0F
        if pending_kind is None:
            pending_kind = nibble
        else:
            out.append(pending_kind | (nibble << 4))
            pending_kind = None

    for values in values_per_node:
        out.extend(encode_varint(len(values)))
        for value in values:
            lon, lat, node_idx, city_idx, kind_byte = remapped_locations[value]
            lon_i = int(round(lon * scale))
            lat_i = int(round(lat * scale))
            out.extend(lon_i.to_bytes(3, "little", signed=True))
            out.extend(lat_i.to_bytes(3, "little", signed=True))
            out.extend(encode_varint(node_idx))
            out.extend(encode_varint(city_idx))
            write_kind_nibble(kind_byte)

    if pending_kind is not None:
        out.append(pending_kind)

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
        packed = pack_trie(
            payload["locations"],
            payload["city_place_nodes"],
            payload["city_place_cities"],
            payload["trie"],
        )
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
    parser.add_argument(
        "--countries",
        type=Path,
        default=DEFAULT_COUNTRIES_PATH,
        help="Path to a countries CSV (name, latitude, longitude).",
    )
    parser.add_argument(
        "--shard-prefix-len",
        type=int,
        default=3,
        help="Shard by this many prefix characters (0 to disable). Defaults to 3.",
    )
    return parser.parse_args(argv)


def main() -> None:
    args = parse_args()
    input_path = args.input if args.input else find_default_csv(Path.cwd())
    print(f"Building trie from {input_path}")
    if args.shard_prefix_len > 0:
        print(f"Sharding trie by first {args.shard_prefix_len} characters")
        shards = build_trie_shards(
            input_path, args.shard_prefix_len, args.countries
        )
        print(f"Built {len(shards)} shards")
        output_base = args.output
        if output_base.suffix:
            output_base = output_base.with_suffix("")
        shards_dir = output_base.parent / "shards"
        shards_dir.mkdir(parents=True, exist_ok=True)
        for shard_key in sorted(shards.keys()):
            shard = shards[shard_key]
            print(
                "Loaded shard "
                f"{shard_key}: "
                f"{len(shard['locations'])} locations, "
                f"{len(shard['node_names'])} nodes, "
                f"{len(shard['city_names'])} cities"
            )
            print(f"Compressing shard {shard_key}")
            shard_trie = compress_trie(shard["trie"])
            print(f"Packing shard {shard_key}")
            payload = {
                "locations": shard["locations"],
                "city_place_nodes": shard["node_names"],
                "city_place_cities": shard["city_names"],
                "trie": shard_trie,
            }
            shard_output = shards_dir / f"{output_base.name}.shard_{shard_key}.packed"
            print(f"Writing shard {shard_key} to {shard_output} ({args.format})")
            write_payload(payload, shard_output, args.format)
    else:
        locations, node_names, city_names, trie = build_trie(
            input_path, args.countries
        )
        print(
            "Loaded "
            f"{len(locations)} locations, "
            f"{len(node_names)} nodes, "
            f"{len(city_names)} cities"
        )
        print("Compressing trie edges")
        trie = compress_trie(trie)
        print("Packing trie payload")
        payload = {
            "locations": locations,
            "city_place_nodes": node_names,
            "city_place_cities": city_names,
            "trie": trie,
        }
        print(f"Writing output to {args.output} ({args.format})")
        write_payload(payload, args.output, args.format)
    print("Done")


if __name__ == "__main__":
    main()
