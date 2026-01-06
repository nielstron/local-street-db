from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

import csv

from build_street_trie import build_trie, compress_trie, insert_trie, write_payload, pack_trie


def lookup_trie(trie, key):
    node = trie
    remaining = key
    while remaining:
        match = None
        for edge, child in node.items():
            if edge == "$":
                continue
            if remaining.startswith(edge):
                match = (edge, child)
                break
        if match is None:
            return []
        edge, node = match
        remaining = remaining[len(edge) :]
    return node.get("$", [])


def test_compress_trie_merges_linear_paths():
    trie = {}
    insert_trie(trie, "cat", 1)
    insert_trie(trie, "car", 2)
    insert_trie(trie, "dog", 3)
    insert_trie(trie, "do", 4)

    compressed = compress_trie(trie)

    assert "ca" in compressed
    assert "do" in compressed

    ca_node = compressed["ca"]
    assert "t" in ca_node
    assert "r" in ca_node

    do_node = compressed["do"]
    assert do_node.get("$") == [4]
    assert "g" in do_node


def test_lookup_trie_with_compressed_edges():
    trie = {}
    insert_trie(trie, "main", 10)
    insert_trie(trie, "market", 11)
    insert_trie(trie, "maple", 12)

    compressed = compress_trie(trie)

    assert lookup_trie(compressed, "main") == [10]
    assert lookup_trie(compressed, "market") == [11]
    assert lookup_trie(compressed, "maple") == [12]
    assert lookup_trie(compressed, "missing") == []


def test_write_payload_msgpack(tmp_path: Path) -> None:
    payload = {"locations": [(1.0, 2.0)], "trie": {"a": {"$": [0]}}}
    out_path = tmp_path / "trie.msgpack"

    write_payload(payload, out_path, "msgpack")

    import msgpack

    data = msgpack.unpackb(out_path.read_bytes(), raw=False)
    assert data == {"locations": [[1.0, 2.0]], "trie": {"a": {"$": [0]}}}


def decode_varint(data: bytes, offset: int) -> tuple[int, int]:
    shift = 0
    value = 0
    while True:
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7F) << shift
        if byte & 0x80 == 0:
            break
        shift += 7
    return value, offset


def test_pack_trie_binary_format() -> None:
    payload = {"locations": [(1.0, 2.0)], "trie": {"a": {"$": [0]}}}
    data = pack_trie(payload["locations"], payload["trie"], scale=10_000_000)

    assert data[:4] == b"STRI"
    assert data[4] == 1
    scale = int.from_bytes(data[5:9], "little", signed=True)
    assert scale == 10_000_000
    offset = 9

    loc_count, offset = decode_varint(data, offset)
    assert loc_count == 1
    lon = int.from_bytes(data[offset : offset + 4], "little", signed=True)
    lat = int.from_bytes(data[offset + 4 : offset + 8], "little", signed=True)
    offset += 8
    assert lon == 10_000_000
    assert lat == 20_000_000

    node_count, offset = decode_varint(data, offset)
    assert node_count == 2

    edge_count, offset = decode_varint(data, offset)
    assert edge_count == 1
    label_len, offset = decode_varint(data, offset)
    label = data[offset : offset + label_len].decode("utf-8")
    offset += label_len
    assert label == "a"
    child_idx, offset = decode_varint(data, offset)
    assert child_idx == 1

    values_count, offset = decode_varint(data, offset)
    assert values_count == 0

    edge_count, offset = decode_varint(data, offset)
    assert edge_count == 0
    values_count, offset = decode_varint(data, offset)
    assert values_count == 1
    value, offset = decode_varint(data, offset)
    assert value == 0


def test_build_trie_from_csv(tmp_path: Path) -> None:
    csv_path = tmp_path / "streets.csv"
    with csv_path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.writer(f)
        writer.writerow(["streetname", "center_lon", "center_lat"])
        writer.writerow(["Main St", "1.0", "2.0"])
        writer.writerow(["Main St", "3.0", "4.0"])
        writer.writerow(["Second St", "5.0", "6.0"])

    locations, trie = build_trie(csv_path)
    assert locations == [(1.0, 2.0), (3.0, 4.0), (5.0, 6.0)]
    compressed = compress_trie(trie)
    assert lookup_trie(compressed, "Main St") == [0, 1]
    assert lookup_trie(compressed, "Second St") == [2]
