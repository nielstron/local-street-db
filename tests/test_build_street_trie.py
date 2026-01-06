from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from build_street_trie import compress_trie, insert_trie


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
