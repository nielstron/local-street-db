import os
import sys
import unittest

SCRIPT_DIR = os.path.join(os.path.dirname(__file__), "..", "scripts")
sys.path.insert(0, os.path.abspath(SCRIPT_DIR))

import download_country_pbfs as dl


class DownloadCountryPbfsTests(unittest.TestCase):
    def test_filter_continent_dirs(self):
        base_url = "https://download.openstreetmap.fr/extracts/"
        links = [
            "../",
            "africa/",
            "europe/",
            "notes.txt",
            "?C=M;O=A",
        ]
        expected = [
            "https://download.openstreetmap.fr/extracts/africa/",
            "https://download.openstreetmap.fr/extracts/europe/",
        ]
        self.assertEqual(dl._filter_continent_dirs(base_url, links), expected)

    def test_filter_country_pbfs(self):
        continent_url = "https://download.openstreetmap.fr/extracts/europe/"
        links = [
            "france.osm.pbf",
            "germany.osm.pbf",
            "france/",
            "france/ile-de-france.osm.pbf",
        ]
        expected = [
            ("https://download.openstreetmap.fr/extracts/europe/france.osm.pbf", "france.osm.pbf"),
            ("https://download.openstreetmap.fr/extracts/europe/germany.osm.pbf", "germany.osm.pbf"),
        ]
        self.assertEqual(dl._filter_country_pbfs(continent_url, links), expected)


if __name__ == "__main__":
    unittest.main()
