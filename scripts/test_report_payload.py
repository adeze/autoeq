"""Canonical dashboard payload contract for native/shared-WASM consumers."""

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from scripts.src.report import (
    build_report_payload, create_html_report,
)
from scripts.test_report import _driver_eq_split_data


class ReportPayloadTests(unittest.TestCase):
    def test_dataset_composer_owns_canonical_shared_sections(self):
        data = _driver_eq_split_data()
        with patch("scripts.src.report._report_assembly", side_effect=AssertionError("Python assembly")):
            payload = build_report_payload(
                data,
                shared_tof_html="legacy tof",
                shared_sections={
                    "crossover_config_html": "",
                    "time_of_flight_html": "shared tof",
                },
            )

        html = [section.get("html", "") for section in payload["sections"]]
        self.assertNotIn("legacy tof", html)
        self.assertNotIn("shared tof", html)
        self.assertTrue(any("Crossover Configuration" in section for section in html))

    def test_empty_shared_section_map_does_not_fall_back_to_python_assembly(self):
        data = _driver_eq_split_data()
        with (
            patch("scripts.src.report._rust_shared_sections", side_effect=AssertionError("legacy fallback")),
            patch("scripts.src.report._report_assembly", side_effect=AssertionError("Python assembly")),
        ):
            payload = build_report_payload(data, shared_sections={})

        self.assertTrue(any(
            "Crossover Configuration" in section.get("html", "")
            for section in payload["sections"]
        ))

    def test_payload_preserves_full_report_without_loading_renderer_assets(self):
        data = _driver_eq_split_data()
        with patch("scripts.src.report.wasm_report.load_assets", side_effect=AssertionError("asset I/O")):
            payload = build_report_payload(data)
        encoded = json.dumps(payload, allow_nan=False)
        self.assertEqual(payload["schema"], "autoeq-report-data-v1")
        self.assertEqual(payload["title"], "Room EQ")
        self.assertEqual({s.get("bucket") for s in payload["sections"] if s.get("bucket")}, {
            "DSP analysis", "Acoustics analysis", "Psychoacoustic report",
        })
        self.assertIn("DSP signal flow", encoded)
        self.assertIn("Details per speaker", encoded)
        self.assertIn("55", encoded)
        self.assertTrue(any(s["kind"] == "figure" for s in payload["sections"]))

    def test_html_entrypoint_delegates_same_full_payload(self):
        data = _driver_eq_split_data()
        expected = build_report_payload(data)
        with (
            patch("scripts.src.report.build_report_payload", return_value=expected),
            patch("scripts.src.report.wasm_report.write_report") as writer,
            tempfile.TemporaryDirectory() as directory,
        ):
            path = Path(directory) / "report.html"
            create_html_report(data, path)
            writer.assert_called_once_with(path, "Room EQ", expected)


if __name__ == "__main__":
    unittest.main()
