"""Boundary tests for the canonical EQ figure adapters."""

import json
import math
from pathlib import Path
import unittest
from unittest.mock import patch

from scripts.src.dsp import compute_eq_response
from scripts.src import native_eq_figures
from scripts.src.native_eq_figures import create_eq_figure, create_multipass_eq_figure

FIXTURE = Path(__file__).parent / "tests/fixtures/eq_figures_python_goldens.json"


def assert_transport_parity(test_case, actual, expected, path="$", tolerance=1e-12):
    """Require exact shape/order and tolerate only JSON f64 transport drift."""
    if isinstance(expected, dict):
        test_case.assertIsInstance(actual, dict, path)
        test_case.assertEqual(set(actual), set(expected), path)
        for key, value in expected.items():
            assert_transport_parity(test_case, actual[key], value, f"{path}.{key}", tolerance)
    elif isinstance(expected, list):
        test_case.assertIsInstance(actual, list, path)
        test_case.assertEqual(len(actual), len(expected), path)
        for index, (actual_value, expected_value) in enumerate(zip(actual, expected)):
            assert_transport_parity(
                test_case, actual_value, expected_value, f"{path}[{index}]", tolerance
            )
    elif isinstance(expected, (int, float)) and not isinstance(expected, bool):
        test_case.assertIsInstance(actual, (int, float), path)
        test_case.assertFalse(isinstance(actual, bool), path)
        test_case.assertTrue(math.isfinite(actual), path)
        test_case.assertLessEqual(abs(actual - expected), tolerance, path)
    else:
        test_case.assertEqual(actual, expected, path)


class NativeEqFigureAdapterTests(unittest.TestCase):
    def test_captured_cases_prepare_the_rust_requests(self):
        cases = json.loads(FIXTURE.read_text(encoding="utf-8"))["cases"]
        for case in cases:
            with self.subTest(case=case["name"]):
                received = []

                def producer(request):
                    received.append(request)
                    return "null"

                with patch.object(native_eq_figures, "produce", side_effect=producer):
                    if case["builder"] == "create_eq_figure":
                        result = create_eq_figure(**case["args"])
                    else:
                        result = create_multipass_eq_figure(**case["args"])

                if case["expected"] is None:
                    self.assertIsNone(result)
                    self.assertEqual(received, [])
                else:
                    self.assertEqual(len(received), 1)
                    assert_transport_parity(self, received[0], case["request"])

    def test_single_builder_receives_dsp_prepared_combined_and_filter_arrays(self):
        filt = {"filter_type": "peak", "freq": 1200.0, "q": 0.8, "db_gain": 3.5}
        received = []

        def producer(request):
            received.append(request)
            return json.dumps({"kind": "figure"})

        with patch.object(native_eq_figures, "produce", side_effect=producer):
            result = create_eq_figure("L", [filt], sample_rate=32000.0, tab="eq")

        self.assertEqual(result, {"kind": "figure"})
        request = received[0]
        self.assertEqual(request["operation"], "eq_figure")
        self.assertEqual(request["builder"], "single")
        self.assertEqual(request["channel_name"], "L")
        self.assertEqual(request["tab"], "eq")
        frequencies = request["freq_points"]
        self.assertEqual(len(frequencies), 500)
        self.assertLessEqual(max(frequencies), 16000.0)
        self.assertEqual(
            request["combined_response"],
            compute_eq_response([filt], frequencies, 32000.0),
        )
        self.assertEqual(request["filters"][0]["metadata"], filt)
        self.assertEqual(
            request["filters"][0]["response"],
            compute_eq_response([filt], frequencies, 32000.0),
        )

    def test_labeled_multipass_keeps_pass_order_and_precomputed_combined_grid(self):
        first = {"filter_type": "peak", "freq": 500.0, "q": 1.0, "db_gain": 2.0}
        second = {"filter_type": "peak", "freq": 3000.0, "q": 1.0, "db_gain": -1.0}
        channel = {
            "plugins": [
                {"plugin_type": "eq", "parameters": {"label": "cea2034_speaker_correction", "filters": [first]}},
                {"plugin_type": "eq", "parameters": {"label": "user_preference", "filters": [second]}},
            ]
        }
        combined = {"freq": [40.0, 700.0, 12000.0], "spl": [1.0, -2.0, 0.5]}
        received = []

        def producer(request):
            received.append(request)
            return "null"

        with patch.object(native_eq_figures, "produce", side_effect=producer):
            self.assertIsNone(
                create_multipass_eq_figure("L", channel, combined, sample_rate=44100.0)
            )

        request = received[0]
        self.assertEqual(request["builder"], "multipass")
        self.assertEqual(request["combined_freq_points"], combined["freq"])
        self.assertEqual(request["combined_response"], combined["spl"])
        self.assertEqual(
            [entry["display_name"] for entry in request["passes"]],
            ["Pass 1: Speaker Correction (CEA2034)", "Pass 3: User Preference"],
        )
        frequencies = request["freq_points"]
        self.assertLessEqual(max(frequencies), 22050.0)
        self.assertEqual(
            request["passes"][0]["response"],
            compute_eq_response([first], frequencies, 44100.0),
        )


if __name__ == "__main__":
    unittest.main()
