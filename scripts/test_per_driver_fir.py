"""Independent exported-tap replay contract for physical FIR plotting."""
import json
import struct
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from scripts.src.loaders import load_roomeq_json, read_fir_wav
from scripts.src.dsp import apply_plugins_to_curve, complex_sum_curves


class PerDriverFirReplay(unittest.TestCase):
    def test_host_fir_resource_request_uses_exact_frozen_identity(self):
        class NativeFirParameters(dict):
            replay_metadata = {
                "_native_fir_resource_status": "available",
                "_native_fir_resource_reason": None,
                "_native_fir_sample_rate": 48_000,
                "_native_fir_resource_key": "driver/woofer.wav",
                "_native_fir_graph_sha256": "a" * 64,
                "_native_fir_expected_resource_sha256": "b" * 64,
            }

            def get(self, key, default=None):
                if key in self.replay_metadata:
                    return self.replay_metadata[key]
                return super().get(key, default)

        parameters = NativeFirParameters({
            "room_eq_fir_placement": "per_driver",
            "ir_file": "driver/woofer.wav",
        })
        plugin = {"plugin_type": "convolution", "parameters": parameters}
        with patch("scripts.src.native_fir_response.produce", return_value="[[0,-0.5]]") as produce:
            result = apply_plugins_to_curve(
                {"freq": [12_000.0], "spl": [0.0], "phase": [0.0]}, [plugin]
            )
        self.assertAlmostEqual(result["spl"][0], -6.020599913, places=7)
        self.assertAlmostEqual(result["phase"][0], -90.0)
        produce.assert_called_once_with({
            "operation": "fir_response",
            "frequencies": [12_000.0],
            "sample_rate": 48_000.0,
            "resource_key": "driver/woofer.wav",
            "graph_sha256": "a" * 64,
            "resource_sha256": "b" * 64,
        })

    def test_unavailable_native_fir_does_not_fall_back_to_taps_or_paths(self):
        class UnavailableFirParameters(dict):
            replay_metadata = {
                "_native_fir_resource_status": "unavailable",
                "_native_fir_resource_reason": "legacy bundle is unverified",
                "_fir_sample_rate": 48_000,
                "_fir_taps": [1.0],
            }

            def get(self, key, default=None):
                if key in self.replay_metadata:
                    return self.replay_metadata[key]
                return super().get(key, default)

        plugin = {"plugin_type": "convolution", "parameters": UnavailableFirParameters({
            "room_eq_fir_placement": "per_driver", "ir_file": "untrusted.wav"})}
        with patch("scripts.src.native_fir_response.produce") as produce:
            with self.assertRaisesRegex(ValueError, "legacy bundle is unverified"):
                apply_plugins_to_curve({"freq": [100.0], "spl": [80.0]}, [plugin])
        produce.assert_not_called()

    def test_distinct_physical_firs_are_applied_before_acoustic_sum(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            drivers = []
            for index, taps in enumerate(([1.0, 0.0], [0.0, 0.5])):
                payload = struct.pack("<2f", *taps)
                fmt = struct.pack("<HHIIHH", 3, 1, 48000, 192000, 4, 32)
                wav = b"WAVEfmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(payload)) + payload
                name = f"driver{index}.wav"
                (root / name).write_bytes(b"RIFF" + struct.pack("<I", len(wav)) + wav)
                drivers.append({"plugins": [{"plugin_type": "convolution", "parameters": {
                    "room_eq_fir_placement": "per_driver", "ir_file": name}}]})
            path = root / "output.json"
            path.write_text(json.dumps({"channels": {"L": {"drivers": drivers}}}))
            data = load_roomeq_json(path)
            curve = {"freq": [12000.0], "spl": [0.0], "phase": [0.0]}
            (root / "driver0.wav").unlink()
            (root / "driver1.wav").write_bytes(b"mutated after load")
            corrected = [apply_plugins_to_curve(curve, driver["plugins"]) for driver in data["channels"]["L"]["drivers"]]
            self.assertAlmostEqual(corrected[1]["phase"][0], -90.0)
            self.assertAlmostEqual(corrected[1]["spl"][0], -6.020599913, places=7)
            summed = complex_sum_curves(*corrected)
            self.assertAlmostEqual(summed["spl"][0], 0.96910013, places=6)

    def test_missing_physical_taps_never_silently_plot_iir_only(self):
        with self.assertRaises(ValueError):
            apply_plugins_to_curve({"freq": [100.0], "spl": [80.0]}, [{"plugin_type": "convolution", "parameters": {
                "room_eq_fir_placement": "per_driver", "ir_file": "missing.wav"}}])

    def test_invalid_wav_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "invalid.wav"
            path.write_bytes(b"not a wav")
            with self.assertRaises(ValueError):
                read_fir_wav(path)


if __name__ == "__main__":
    unittest.main()
