"""Boundary tests for the canonical EQ figure adapters."""

import json
from pathlib import Path
import unittest

from scripts.src.dsp import compute_eq_response
from scripts.src.native_report import using_producer
from scripts.src.native_eq_figures import create_eq_figure, create_multipass_eq_figure

FIXTURE = Path(__file__).parent / "tests/fixtures/eq_figures_python_goldens.json"


class NativeEqFigureAdapterTests(unittest.TestCase):
    def test_captured_cases_prepare_the_rust_requests(self):
        cases = json.loads(FIXTURE.read_text(encoding="utf-8"))["cases"]
        for case in cases:
            with self.subTest(case=case["name"]):
                received = []

                def producer(request):
                    received.append(request)
                    return "null"

                with using_producer(producer):
                    if case["builder"] == "create_eq_figure":
                        result = create_eq_figure(**case["args"])
                    else:
                        result = create_multipass_eq_figure(**case["args"])

                if case["expected"] is None:
                    self.assertIsNone(result)
                    self.assertEqual(received, [])
                else:
                    self.assertEqual(received, [case["request"]])

    def test_single_builder_receives_dsp_prepared_combined_and_filter_arrays(self):
        filt = {"filter_type": "peak", "freq": 1200.0, "q": 0.8, "db_gain": 3.5}
        received = []

        def producer(request):
            received.append(request)
            return json.dumps({"kind": "figure"})

        with using_producer(producer):
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

        with using_producer(producer):
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
