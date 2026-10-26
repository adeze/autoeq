"""Parity tests for canonical EQ filter section HTML."""

import json
from pathlib import Path
import unittest

from scripts.src.report import (
    _all_eq_filters_html,
    _driver_eq_filters_html,
    _eq_filter_summary_html,
    _eq_filter_table_html,
    _eq_passes_list_html,
    _format_eq_filter_line,
)
from scripts.test_report import _driver_eq_split_data


FIXTURE = Path(__file__).parent / "tests" / "fixtures" / "eq_sections_python_goldens.json"
SAFETY_FIXTURE = Path(__file__).parent / "tests" / "fixtures" / "eq_sections_html_safety.json"


def _filters():
    peak = {"filter_type": "peak", "freq": 55.0, "q": 2.0, "db_gain": 1.5}
    warped = {
        "topology": "warped_biquad",
        "filter_type": "highpass",
        "freq": 77.25,
        "q": 0.707,
        "db_gain": -2.5,
    }
    kautz = {
        "topology": "kautz_filter",
        "sections": [
            {"pole_freq": 500.0, "q": 0.707, "gain": 0.123456789},
            {"frequency": 1200.0, "q": 1.25, "gain": -0.000123456},
        ],
    }
    return peak, warped, kautz


def _passes():
    peak, warped, kautz = _filters()
    return [
        {
            "label": "room_eq_correction",
            "display_name": "Room EQ",
            "color": "rgba(100, 100, 255, 0.9)",
            "filters": [peak, warped],
        },
        {"label": "", "display_name": "", "color": "#aaa", "filters": [kautz]},
    ]


class NativeEqSectionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.expected = json.loads(FIXTURE.read_text(encoding="utf-8"))

    def test_filter_lines_preserve_biquad_and_kautz_formatting(self):
        peak, warped, kautz = _filters()
        self.assertEqual(_format_eq_filter_line(peak, 3), self.expected["peak_line"])
        self.assertEqual(_format_eq_filter_line(warped, 1), self.expected["warped_line"])
        self.assertEqual(_format_eq_filter_line(kautz, 2), self.expected["kautz_line"])

    def test_pass_list_and_tables_match_saved_python_html(self):
        self.assertEqual(_eq_passes_list_html(_passes()), self.expected["labeled_passes"])
        self.assertEqual(_eq_filter_table_html(_passes()), self.expected["labeled_table"])

    def test_driver_split_and_all_filter_summary_match_goldens(self):
        data = _driver_eq_split_data()
        self.assertEqual(_driver_eq_filters_html(data, "LFE", 0, "eq_2"), self.expected["driver_both"])
        self.assertEqual(_driver_eq_filters_html(data, "LFE", 1, "eq_3"), self.expected["driver_one"])
        self.assertEqual(_all_eq_filters_html(data), self.expected["all_filters"])

    def test_count_summary_html_matches_saved_python_output(self):
        peak, _, _ = _filters()
        cases = {
            "summary_equal": {"channels": {"L": {"plugins": []}, "R": {"plugins": []}}},
            "summary_varied": {"channels": {
                "L": {"plugins": [{"plugin_type": "eq", "parameters": {"filters": [peak]}}]},
                "R": {"plugins": []},
            }},
            "summary_empty": {"channels": {}},
        }
        for key, data in cases.items():
            with self.subTest(key=key):
                self.assertEqual(_eq_filter_summary_html(data), self.expected[key])

    def test_filter_type_and_driver_labels_are_escaped(self):
        peak, _, _ = _filters()
        peak["filter_type"] = "peak<script>"
        rendered = _format_eq_filter_line(peak, 1)
        self.assertIn("PEAK&lt;SCRIPT&gt;", rendered)
        self.assertNotIn("<script>", rendered)

    def test_prepared_pass_labels_and_colors_are_escaped_in_real_producer(self):
        malicious = json.loads(SAFETY_FIXTURE.read_text(encoding="utf-8"))["passes"]
        for render in (_eq_passes_list_html, _eq_filter_table_html):
            with self.subTest(render=render.__name__):
                html = render(malicious)
                self.assertIn("&lt;img", html)
                self.assertIn("&quot; onmouseover=&quot;alert(1)", html)
                self.assertNotIn("<img", html)
                self.assertNotIn('onmouseover="alert(1)', html)

    def test_driver_id_prefix_is_restricted_to_safe_characters(self):
        data = _driver_eq_split_data()
        id_prefix = json.loads(SAFETY_FIXTURE.read_text(encoding="utf-8"))["id_prefix"]
        html = _driver_eq_filters_html(data, "LFE", 0, id_prefix)
        self.assertNotIn(id_prefix, html)
        self.assertIn("id=\"eq_alert1_drv\"", html)


if __name__ == "__main__":
    unittest.main()
