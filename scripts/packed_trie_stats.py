#!/usr/bin/env python3
import argparse
import gzip
import sys
import time
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, Iterable, Tuple

MAGIC = b"STRI"


class ParseError(RuntimeError):
    pass


def decode_varint(data: bytes, offset: int) -> Tuple[int, int, int]:
    value = 0
    shift = 0
    start = offset
    while True:
        if offset >= len(data):
            raise ParseError("varint extends past end of buffer")
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7F) << shift
        if not (byte & 0x80):
            break
        shift += 7
    return value, offset, offset - start


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


def maybe_gunzip(data: bytes) -> Tuple[bytes, bool]:
    if len(data) >= 2 and data[0] == 0x1F and data[1] == 0x8B:
        return gzip.decompress(data), True
    return data, False


@dataclass
class TrieStats:
    bytes_by_section: Counter = field(default_factory=Counter)
    meta: Dict[str, int] = field(default_factory=dict)
    version: int = 0
    scale: int = 0
    total_bytes: int = 0
    parsed_bytes: int = 0
    estimates: Dict[str, int] = field(default_factory=dict)

    def add(self, key: str, count: int) -> None:
        if count:
            self.bytes_by_section[key] += count


@dataclass
class ParsedTrie:
    stats: TrieStats
    is_gz: bool
    compressed_size: int
    path: Path


SECTION_GROUPS = {
    "header": ["header.magic", "header.version", "header.scale"],
    "place_nodes": [
        "place_nodes.count_varint",
        "place_nodes.name_len_varint",
        "place_nodes.name_bytes",
        "place_nodes.prefix_len_varint",
        "place_nodes.suffix_len_varint",
        "place_nodes.suffix_bytes",
    ],
    "place_cities": [
        "place_cities.count_varint",
        "place_cities.name_len_varint",
        "place_cities.name_bytes",
        "place_cities.prefix_len_varint",
        "place_cities.suffix_len_varint",
        "place_cities.suffix_bytes",
    ],
    "locations": [
        "locations.count_varint",
        "locations.lonlat_bytes",
        "locations.node_idx_varint",
        "locations.city_idx_varint",
        "locations.kind_bytes",
    ],
    "label_table": [
        "label_table.count_varint",
        "label_table.label_len_varint",
        "label_table.label_bytes",
    ],
    "louds": [
        "louds.node_count_varint",
        "louds.bit_count_varint",
        "louds.bitvector_bytes",
    ],
    "trie_edges": [
        "trie.nodes.count_varint",
        "trie.edges.count_varint",
        "trie.edges.label_len_varint",
        "trie.edges.label_bytes",
        "trie.edges.label_idx_varint",
        "trie.edges.child_idx_varint",
        "trie.edges.count_varint_louds",
        "trie.edges.label_len_varint_louds",
        "trie.edges.label_bytes_louds",
    ],
    "trie_values": [
        "trie.values.count_varint",
        "trie.values.value_varint",
    ],
}


