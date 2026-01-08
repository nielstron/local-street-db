#!/usr/bin/env python3
"""Build all streetdb assets from local PBFs."""
from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import gzip
import tarfile
from concurrent.futures import ThreadPoolExecutor, as_completed
from concurrent.futures.process import ProcessPoolExecutor
from pathlib import Path


def run(cmd: list[str], *, cwd: Path | None = None) -> None:
    result = subprocess.run(cmd, cwd=cwd, check=False)
    if result.returncode != 0:
        raise subprocess.CalledProcessError(result.returncode, cmd)


def extract_one(extract_bin: Path, pbf: Path, out_csv: Path) -> None:
    run([
        str(extract_bin),
        "--input",
        str(pbf),
        "--output",
        str(out_csv),
    ])
    print(f"  -> {out_csv}", flush=True)


def merge_csvs(csv_dir: Path, merged_csv: Path) -> None:
    merged_csv.parent.mkdir(parents=True, exist_ok=True)
    csvs = sorted(csv_dir.glob("*.csv"))
    if not csvs:
        raise RuntimeError(f"No CSVs found in {csv_dir}")

    with merged_csv.open("w", encoding="utf-8", newline="") as out_f:
        first = True
        for csv in csvs:
            with csv.open("r", encoding="utf-8", newline="") as in_f:
                if first:
                    shutil.copyfileobj(in_f, out_f)
                    first = False
                else:
                    _ = in_f.readline()
                    shutil.copyfileobj(in_f, out_f)


def create_tarball(root_dir: Path, tarball: Path) -> None:
    if tarball.exists():
        tarball.unlink()
    with tarfile.open(tarball, mode="w:xz") as tf:
        shards_dir = root_dir / "build" / "shards"
        if not shards_dir.exists():
            raise FileNotFoundError(f"missing shards at {shards_dir}")
        tf.add(shards_dir, arcname="build/shards")


def gzip_shards(shards_dir: Path, *, keep_original: bool = False) -> list[Path]:
    if not shards_dir.exists():
        raise FileNotFoundError(f"missing shards at {shards_dir}")
    gz_paths: list[Path] = []
    for shard_path in sorted(shards_dir.glob("*.packed")):
        gz_path = shard_path.with_suffix(shard_path.suffix + ".gz")
        with shard_path.open("rb") as src, gzip.open(gz_path, "wb", compresslevel=9) as dst:
            shutil.copyfileobj(src, dst)
        gz_paths.append(gz_path)
        if not keep_original:
            shard_path.unlink()
    return gz_paths


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Build all streetdb assets.")
    parser.add_argument(
        "--from-trie",
        action="store_true",
        help="Skip PBF extraction/merge and rebuild trie + tarball only.",
    )
    parser.add_argument(
        "--mini",
        action="store_true",
        help="Run the pipeline only for Switzerland (switzerland-latest.osm.pbf).",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root_dir = Path(__file__).resolve().parents[1]
    pbfs_dir = root_dir / "pbfs"
    build_dir = root_dir / "build"
    csv_dir = build_dir / "csvs"
    merged_csv = build_dir / "streets_merged.csv"
    packed_trie = build_dir / "street_trie.packed"
    tarball = build_dir / "street_trie.packed.tar.xz"
    shards_dir = build_dir / "shards"

    if args.from_trie and args.mini:
        print("--mini ignored when --from-trie is set.", file=sys.stderr)

    if not args.from_trie:
        csv_dir.mkdir(parents=True, exist_ok=True)

        pbfs = sorted(pbfs_dir.glob("*.pbf"))
        if args.mini:
            swiss_default = pbfs_dir / "switzerland-latest.osm.pbf"
            if swiss_default.exists():
                pbfs = [swiss_default]
            else:
                pbfs = [pbf for pbf in pbfs if "switzerland" in pbf.name.lower()]
            if not pbfs:
                print(
                    f"Missing Switzerland PBF at {swiss_default}",
                    file=sys.stderr,
                )
                return 1
        if not pbfs:
            print(f"No .pbf files found in {pbfs_dir}", file=sys.stderr)
            return 1

        print(f"Extracting CSVs from {len(pbfs)} PBF files...")
        jobs = int(os.environ.get("JOBS", "16"))
        print(
            f"Using up to {jobs} parallel jobs for extraction (set JOBS to override)."
        )

        print("Building extractor binary...")
        run([
            "cargo",
            "build",
            "--release",
            "--manifest-path",
            str(root_dir / "extract" / "Cargo.toml"),
        ])
        extract_bin = (
            root_dir / "extract" / "target" / "release" / "extract_street_polygons"
        )

        failed = False
        with ProcessPoolExecutor(max_workers=jobs) as executor:
            futures = [
                executor.submit(
                    extract_one, extract_bin, pbf, csv_dir / f"{pbf.stem}.csv"
                )
                for pbf in pbfs
            ]
            for future in as_completed(futures):
                try:
                    future.result()
                except subprocess.CalledProcessError:
                    failed = True

        if failed:
            print("Extraction failed.", file=sys.stderr)
            return 1

        print(f"Merging CSVs into {merged_csv}")
        merge_csvs(csv_dir, merged_csv)
    else:
        if not merged_csv.exists():
            print(f"Missing merged CSV at {merged_csv}", file=sys.stderr)
            return 1

    print(f"Building packed trie at {packed_trie}")
    if shards_dir.exists():
        shutil.rmtree(shards_dir)
    run([
        "uv",
        "run",
        "python",
        str(root_dir / "trie" / "build_street_trie.py"),
        "--input",
        str(merged_csv),
        "--output",
        str(packed_trie),
        "--format",
        "packed",
    ])

    print(f"Gzipping shard files in {shards_dir}")
    gzip_shards(shards_dir)

    print(f"Creating tarball at {tarball}")
    create_tarball(root_dir, tarball)

    print("Done.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
