"""Host-side validation for frozen native acoustic driver curves."""

import copy
import json
import unittest
from unittest.mock import patch

from scripts.src.loaders import RoomEqData
from scripts.src.native_driver_replay import (
    resolve_native_driver_curve,
    validate_native_curve,
)
from scripts.src.report import build_report_payload
from scripts.test_figures import two_sub_overview_data


def _curve(*, phase=None, auxiliaries=False):
    result = {"freq": [40.0, 80.0, 160.0], "spl": [70.0, 71.0, 69.0]}
    if phase is not None:
        result["phase"] = list(phase)
    if auxiliaries:
        result["noise_floor_db"] = [20.0, 21.0, 22.0]
        result["coherence"] = [0.9, 0.8, 0.7]
        result["norm_range"] = [40.0, 160.0]
    return result


def _driver(index=4, name="woofer", *, phase=None, auxiliaries=False):
    return {
        "index": index,
        "name": name,
        "initial_curve": _curve(phase=phase, auxiliaries=auxiliaries),
        "plugins": [],
    }


def _data(driver, state=None):
    if state is None:
        state = {"status": "available", "curve": copy.deepcopy(driver["initial_curve"])}
    data = RoomEqData({"channels": {"LFE": {"drivers": [driver]}}}, ".")
    data.native_dsp_replay_present = True
    data.native_playback_sample_rate = 48000
    data.native_dsp_replay = {
        "sample_rate_hz": 48000,
        "drivers": {
            "LFE": [{"name": driver["name"], "index": driver["index"], "state": state}]
        },
    }
    return data


class NativeCurveValidationTests(unittest.TestCase):
    def test_valid_curve_keeps_unknown_phase_and_auxiliary_evidence(self):
        curve = _curve(auxiliaries=True)
        self.assertEqual(validate_native_curve(curve), (True, None))
        data = _data(_driver(auxiliaries=True), {"status": "available", "curve": curve})
        handled, selected, reason = resolve_native_driver_curve(data, "LFE", _driver(auxiliaries=True))
        self.assertTrue(handled)
        self.assertIsNone(reason)
        self.assertEqual(selected, curve)
        self.assertNotIn("phase", selected)
        self.assertEqual(selected["coherence"], [0.9, 0.8, 0.7])

    def test_identity_uses_original_index_and_name(self):
        driver = _driver(index=4)
        data = _data(driver)
        data.native_dsp_replay["drivers"]["LFE"].insert(
            0, {"name": "filtered-out-driver", "index": 0, "state": {"status": "unavailable", "reason": "no curve"}}
        )
        handled, curve, reason = resolve_native_driver_curve(data, "LFE", driver)
        self.assertTrue(handled)
        self.assertEqual(curve, driver["initial_curve"])
        self.assertIsNone(reason)

    def test_band_binding_uses_engine_inclusive_bins_and_preserves_aux_values(self):
        driver = _driver(auxiliaries=True)
        driver["initial_curve"]["freq"] = [79.99999999, 80.0, 160.0]
        driver["initial_curve"]["noise_floor_db"] = [10.0, 11.0, 12.0]
        driver["initial_curve"]["coherence"] = [0.6, 0.7, 0.8]
        driver["initial_curve"]["spl"] = [68.0, 70.0, 69.0]
        driver["measured_band_hz"] = [80.0, 160.0]
        prepared = {
            "freq": [80.0, 160.0],
            "spl": [70.0, 69.0],
            "noise_floor_db": [11.0, 12.0],
            "coherence": [0.7, 0.8],
            "norm_range": [40.0, 160.0],
        }
        data = _data(driver, {"status": "available", "curve": prepared})
        handled, curve, reason = resolve_native_driver_curve(data, "LFE", driver)
        self.assertTrue(handled)
        self.assertIsNone(reason)
        self.assertEqual(curve, prepared)

    def test_unavailable_result_is_explicit_and_does_not_become_a_curve(self):
        driver = _driver()
        data = _data(driver, {"status": "unavailable", "reason": "global routing stage ownership unsupported"})
        handled, curve, reason = resolve_native_driver_curve(data, "LFE", driver)
        self.assertTrue(handled)
        self.assertIsNone(curve)
        self.assertEqual(reason, "global routing stage ownership unsupported")

    def test_missing_marker_retains_standalone_legacy_behavior(self):
        driver = _driver()
        data = RoomEqData({"channels": {"LFE": {"drivers": [driver]}}, "sample_rate": 48000}, ".")
        self.assertEqual(resolve_native_driver_curve(data, "LFE", driver), (False, None, None))

    def test_rate_identity_grid_phase_and_auxiliary_mismatches_fail_closed(self):
        driver = _driver(auxiliaries=True)
        mutations = [
            (lambda d: d.native_dsp_replay.update(sample_rate_hz=True), "sample rate"),
            (lambda d: setattr(d, "native_playback_sample_rate", 44100), "does not match"),
            (lambda d: d.native_dsp_replay["drivers"]["LFE"][0]["state"]["curve"].update(freq=[40.0, 81.0, 160.0]), "grid"),
            (lambda d: d.native_dsp_replay["drivers"]["LFE"][0]["state"]["curve"].update(phase=[0.0, 0.0, 0.0]), "unknown phase"),
            (lambda d: d.native_dsp_replay["drivers"]["LFE"][0]["state"]["curve"].pop("coherence"), "dropped measured coherence"),
            (lambda d: d.native_dsp_replay["drivers"]["LFE"].append(copy.deepcopy(d.native_dsp_replay["drivers"]["LFE"][0])), "ambiguous"),
        ]
        for mutate, expected in mutations:
            with self.subTest(expected=expected):
                data = _data(driver)
                mutate(data)
                handled, curve, reason = resolve_native_driver_curve(data, "LFE", driver)
                self.assertTrue(handled)
                self.assertIsNone(curve)
                self.assertIn(expected, reason)

    def test_generic_validator_rejects_bad_grid_and_nonfinite_auxiliary_data(self):
        bad_grids = [
            {"freq": [1.0, 1.0], "spl": [0.0, 0.0]},
            {"freq": [1.0, float("nan")], "spl": [0.0, 0.0]},
            {"freq": [1.0, 2.0], "spl": [0.0, float("inf")]},
            {"freq": [1.0, 2.0], "spl": [0.0, 1.0], "coherence": [1.0]},
            {"freq": [10**1000], "spl": [0.0]},
        ]
        for curve in bad_grids:
            with self.subTest(curve=curve):
                valid, reason = validate_native_curve(curve)
                self.assertFalse(valid)
                self.assertTrue(reason)