def parse_packed_trie(data: bytes, collect_edges: bool = False) -> TrieStats:
    stats = TrieStats()
    offset = 0

    if len(data) < 5:
        raise ParseError("file too small")

    magic = data[:4]
    stats.add("header.magic", 4)
    offset += 4
    if magic != MAGIC:
        raise ParseError("invalid magic")

    version = data[offset]
    stats.add("header.version", 1)
    offset += 1
    if version not in (3, 4, 5, 6, 7, 8, 9, 10, 11):
        raise ParseError(f"unsupported version {version}")

    if version in (5, 6, 7, 8, 9, 10, 11):
        if offset + 3 > len(data):
            raise ParseError("unexpected EOF reading scale")
        scale = data[offset] | (data[offset + 1] << 8) | (data[offset + 2] << 16)
        stats.add("header.scale", 3)
        offset += 3
    else:
        if offset + 4 > len(data):
            raise ParseError("unexpected EOF reading scale")
        scale = int.from_bytes(data[offset : offset + 4], "little", signed=True)
        stats.add("header.scale", 4)
        offset += 4

    stats.version = version
    stats.scale = scale

    place_node_count, offset, size = decode_varint(data, offset)
    stats.add("place_nodes.count_varint", size)
    stats.meta["place_nodes"] = place_node_count
    if version >= 9:
        for _ in range(place_node_count):
            prefix_len, offset, size = decode_varint(data, offset)
            stats.add("place_nodes.prefix_len_varint", size)
            suffix_len, offset, size = decode_varint(data, offset)
            stats.add("place_nodes.suffix_len_varint", size)
            if offset + suffix_len > len(data):
                raise ParseError("unexpected EOF reading place node suffix")
            stats.add("place_nodes.suffix_bytes", suffix_len)
            offset += suffix_len
    else:
        for _ in range(place_node_count):
            name_len, offset, size = decode_varint(data, offset)
            stats.add("place_nodes.name_len_varint", size)
            if offset + name_len > len(data):
                raise ParseError("unexpected EOF reading place node name")
            stats.add("place_nodes.name_bytes", name_len)
            offset += name_len

    city_count, offset, size = decode_varint(data, offset)
    stats.add("place_cities.count_varint", size)
    stats.meta["place_cities"] = city_count
    if version >= 9:
        for _ in range(city_count):
            prefix_len, offset, size = decode_varint(data, offset)
            stats.add("place_cities.prefix_len_varint", size)
            suffix_len, offset, size = decode_varint(data, offset)
            stats.add("place_cities.suffix_len_varint", size)
            if offset + suffix_len > len(data):
                raise ParseError("unexpected EOF reading city suffix")
            stats.add("place_cities.suffix_bytes", suffix_len)
            offset += suffix_len
    else:
        for _ in range(city_count):
            name_len, offset, size = decode_varint(data, offset)
            stats.add("place_cities.name_len_varint", size)
            if offset + name_len > len(data):
                raise ParseError("unexpected EOF reading city name")
            stats.add("place_cities.name_bytes", name_len)
            offset += name_len

    if version <= 5:
        loc_count, offset, size = decode_varint(data, offset)
        stats.add("locations.count_varint", size)
        stats.meta["locations"] = loc_count
        for _ in range(loc_count):
            if version == 5:
                if offset + 6 > len(data):
                    raise ParseError("unexpected EOF reading location lon/lat")
                stats.add("locations.lonlat_bytes", 6)
                offset += 6
            else:
                if offset + 8 > len(data):
                    raise ParseError("unexpected EOF reading location lon/lat")
                stats.add("locations.lonlat_bytes", 8)
                offset += 8
            _, offset, size = decode_varint(data, offset)
            stats.add("locations.node_idx_varint", size)
            _, offset, size = decode_varint(data, offset)
            stats.add("locations.city_idx_varint", size)
    else:
        stats.meta["locations"] = 0

    if version == 4:
        label_count, offset, size = decode_varint(data, offset)
        stats.add("label_table.count_varint", size)
        stats.meta["label_table"] = label_count
        for _ in range(label_count):
            label_len, offset, size = decode_varint(data, offset)
            stats.add("label_table.label_len_varint", size)
            if offset + label_len > len(data):
                raise ParseError("unexpected EOF reading label table")
            stats.add("label_table.label_bytes", label_len)
            offset += label_len

    if version in (7, 9, 10, 11):
        node_count, offset, size = decode_varint(data, offset)
        stats.add("louds.node_count_varint", size)
        stats.meta["trie_nodes"] = node_count

        louds_bit_count, offset, size = decode_varint(data, offset)
        stats.add("louds.bit_count_varint", size)
        louds_byte_count = (louds_bit_count + 7) // 8
        if offset + louds_byte_count > len(data):
            raise ParseError("unexpected EOF reading louds bitvector")
        stats.add("louds.bitvector_bytes", louds_byte_count)
        offset += louds_byte_count

        edge_count, offset, size = decode_varint(data, offset)
        stats.add("trie.edges.count_varint_louds", size)
        edge_total = edge_count
        for _ in range(edge_count):
            label_len, offset, size = decode_varint(data, offset)
            stats.add("trie.edges.label_len_varint_louds", size)
            if offset + label_len > len(data):
                raise ParseError("unexpected EOF reading edge label")
            stats.add("trie.edges.label_bytes_louds", label_len)
            offset += label_len

        value_total = 0
        kind_pending = False
        for _ in range(node_count):
            values_count, offset, size = decode_varint(data, offset)
            stats.add("trie.values.count_varint", size)
            value_total += values_count
            for _ in range(values_count):
                if offset + 6 > len(data):
                    raise ParseError("unexpected EOF reading inline location lon/lat")
                stats.add("locations.lonlat_bytes", 6)
                offset += 6
                _, offset, size = decode_varint(data, offset)
                stats.add("locations.node_idx_varint", size)
                _, offset, size = decode_varint(data, offset)
                stats.add("locations.city_idx_varint", size)
                if version >= 11:
                    if kind_pending:
                        if offset + 1 > len(data):
                            raise ParseError("unexpected EOF reading location kind byte")
                        stats.add("locations.kind_bytes", 1)
                        offset += 1
                        kind_pending = False
                    else:
                        kind_pending = True
                elif version >= 10:
                    if offset + 1 > len(data):
                        raise ParseError("unexpected EOF reading location kind byte")
                    stats.add("locations.kind_bytes", 1)
                    offset += 1
        if version >= 11 and kind_pending:
            if offset + 1 > len(data):
                raise ParseError("unexpected EOF reading location kind byte")
            stats.add("locations.kind_bytes", 1)
            offset += 1
    else:
        node_count, offset, size = decode_varint(data, offset)
        stats.add("trie.nodes.count_varint", size)
        stats.meta["trie_nodes"] = node_count
        edge_total = 0
        value_total = 0
        if collect_edges:
            label_counts: Counter = Counter()
            label_bytes_total = 0
            label_len_varints_total = 0
            label_idx_varints_total = 0
            child_varints_total = 0
            child_delta_varints_total = 0
        for _ in range(node_count):
            edge_count, offset, size = decode_varint(data, offset)
            stats.add("trie.edges.count_varint", size)
            edge_total += edge_count
            prev_child = 0
            for _ in range(edge_count):
                if version == 4:
                    label_idx, offset, size = decode_varint(data, offset)
                    stats.add("trie.edges.label_idx_varint", size)
                    if collect_edges:
                        label_idx_varints_total += size
                else:
                    label_len, offset, size = decode_varint(data, offset)
                    stats.add("trie.edges.label_len_varint", size)
                    if offset + label_len > len(data):
                        raise ParseError("unexpected EOF reading edge label")
                    if collect_edges:
                        label_len_varints_total += size
                        label_bytes = data[offset : offset + label_len]
                        label_counts[label_bytes] += 1
                        label_bytes_total += label_len
                    stats.add("trie.edges.label_bytes", label_len)
                    offset += label_len
                child_idx, offset, size = decode_varint(data, offset)
                stats.add("trie.edges.child_idx_varint", size)
                if collect_edges:
                    child_varints_total += size
                    delta = child_idx - prev_child
                    if delta < 0:
                        delta = child_idx
                    _, _, delta_size = decode_varint(encode_varint(delta), 0)
                    child_delta_varints_total += delta_size
                    prev_child = child_idx

            values_count, offset, size = decode_varint(data, offset)
            stats.add("trie.values.count_varint", size)
            value_total += values_count
            for _ in range(values_count):
                if version == 6:
                    if offset + 6 > len(data):
                        raise ParseError("unexpected EOF reading inline location lon/lat")
                    stats.add("locations.lonlat_bytes", 6)
                    offset += 6
                    _, offset, size = decode_varint(data, offset)
                    stats.add("locations.node_idx_varint", size)
                    _, offset, size = decode_varint(data, offset)
                    stats.add("locations.city_idx_varint", size)
                else:
                    _, offset, size = decode_varint(data, offset)
                    stats.add("trie.values.value_varint", size)

    stats.meta["trie_edges"] = edge_total
    stats.meta["trie_values"] = value_total
    if version in (6, 7, 9, 10):
        stats.meta["locations"] = value_total
    if collect_edges and version != 7 and version != 9 and version != 10:
        stats.estimates["edges.label_bytes_inline"] = label_bytes_total
        stats.estimates["edges.label_len_varints_inline"] = label_len_varints_total
        stats.estimates["edges.child_varints_inline"] = child_varints_total
        if label_counts:
            label_index_sizes = []
            for idx, (label, count) in enumerate(
                sorted(label_counts.items(), key=lambda kv: kv[1], reverse=True)
            ):
                size = len(encode_varint(idx))
                label_index_sizes.append(size * count)
            stats.estimates["edges.label_idx_varints_table"] = sum(label_index_sizes)
        else:
            stats.estimates["edges.label_idx_varints_table"] = label_idx_varints_total
        label_table_bytes = sum(len(label) for label in label_counts.keys())
        label_table_len_varints = sum(len(encode_varint(len(label))) for label in label_counts.keys())
        label_table_count_varint = len(encode_varint(len(label_counts)))
        stats.estimates["label_table.bytes"] = label_table_bytes
        stats.estimates["label_table.len_varints"] = label_table_len_varints
        stats.estimates["label_table.count_varint"] = label_table_count_varint
        stats.estimates["edges.child_varints_delta"] = child_delta_varints_total

    stats.total_bytes = len(data)
    stats.parsed_bytes = offset
    return stats


