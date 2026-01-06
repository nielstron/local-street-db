import gzip
import os
import sys
import unittest
from pathlib import Path

SCRIPT_DIR = os.path.join(os.path.dirname(__file__), "..", "scripts")
sys.path.insert(0, os.path.abspath(SCRIPT_DIR))

import build_all


class BuildAllTests(unittest.TestCase):
    def test_gzip_shards_removes_originals(self):
        tmp_dir = Path(__file__).resolve().parent / "_tmp_shards"
        shards_dir = tmp_dir / "shards"
        shards_dir.mkdir(parents=True, exist_ok=True)
        try:
            shard_a = shards_dir / "street_trie.shard_aaa.packed"
            shard_b = shards_dir / "street_trie.shard_bbb.packed"
            shard_a.write_bytes(b"alpha")
            shard_b.write_bytes(b"beta")

            gz_paths = build_all.gzip_shards(shards_dir)

            self.assertEqual(len(gz_paths), 2)
            self.assertFalse(shard_a.exists())
            self.assertFalse(shard_b.exists())

            for gz_path, expected in [
                (shard_a.with_suffix(".packed.gz"), b"alpha"),
                (shard_b.with_suffix(".packed.gz"), b"beta"),
            ]:
                self.assertTrue(gz_path.exists())
                with gzip.open(gz_path, "rb") as fh:
                    self.assertEqual(fh.read(), expected)
        finally:
            if tmp_dir.exists():
                for path in sorted(tmp_dir.rglob("*"), reverse=True):
                    if path.is_file():
                        path.unlink()
                    else:
                        path.rmdir()


if __name__ == "__main__":
    unittest.main()
