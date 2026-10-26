"""Adapter tests for the canonical response-figure operations."""

import json
import math
import unittest
from unittest.mock import patch

from scripts.src import native_response_figures


class NativeResponseFigureTests(unittest.TestCase):
    def test_response_figure_operation_decodes_shared_payload(self):
        requests = []

        def produce(request):
            requests.append(request)
            return json.dumps({"channel_figure": {"kind": "figure"}})

        with patch.object(native_response_figures, "produce", side_effect=produce):
            self.assertEqual(
                native_response_figures.response_figure_report({"channel_name": "L"}),
                {"channel_figure": {"kind": "figure"}},
            )

        self.assertEqual(requests[0]["operation"], "response_figures")
        self.assertEqual(requests[0]["data"], {"channel_name": "L"})

    def test_resampling_operation_and_nonfinite_input_normalization(self):
        requests = []

        def produce(request):
            requests.append(request)
            return "[0.0, 1.0]"

        with patch.object(native_response_figures, "produce", side_effect=produce):
            self.assertEqual(
                native_response_figures.resample_spl_onto_grid(
                    [20.0, math.nan], [1.0, 2.0], [10.0, 30.0]
                ),
                [0.0, 1.0],
            )

        self.assertEqual(requests[0]["operation"], "resample_spl_onto_grid")
        self.assertIsNone(requests[0]["source_freq"][1])
        json.dumps(requests, allow_nan=False)

    def test_overlay_operation_preserves_legacy_in_place_identity(self):
        original = {"figure": {"series": []}, "tab": "left"}

        def produce(request):
            self.assertEqual(request["operation"], "add_response_overlays")
            self.assertEqual(request["figure"], original)
            self.assertEqual(request["channel_name"], "L")
            self.assertEqual(request["target_curve"], {"freq": [100], "spl": [0]})
            return json.dumps({"figure": {"series": [{"name": "Target"}]}, "tab": "left"})

        with patch.object(native_response_figures, "produce", side_effect=produce):
            updated = native_response_figures.add_channel_response_overlays(
                original, "L", {"freq": [100], "spl": [0]}, None
            )

        self.assertIs(updated, original)
        self.assertEqual(original["figure"]["series"][0]["name"], "Target")


if __name__ == "__main__":
    unittest.main()
