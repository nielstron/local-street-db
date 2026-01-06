import csv
from pathlib import Path
import textwrap
import sys

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from extract_street_polygons import collect_names, extract_to_csv, polygon_centroid


OSM_SAMPLE = textwrap.dedent(
    """\
    <?xml version='1.0' encoding='UTF-8'?>
    <osm version="0.6" generator="test">
      <node id="1" lat="0.0" lon="0.0" />
      <node id="2" lat="0.0" lon="1.0" />
      <node id="3" lat="1.0" lon="1.0" />
      <node id="4" lat="1.0" lon="0.0" />
      <node id="5" lat="2.0" lon="0.0" />
      <node id="6" lat="2.0" lon="1.0" />
      <way id="10">
        <nd ref="1" />
        <nd ref="2" />
        <nd ref="3" />
        <nd ref="4" />
        <nd ref="1" />
        <tag k="highway" v="residential" />
        <tag k="name" v="Main Street" />
        <tag k="alt_name" v="Old Main" />
      </way>
      <way id="11">
        <nd ref="4" />
        <nd ref="5" />
        <nd ref="6" />
        <tag k="highway" v="residential" />
        <tag k="name" v="Open Way" />
      </way>
    </osm>
    """
)

OSM_MULTI_NAME = textwrap.dedent(
    """\
    <?xml version='1.0' encoding='UTF-8'?>
    <osm version="0.6" generator="test">
      <node id="1" lat="0.0" lon="0.0" />
      <node id="2" lat="0.0" lon="2.0" />
      <node id="3" lat="2.0" lon="2.0" />
      <node id="4" lat="2.0" lon="0.0" />
      <way id="20">
        <nd ref="1" />
        <nd ref="2" />
        <nd ref="3" />
        <nd ref="4" />
        <nd ref="1" />
        <tag k="highway" v="primary" />
        <tag k="name" v="First;Second" />
        <tag k="name:de" v="Erste Straße" />
      </way>
    </osm>
    """
)

OSM_WITH_BOUNDARY = textwrap.dedent(
    """\
    <?xml version='1.0' encoding='UTF-8'?>
    <osm version="0.6" generator="test">
      <node id="1" lat="0.0" lon="0.0" />
      <node id="2" lat="0.0" lon="2.0" />
      <node id="3" lat="2.0" lon="2.0" />
      <node id="4" lat="2.0" lon="0.0" />
      <node id="5" lat="0.5" lon="0.5" />
      <node id="6" lat="0.5" lon="1.5" />
      <node id="7" lat="1.5" lon="1.5" />
      <node id="8" lat="1.5" lon="0.5" />
      <way id="100">
        <nd ref="1" />
        <nd ref="2" />
        <nd ref="3" />
        <nd ref="4" />
        <nd ref="1" />
      </way>
      <relation id="200">
        <member type="way" ref="100" role="outer" />
        <tag k="type" v="multipolygon" />
        <tag k="boundary" v="administrative" />
        <tag k="admin_level" v="8" />
        <tag k="name" v="Test City" />
      </relation>
      <way id="201">
        <nd ref="5" />
        <nd ref="6" />
        <nd ref="7" />
        <nd ref="8" />
        <nd ref="5" />
        <tag k="highway" v="residential" />
        <tag k="name" v="Center Road" />
      </way>
      <way id="202">
        <nd ref="5" />
        <nd ref="6" />
        <nd ref="7" />
        <nd ref="8" />
        <nd ref="5" />
        <tag k="highway" v="residential" />
        <tag k="name" v="Tagged Road" />
        <tag k="addr:city" v="Explicit City" />
        <tag k="addr:place" v="Place City" />
      </way>
    </osm>
    """
)


def test_extract_to_csv(tmp_path: Path) -> None:
    osm_path = tmp_path / "sample.osm"
    out_path = tmp_path / "out.csv"
    osm_path.write_text(OSM_SAMPLE, encoding="utf-8")

    extract_to_csv(osm_path, out_path, ["8"])

    with out_path.open(newline="", encoding="utf-8") as f:
        rows = list(csv.reader(f))

    assert rows[0] == [
        "streetname",
        "center_lon",
        "center_lat",
        "city",
        "city_addr",
        "city_boundary",
        "city_place",
    ]
    # Two names for the closed way, one entry per name.
    names = [row[0] for row in rows[1:]]
    assert names == ["Main Street", "Old Main"]


def test_extract_to_csv_splits_multi_names(tmp_path: Path) -> None:
    osm_path = tmp_path / "multi.osm"
    out_path = tmp_path / "out.csv"
    osm_path.write_text(OSM_MULTI_NAME, encoding="utf-8")

    extract_to_csv(osm_path, out_path, ["8"])

    with out_path.open(newline="", encoding="utf-8") as f:
        rows = list(csv.reader(f))

    names = [row[0] for row in rows[1:]]
    assert names == ["First", "Second", "Erste Straße"]


def test_collect_names_dedupes_semicolon_list():
    class Tag:
        def __init__(self, k, v):
            self.k = k
            self.v = v

    tags = [Tag("name", "Alpha;Beta"), Tag("alt_name", "Beta;Gamma")]
    assert collect_names(tags) == ["Alpha", "Beta", "Gamma"]


def test_polygon_centroid_square():
    coords = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0), (0.0, 0.0)]
    cx, cy = polygon_centroid(coords)
    assert round(cx, 6) == 1.0
    assert round(cy, 6) == 1.0


def test_city_fields_from_boundary_and_tags(tmp_path: Path) -> None:
    osm_path = tmp_path / "boundary.osm"
    out_path = tmp_path / "out.csv"
    osm_path.write_text(OSM_WITH_BOUNDARY, encoding="utf-8")

    extract_to_csv(osm_path, out_path, ["8"])

    with out_path.open(newline="", encoding="utf-8") as f:
        rows = list(csv.reader(f))

    header = rows[0]
    idx = {name: i for i, name in enumerate(header)}
    rows_by_name = {row[0]: row for row in rows[1:]}

    center_row = rows_by_name["Center Road"]
    assert center_row[idx["city"]] == "Test City"
    assert center_row[idx["city_boundary"]] == "Test City"
    assert center_row[idx["city_addr"]] == ""
    assert center_row[idx["city_place"]] == ""

    tagged_row = rows_by_name["Tagged Road"]
    assert tagged_row[idx["city"]] == "Explicit City"
    assert tagged_row[idx["city_addr"]] == "Explicit City"
    assert tagged_row[idx["city_boundary"]] == "Test City"
    assert tagged_row[idx["city_place"]] == "Place City"
