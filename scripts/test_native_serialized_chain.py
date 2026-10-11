"""Parity and intentional-correction tests for canonical plugin-chain replay."""

import json
import math
from pathlib import Path
import unittest
from unittest.mock import patch

from scripts.src.dsp import apply_plugins_to_curve
from scripts.src.native_serialized_chain import serialized_plugin_chain_response


GOLDENS = Path(__file__).parent / "tests" / "fixtures" / "serialized_chain_python_goldens.json"
PARITY_CASES = {
    "gain_positive",
    "gain_inverted",
    "delay_48k",
    "peq_48k",
    "peq_96k_near_nyquist",
    "lr24_lowpass_96k",
    "ordered_gain_peq_delay_crossover",
    "gain_without_input_phase",
}


class NativeSerializedChainTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(GOLDENS.read_text(encoding="utf-8"))

    def test_legacy_python_goldens_for_unchanged_chains(self):
        for case in self.fixture["cases"]:
            if case["name"] not in PARITY_CASES:
                continue
            with self.subTest(case=case["name"]):
                result = apply_plugins_to_curve(case["input"], case["plugins"], case["sample_rate"])
                self.assertEqual(result["freq"], case["result"]["freq"])
                self.assertEqual("phase" in result, "phase" in case["result"])
                for actual, expected in zip(result["spl"], case["result"]["spl"]):
                    self.assertAlmostEqual(actual, expected, delta=1e-9)
                if "phase" in result:
                    for actual, expected in zip(result["phase"], case["result"]["phase"]):
                        # The legacy chain can carry -0.0 to select -180 while
                        # Rust's canonical complex response selects +180. These
                        # are the same phasor; compare the phase difference on
                        # the circle rather than inferring Python stage history.
                        wrapped_delta = abs((actual - expected + 180.0) % 360.0 - 180.0)
                        self.assertAlmostEqual(wrapped_delta, 0.0, delta=1e-9)

    def test_inversion_negative_real_axis_is_phase_equivalent(self):
        case = next(case for case in self.fixture["cases"] if case["name"] == "gain_inverted")
        result = apply_plugins_to_curve(case["input"], case["plugins"], case["sample_rate"])
        actual = result["phase"][0]
        legacy = case["result"]["phase"][0]
        self.assertEqual(actual, 350.0)
        self.assertEqual(legacy, -10.0)
        self.assertAlmostEqual(abs((actual - legacy + 180.0) % 360.0 - 180.0), 0.0, delta=1e-9)

    def test_fir_gain_and_dry_wet_are_honored_by_canonical_chain(self):
        case = next(case for case in self.fixture["cases"]
                    if case["name"] == "per_driver_fir_ignores_gain_and_mix")
        curve = case["input"]
        plugin = case["plugins"]
        result = apply_plugins_to_curve(curve, plugin, case["sample_rate"])
        parameters = plugin[0]["parameters"]
        from scripts.src.native_fir_response import fir_response

        wet = fir_response(curve["freq"], case["sample_rate"], taps=parameters["_fir_taps"])
        gain = 10.0 ** (parameters["gain_db"] / 20.0)
        mix = parameters["mix"]
        for index, response in enumerate(wet):
            expected = (1.0 - mix) + response * (mix * gain)
            expected_spl = 20.0 * math.log10(max(
                10.0 ** (curve["spl"][index] / 20.0) * abs(expected), 1.0e-12))
            expected_phase = curve["phase"][index] + math.degrees(math.atan2(expected.imag, expected.real))
            self.assertAlmostEqual(result["spl"][index], expected_spl, delta=1e-9)
            self.assertAlmostEqual(result["phase"][index], expected_phase, delta=1e-9)
        self.assertNotEqual(result, case["result"], "legacy Python dropped the FIR mix and gain")

    def test_unavailable_ordinary_convolution_fails_closed(self):
        with self.assertRaisesRegex(ValueError, "Convolution FIR resource.*unavailable"):
            apply_plugins_to_curve(
                {"freq": [100.0], "spl": [80.0]},
                [{"plugin_type": "convolution", "parameters": {"ir_file": "missing.wav"}}],
            )

    def test_output_both_is_unity_and_not_python_highpass_correction(self):
        curve = {"freq": [0.0, 24_000.0], "spl": [80.0, 80.0], "phase": [0.0, 0.0]}
        result = apply_plugins_to_curve(curve, [{"plugin_type": "crossover", "parameters": {
            "type": "LR24", "output": "both", "frequency": 120.0}}], 48_000.0)
        for level in result["spl"]:
            self.assertAlmostEqual(level, 80.0, delta=1e-9)
        self.assertEqual(result["phase"], [0.0, 0.0])

    def test_band_split_merge_performs_two_band_reconstruction(self):
        frequencies = [0.0, 24_000.0]
        transfer = serialized_plugin_chain_response([
            {"plugin_type": "band_split", "parameters": {"type": "LR24", "frequency": 120.0}},
            {"plugin_type": "gain", "parameters": {"gain_db": 6.0, "channels": [0]}},
            {"plugin_type": "band_merge", "parameters": {"bands": 2}},
        ], frequencies, 48_000.0)
        self.assertAlmostEqual(abs(transfer[0]), 10.0 ** (6.0 / 20.0), delta=1e-9)
        self.assertAlmostEqual(abs(transfer[1]), 1.0, delta=1e-9)

    def test_unknown_plugin_is_rejected_by_canonical_dispatch(self):
        with self.assertRaisesRegex(ValueError, "unsupported serialized RoomEQ plugin"):
            serialized_plugin_chain_response([
                {"plugin_type": "vendor_dynamic_processor", "parameters": {"amount": 0.9}},
            ], [100.0], 48_000.0)

    def test_native_fir_identity_is_resolved_and_hidden_cache_is_not_serialized(self):
        parameters = {
            "ir_file": "driver/woofer.wav",
            "room_eq_fir_placement": "per_driver",
            "_native_fir_resource_status": "available",
            "_native_fir_sample_rate": 48_000.0,
            "_native_fir_resource_key": "driver/woofer.wav",
            "_native_fir_graph_sha256": "a" * 64,
            "_native_fir_expected_resource_sha256": "b" * 64,
        }
        with patch("scripts.src.native_fir_response.produce", return_value="[[1,0],[0,1]]") as fir_produce:
            with patch("scripts.src.native_serialized_chain.produce", return_value="[[1,0],[0,1]]") as chain_produce:
                serialized_plugin_chain_response(
                    [{"plugin_type": "convolution", "parameters": parameters}],
                    [0.0, 12_000.0], 48_000.0)
        fir_produce.assert_called_once_with({
            "operation": "fir_response", "frequencies": [0.0, 12_000.0],
            "sample_rate": 48_000.0, "resource_key": "driver/woofer.wav",
            "graph_sha256": "a" * 64, "resource_sha256": "b" * 64,
        })
        request = chain_produce.call_args.args[0]
        self.assertEqual(request["fir_responses"], {"driver/woofer.wav": [[1, 0], [0, 1]]})
        self.assertNotIn("_native_fir_graph_sha256", json.dumps(request))

    def test_unavailable_native_resource_never_falls_back_to_cached_taps(self):
        parameters = {
            "ir_file": "driver/woofer.wav", "_fir_sample_rate": 48_000.0, "_fir_taps": [1.0],
            "_native_fir_resource_status": "unavailable",
            "_native_fir_resource_reason": "manifest is unverified",
        }
        with patch("scripts.src.native_fir_response.produce") as fir_produce:
            with self.assertRaisesRegex(ValueError, "manifest is unverified"):
                serialized_plugin_chain_response(
                    [{"plugin_type": "convolution", "parameters": parameters}],
                    [100.0], 48_000.0)
        fir_produce.assert_not_called()

    def test_response_grid_is_nonempty_finite_and_within_nyquist(self):
        for grid in ([], [float("nan")], [-1.0], [24_001.0]):
            with self.subTest(grid=grid), self.assertRaises(ValueError):
                serialized_plugin_chain_response([], grid, 48_000.0)

    def test_legacy_missing_parameters_are_normalized_before_dispatch(self):
        with patch("scripts.src.native_serialized_chain.produce", return_value="[[1,0]]") as producer:
            serialized_plugin_chain_response([
                {"plugin_type": "GAIN", "parameters": {}},
                {"plugin_type": "delay", "parameters": {}},
                {"plugin_type": "crossover", "parameters": {}},
                {"plugin_type": "eq", "parameters": {}},
            ], [100.0], 48_000.0)
        request = producer.call_args.args[0]
        self.assertEqual(request["plugins"], [
            {"plugin_type": "gain", "parameters": {"gain_db": 0.0}},
            {"plugin_type": "delay", "parameters": {"delay_ms": 0.0}},
            {"plugin_type": "crossover", "parameters": {
                "type": "LR24", "frequency": 80.0, "output": "low"}},
            {"plugin_type": "eq", "parameters": {"filters": []}},
        ])


if __name__ == "__main__":
    unittest.main()
