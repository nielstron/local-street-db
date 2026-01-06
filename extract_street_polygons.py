#!/usr/bin/env python3
import argparse
import csv
from pathlib import Path
from typing import Iterable, List, Tuple

import osmium
import osmium.area
from shapely import wkb
from shapely.geometry import Point

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


class AdminBoundaryCollector(osmium.SimpleHandler):
    def __init__(self, admin_levels: List[str]):
        super().__init__()
        self.admin_levels = set(admin_levels)
        self.boundaries: List[Tuple[str, object]] = []
        self.wkb_factory = osmium.geom.WKBFactory()

    def area(self, a):
        if a.tags.get("boundary") != "administrative":
            return
        admin_level = a.tags.get("admin_level")
        if admin_level not in self.admin_levels:
            return
        name = a.tags.get("name")
        if not name:
            return

        try:
            geom_wkb = self.wkb_factory.create_multipolygon(a)
        except RuntimeError:
            return

        try:
            if isinstance(geom_wkb, (bytes, bytearray, memoryview)):
                geom = wkb.loads(geom_wkb)
            else:
                geom = wkb.loads(geom_wkb, hex=True)
        except Exception:
            return

        if not geom.is_empty:
            self.boundaries.append((name, geom))


def collect_admin_boundaries(input_path: Path, admin_levels: List[str]) -> List[Tuple[str, object]]:
    collector = AdminBoundaryCollector(admin_levels)
    manager = osmium.area.AreaManager()
    buffer = osmium.area.AreaManagerBufferHandler(manager)
    buffer.apply_file(str(input_path), locations=True, idx="flex_mem")
    second = osmium.area.AreaManagerSecondPassHandler(manager, collector)
    second.apply_file(str(input_path), locations=True, idx="flex_mem")
    return collector.boundaries


class StreetPolygonHandler(osmium.SimpleHandler):
    def __init__(self, writer: csv.writer, boundaries: List[Tuple[str, object]]):
        super().__init__()
        self.writer = writer
        self.boundaries = boundaries

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

        city_addr = w.tags.get("addr:city")
        city_place = w.tags.get("addr:place")
        city_boundary = None

        if self.boundaries:
            point = Point(center_lon, center_lat)
            for boundary_name, boundary_geom in self.boundaries:
                if boundary_geom.covers(point):
                    city_boundary = boundary_name
                    break

        city = city_addr or city_boundary or city_place

        for name in names:
            self.writer.writerow(
                [
                    name,
                    f"{center_lon:.7f}",
                    f"{center_lat:.7f}",
                    city or "",
                    city_addr or "",
                    city_boundary or "",
                    city_place or "",
                ]
            )


def find_default_pbf(folder: Path) -> Path:
    pbfs = sorted(folder.glob("*.pbf"))
    if not pbfs:
        raise FileNotFoundError("no .pbf files found in current directory")
    if len(pbfs) > 1:
        raise FileExistsError("multiple .pbf files found; pass --input explicitly")
    return pbfs[0]


def extract_to_csv(input_path: Path, output_path: Path, admin_levels: List[str]) -> None:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with output_path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.writer(f)
        writer.writerow(
            [
                "streetname",
                "center_lon",
                "center_lat",
                "city",
                "city_addr",
                "city_boundary",
                "city_place",
            ]
        )
        boundaries = collect_admin_boundaries(input_path, admin_levels)
        handler = StreetPolygonHandler(writer, boundaries)
        handler.apply_file(str(input_path), locations=True)


def parse_args(argv: Iterable[str] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Extract named street polygons from an .osm.pbf file."
    )
    parser.add_argument(
        "--input",
        type=Path,
        help="Path to a .pbf file. Defaults to the only .pbf in the current folder.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("street_polygons.csv"),
        help="Output CSV path.",
    )
    parser.add_argument(
        "--admin-levels",
        default="8",
        help="Comma-separated admin_level values to treat as city boundaries.",
    )
    return parser.parse_args(argv)


def main() -> None:
    args = parse_args()
    input_path = args.input if args.input else find_default_pbf(Path.cwd())
    admin_levels = [item.strip() for item in args.admin_levels.split(",") if item.strip()]
    extract_to_csv(input_path, args.output, admin_levels)


if __name__ == "__main__":
    main()