def format_bytes(count: int) -> str:
    return f"{count:,}"


def summarize_sections(stats: TrieStats) -> Dict[str, int]:
    grouped: Dict[str, int] = {}
    for group, keys in SECTION_GROUPS.items():
        grouped[group] = sum(stats.bytes_by_section.get(key, 0) for key in keys)
    other = stats.total_bytes - sum(grouped.values())
    if other:
        grouped["unclassified"] = other
    return grouped


def render_stats(parsed: ParsedTrie) -> str:
    stats = parsed.stats
    grouped = summarize_sections(stats)
    lines = []
    lines.append(f"{parsed.path}")
    if parsed.is_gz:
        lines.append(
            f"  compressed: {format_bytes(parsed.compressed_size)} bytes, "
            f"uncompressed: {format_bytes(stats.total_bytes)} bytes"
        )
    else:
        lines.append(f"  size: {format_bytes(stats.total_bytes)} bytes")
    lines.append(
        "  meta: "
        f"place_nodes={stats.meta.get('place_nodes', 0)}, "
        f"place_cities={stats.meta.get('place_cities', 0)}, "
        f"locations={stats.meta.get('locations', 0)}, "
        f"trie_nodes={stats.meta.get('trie_nodes', 0)}, "
        f"trie_edges={stats.meta.get('trie_edges', 0)}, "
        f"trie_values={stats.meta.get('trie_values', 0)}"
    )
    if stats.parsed_bytes != stats.total_bytes:
        diff = stats.total_bytes - stats.parsed_bytes
        lines.append(
            f"  warning: parsed {format_bytes(stats.parsed_bytes)} bytes, "
            f"{format_bytes(diff)} bytes remaining"
        )

    total = stats.total_bytes
    lines.append("  breakdown:")
    for section, count in sorted(grouped.items(), key=lambda kv: kv[1], reverse=True):
        pct = (count / total * 100.0) if total else 0.0
        lines.append(
            f"    {section:14} {format_bytes(count):>12} bytes  {pct:6.2f}%"
        )
    if stats.estimates:
        lines.append("  estimates:")
        for key, count in sorted(stats.estimates.items()):
            lines.append(f"    {key:30} {format_bytes(count):>12} bytes")
    return "\n".join(lines)


