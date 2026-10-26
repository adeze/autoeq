"""Fail-closed consumption of authoritative native routed source curves."""
import unittest
from unittest.mock import patch

from scripts.src.loaders import RoomEqData
from scripts.src.dsp import build_post_dsp_source_curves


class NativeSourceReplayTests(unittest.TestCase):
    def data(self, table):
        data = RoomEqData({"channels": {"L": {"final_curve": {
            "freq": [100, 1000], "spl": [99, 99]
        }}}, "metadata": {"bass_management": {"routing_graph": {
            "input_channels": ["L", "LFE"]
        }}}}, None)
        data.native_dsp_replay_present = True
        data.native_dsp_replay = {"sample_rate_hz": None, "routed_sources": table}
        self.addCleanup(data.close)
        return data

    def test_saved_unknown_phase_is_used_without_inventing_rate(self):
        curve = {"freq": [100, 1000], "spl": [80, 81], "phase": None}
        data = self.data({"L": {"status": "available", "curve": curve},
                          "LFE": {"status": "unavailable", "reason": "No measurement"}})
        with patch("scripts.src.dsp.apply_plugins_to_curve", side_effect=AssertionError("no replay")):
            self.assertEqual(build_post_dsp_source_curves(data), {"L": curve})
        self.assertEqual(data.native_routed_source_unavailability, {"LFE": "No measurement"})

    def test_missing_and_malformed_native_sources_do_not_use_final_curve(self):
        for table in [None, {}, {"L": {"status": "available", "curve": {
            "freq": [100, 100], "spl": [80, 80]
        }}}]:
            with self.subTest(table=table):
                data = self.data(table)
                with patch("scripts.src.dsp.apply_plugins_to_curve", side_effect=AssertionError("no replay")):
                    self.assertEqual(build_post_dsp_source_curves(data), {})
                self.assertIn("L", data.native_routed_source_unavailability)

    def test_logical_sources_remain_separate(self):
        left = {"freq": [100, 1000], "spl": [80, 81]}
        lfe = {"freq": [100, 1000], "spl": [90, 91]}
        data = self.data({"L": {"status": "available", "curve": left},
                          "LFE": {"status": "available", "curve": lfe}})
        self.assertEqual(build_post_dsp_source_curves(data), {"L": left, "LFE": lfe})
        self.assertEqual(data.native_routed_source_unavailability, {})

    def test_markerless_legacy_saved_curves_still_work(self):
        curve = {"freq": [100, 1000], "spl": [80, 81]}
        data = RoomEqData({"deployed_source_curves": {"L": curve}}, None)
        self.addCleanup(data.close)
        self.assertEqual(build_post_dsp_source_curves(data), {"L": curve})


if __name__ == "__main__":
    unittest.main()
