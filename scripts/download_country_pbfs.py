#!/usr/bin/env python3
"""Download country-level OSM PBF extracts into pbfs/countries."""

from __future__ import annotations

import argparse
import html.parser
import os
import sys
import urllib.error
import urllib.parse
import urllib.request

from pypdl import Pypdl

DEFAULT_BASE_URL = "https://download.openstreetmap.fr/extracts/"
DEFAULT_OUT_DIR = os.path.join("pbfs", "countries")


class _LinkParser(html.parser.HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.links: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag.lower() != "a":
            return
        for key, value in attrs:
            if key.lower() == "href" and value:
                self.links.append(value)


def _fetch_text(url: str) -> str:
    req = urllib.request.Request(
        url,
        headers={"User-Agent": "streetdb-country-pbf-downloader/1.0"},
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        return resp.read().decode("utf-8", errors="replace")


def _parse_links(html_text: str) -> list[str]:
    parser = _LinkParser()
    parser.feed(html_text)
    return parser.links


def _filter_continent_dirs(base_url: str, links: list[str]) -> list[str]:
    dirs: list[str] = []
    for href in links:
        if href in ("../", "./"):
            continue
        if not href.endswith("/"):
            continue
        if href.startswith("?"):
            continue
        dirs.append(urllib.parse.urljoin(base_url, href))
    return sorted(set(dirs))


def _list_continent_dirs(base_url: str) -> list[str]:
    html_text = _fetch_text(base_url)
    links = _parse_links(html_text)
    return _filter_continent_dirs(base_url, links)


def _filter_country_pbfs(continent_url: str, links: list[str]) -> list[tuple[str, str]]:
    pbfs: list[tuple[str, str]] = []
    for href in links:
        if not href.endswith(".osm.pbf"):
            continue
        if "/" in href:
            continue
        file_url = urllib.parse.urljoin(continent_url, href)
        pbfs.append((file_url, href))
    return sorted(set(pbfs))


def _list_country_pbfs(continent_url: str) -> list[tuple[str, str]]:
    html_text = _fetch_text(continent_url)
    links = _parse_links(html_text)
    return _filter_country_pbfs(continent_url, links)


def _download_files_parallel(
    tasks: list[tuple[str, str]],
    *,
    force: bool,
    max_concurrent: int,
    segments: int,
) -> None:
    download_tasks: list[dict[str, str]] = []
    for url, dest_path in tasks:
        if not force and os.path.exists(dest_path):
            continue
        download_tasks.append({"url": url, "file_path": dest_path})

    if not download_tasks:
        return

    downloader = Pypdl(max_concurrent=max_concurrent)
    downloader.start(
        tasks=download_tasks,
        multisegment=True,
        segments=segments,
        overwrite=force,
        display=True,
        block=True,
    )


def download_country_pbfs(
    base_url: str,
    out_dir: str,
    *,
    force: bool,
    max_concurrent: int,
    segments: int,
) -> list[str]:
    os.makedirs(out_dir, exist_ok=True)
    downloaded: list[str] = []
    for continent_url in _list_continent_dirs(base_url):
        tasks: list[tuple[str, str]] = []
        for file_url, filename in _list_country_pbfs(continent_url):
            dest_path = os.path.join(out_dir, filename)
            tasks.append((file_url, dest_path))
            downloaded.append(dest_path)
        _download_files_parallel(
            tasks,
            force=force,
            max_concurrent=max_concurrent,
            segments=segments,
        )
    return downloaded


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Download country-level OSM PBF extracts into pbfs/countries.",
    )
    parser.add_argument("--base-url", default=DEFAULT_BASE_URL)
    parser.add_argument("--out-dir", default=DEFAULT_OUT_DIR)
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--max-concurrent", type=int, default=4)
    parser.add_argument("--segments", type=int, default=5)
    args = parser.parse_args(argv)

    try:
        download_country_pbfs(
            args.base_url,
            args.out_dir,
            force=args.force,
            max_concurrent=args.max_concurrent,
            segments=args.segments,
        )
    except (urllib.error.URLError, RuntimeError, ValueError) as exc:
        print(f"download failed: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