def iter_shard_files(path: Path) -> Iterable[Path]:
    if path.is_file():
        yield path
        return
    for shard in sorted(path.glob("*.packed")):
        yield shard
    for shard in sorted(path.glob("*.packed.gz")):
        yield shard


def render_progress(label: str, current: int, total: int, start_time: float) -> str:
    if total <= 0:
        return f"{label} {current}"
    width = 28
    ratio = min(max(current / total, 0.0), 1.0)
    filled = int(width * ratio)
    bar = "#" * filled + "-" * (width - filled)
    elapsed = time.time() - start_time
    return f"{label} [{bar}] {current}/{total} {ratio * 100:5.1f}% {elapsed:6.1f}s"


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Compute byte composition of packed street trie shards."
    )
    parser.add_argument(
        "--input",
        type=Path,
        default=Path("build/shards"),
        help="Shard file or directory (default: build/shards).",
    )
    parser.add_argument(
        "--per-shard",
        action="store_true",
        help="Print a breakdown for each shard.",
    )
    parser.add_argument(
        "--edge-estimates",
        action="store_true",
        help="Estimate savings from label table and child index deltas.",
    )
    parser.add_argument(
        "--no-progress",
        action="store_true",
        help="Disable progress output.",
    )
    args = parser.parse_args()

    shard_paths = list(iter_shard_files(args.input))
    if not shard_paths:
        raise SystemExit(f"No shard files found in {args.input}")

    totals = TrieStats()
    total_compressed = 0
    total_uncompressed = 0
    total_edges = 0
    total_values = 0

    rendered = []
    show_progress = (not args.no_progress) and len(shard_paths) > 1
    start_time = time.time()
    for idx, shard_path in enumerate(shard_paths, start=1):
        if show_progress:
            msg = render_progress("Parsing", idx - 1, len(shard_paths), start_time)
            sys.stderr.write(f"\r{msg}")
            sys.stderr.flush()
        raw = shard_path.read_bytes()
        uncompressed, is_gz = maybe_gunzip(raw)
        stats = parse_packed_trie(uncompressed, collect_edges=args.edge_estimates)
        parsed = ParsedTrie(
            stats=stats,
            is_gz=is_gz,
            compressed_size=len(raw),
            path=shard_path,
        )
        if args.per_shard:
            rendered.append(render_stats(parsed))

        totals.version = stats.version
        totals.scale = stats.scale
        totals.total_bytes += stats.total_bytes
        totals.parsed_bytes += stats.parsed_bytes
        totals.bytes_by_section.update(stats.bytes_by_section)
        if stats.estimates:
            for key, count in stats.estimates.items():
                totals.estimates[key] = totals.estimates.get(key, 0) + count
        total_compressed += len(raw)
        total_uncompressed += stats.total_bytes
        total_edges += stats.meta.get("trie_edges", 0)
        total_values += stats.meta.get("trie_values", 0)
        for key in ("place_nodes", "place_cities", "locations", "trie_nodes"):
            totals.meta[key] = totals.meta.get(key, 0) + stats.meta.get(key, 0)

    if show_progress:
        msg = render_progress("Parsing", len(shard_paths), len(shard_paths), start_time)
        sys.stderr.write(f"\r{msg}\n")
        sys.stderr.flush()

    if args.per_shard:
        print("\n\n".join(rendered))
        print("")

    grouped = summarize_sections(totals)
    print("TOTAL")
    if total_compressed and total_compressed != total_uncompressed:
        print(
            f"  compressed: {format_bytes(total_compressed)} bytes, "
            f"uncompressed: {format_bytes(total_uncompressed)} bytes"
        )
    else:
        print(f"  size: {format_bytes(totals.total_bytes)} bytes")
    print(
        "  meta: "
        f"place_nodes={totals.meta.get('place_nodes', 0)}, "
        f"place_cities={totals.meta.get('place_cities', 0)}, "
        f"locations={totals.meta.get('locations', 0)}, "
        f"trie_nodes={totals.meta.get('trie_nodes', 0)}, "
        f"trie_edges={total_edges}, "
        f"trie_values={total_values}"
    )
    if totals.parsed_bytes != totals.total_bytes:
        diff = totals.total_bytes - totals.parsed_bytes
        print(
            f"  warning: parsed {format_bytes(totals.parsed_bytes)} bytes, "
            f"{format_bytes(diff)} bytes remaining"
        )
    print("  breakdown:")
    for section, count in sorted(grouped.items(), key=lambda kv: kv[1], reverse=True):
        pct = (count / totals.total_bytes * 100.0) if totals.total_bytes else 0.0
        print(f"    {section:14} {format_bytes(count):>12} bytes  {pct:6.2f}%")
    if totals.estimates:
        print("  estimates:")
        for key, count in sorted(totals.estimates.items()):
            print(f"    {key:30} {format_bytes(count):>12} bytes")


if __name__ == "__main__":
    main()
