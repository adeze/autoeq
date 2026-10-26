"""Parity and request-contract tests for native FIR response evaluation."""

import json
from pathlib import Path
import unittest
from unittest.mock import patch

from scripts.src.dsp import apply_plugins_to_curve
from scripts.src.native_fir_response import fir_response


FIXTURE = Path(__file__).parent / "tests" / "fixtures" / "per_driver_fir_python_goldens.json"


class NativeFirResponseTests(unittest.TestCase):
    def test_standalone_pipeline_matches_captured_python_goldens(self):
        fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
        for case in fixture["cases"]:
            with self.subTest(taps=case["taps"]):
                source = {
                    "freq": case["frequencies"],
                    "spl": [fixture["source_spl"]] * len(case["frequencies"]),
                    "phase": [0.0] * len(case["frequencies"]),
                }
                plugin = {
                    "plugin_type": "convolution",
                    "parameters": {
                        "room_eq_fir_placement": "per_driver",
                        "ir_file": "captured-fixture.wav",
                        "_fir_sample_rate": case["sample_rate"],
                        "_fir_taps": case["taps"],
                    },
                }
                result = apply_plugins_to_curve(source, [plugin])
                for actual, expected in zip(result["spl"], case["spl"]):
                    self.assertAlmostEqual(actual, expected, delta=1e-10)
                for actual, expected in zip(result["phase"], case["phase"]):
                    self.assertAlmostEqual(actual, expected, delta=1e-10)

    def test_resource_request_binds_exact_frozen_identity(self):
        with patch("scripts.src.native_fir_response.produce", return_value="[[0,-0.5]]") as produce:
            result = fir_response(
                [12_000.0], 48_000.0,
                resource_key="driver/woofer.wav",
                graph_sha256="a" * 64,
                resource_sha256="b" * 64,
            )
        self.assertEqual(result, [complex(0.0, -0.5)])
        produce.assert_called_once_with({
            "operation": "fir_response",
            "frequencies": [12_000.0],
            "sample_rate": 48_000.0,
            "resource_key": "driver/woofer.wav",
            "graph_sha256": "a" * 64,
            "resource_sha256": "b" * 64,
        })

    def test_standalone_request_uses_only_captured_taps(self):
        with patch("scripts.src.native_fir_response.produce", return_value="[[0.5,0]]") as produce:
            self.assertEqual(fir_response([0.0], 48_000.0, taps=[0.5]), [0.5 + 0j])
        produce.assert_called_once_with({
            "operation": "fir_response",
            "frequencies": [0.0],
            "sample_rate": 48_000.0,
            "taps": [0.5],
        })

    def test_resource_key_requires_graph_and_resource_hashes(self):
        with self.assertRaisesRegex(ValueError, "graph SHA-256"):
            fir_response([100.0], 48_000.0, resource_key="fir.wav", resource_sha256="b" * 64)

    def test_invalid_grid_and_nonfinite_response_are_rejected(self):
        for frequency in (float("nan"), float("inf"), -1.0, 24_001.0):
            with self.subTest(frequency=frequency), self.assertRaises(ValueError):
                fir_response([frequency], 48_000.0, taps=[1.0])
        with patch("scripts.src.native_fir_response.produce", return_value="not json"):
            with self.assertRaises(ValueError):
                fir_response([100.0], 48_000.0, taps=[1.0])

    def test_producer_errors_remain_unavailable(self):
        with patch("scripts.src.native_fir_response.produce", side_effect=RuntimeError("resource hash mismatch")):
            with self.assertRaisesRegex(ValueError, "resource hash mismatch"):
                fir_response([100.0], 48_000.0, resource_key="fir.wav",
                             graph_sha256="a" * 64, resource_sha256="b" * 64)


if __name__ == "__main__":
    unittest.main()