class NativeDriverReportTests(unittest.TestCase):
    def _report_data(self, *, unavailable=False):
        payload = two_sub_overview_data()
        rows = payload["channels"]["LFE"]["drivers"]
        for index, driver in enumerate(rows):
            driver["index"] = index
        data = RoomEqData(payload, ".")
        data.native_dsp_replay_present = True
        data.native_playback_sample_rate = 48000
        data.native_dsp_replay = {
            "sample_rate_hz": 48000,
            "drivers": {
                "LFE": [
                    {
                        "name": driver["name"],
                        "index": index,
                        "state": (
                            {"status": "unavailable", "reason": "frozen FIR resource unavailable"}
                            if unavailable and index == 0
                            else {"status": "available", "curve": copy.deepcopy(driver["initial_curve"])}
                        ),
                    }
                    for index, driver in enumerate(rows)
                ]
            },
        }
        return data

    def _build_without_python_replay(self, data):
        from scripts.src import native_dataset_adapter

        with (
            patch("scripts.src.dsp.build_post_dsp_source_curves",
                  side_effect=AssertionError("legacy source replay")),
            patch("scripts.src.dsp.per_driver_corrected_curve",
                  side_effect=AssertionError("legacy driver replay")),
            patch("scripts.src.dsp.per_driver_effective_eq",
                  side_effect=AssertionError("legacy EQ preview replay")),
            patch.object(native_dataset_adapter, "produce",
                         wraps=native_dataset_adapter.produce) as producer,
        ):
            payload = build_report_payload(data)
        request = producer.call_args.args[0]
        self.assertEqual(request["operation"], "report_dataset")
        self.assertTrue(request["data"]["replay"])
        return payload

    def test_available_prepared_curve_bypasses_python_chain_replay(self):
        data = self._report_data()
        data.native_dsp_replay["drivers"]["LFE"][0]["state"]["curve"]["spl"] = [72.0] * 4
        payload = self._build_without_python_replay(data)
        figures = [
            section["figure"]
            for section in payload["sections"]
            if section.get("kind") == "figure" and section.get("tab") == "Left Sub"
        ]
        response = next(figure for figure in figures if figure["title"].endswith("(Full Range)"))
        corrected = next(series for series in response["series"] if series["name"] == "After EQ")
        self.assertEqual(corrected["y"], [72.0] * 4)

    def test_unavailable_prepared_curve_is_visible_and_never_falls_back(self):
        data = self._report_data(unavailable=True)
        data.native_dsp_replay["routed_sources"] = {
            "L": {"status": "unavailable", "reason": "route measurement missing"}
        }
        payload = self._build_without_python_replay(data)
        encoded = json.dumps(payload, ensure_ascii=False)
        self.assertIn("Raw measurement only; corrected response unavailable", encoded)
        self.assertIn("frozen FIR resource unavailable", encoded)
        self.assertIn("Corrected routed response unavailable", encoded)
        self.assertNotIn("Python DSP", encoded)

    def test_native_overview_does_not_replay_per_driver_effective_eq(self):
        import scripts.src.figures as figures
        from scripts.src.figures import create_combined_figure

        data = two_sub_overview_data()
        for index, driver in enumerate(data["channels"]["LFE"]["drivers"]):
            driver["index"] = index
        native = RoomEqData(data, ".")
        native.native_dsp_replay_present = True
        native.native_playback_sample_rate = 48000
        native.native_dsp_replay = {
            "sample_rate_hz": 48000,
            "drivers": {},
            "routed_sources": {
                name: {"status": "available", "curve": copy.deepcopy(channel["final_curve"])}
                for name, channel in data["channels"].items()
                if channel.get("final_curve")
            },
        }
        with patch.object(
            figures, "per_driver_effective_eq", side_effect=AssertionError("Python DSP replay")
        ):
            panels = create_combined_figure(native)
        self.assertEqual(len(panels), 3)


if __name__ == "__main__":
    unittest.main()
