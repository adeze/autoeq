"""Parity tests for the Rust-canonical loaded target overlay builder."""

import json
import math
from pathlib import Path
import unittest
from unittest.mock import patch

from scripts.src.native_target_overlay import (
    _route_crossover_db,
    build_target_overlay_curves,
)


FIXTURE = Path(__file__).parent / "tests/fixtures/target_overlay_python_goldens.json"


def assert_json_close(test: unittest.TestCase, actual, expected, path="$"):
    if isinstance(expected, dict):
        test.assertIsInstance(actual, dict, path)
        test.assertEqual(set(actual), set(expected), path)
        for key, value in expected.items():
            assert_json_close(test, actual[key], value, f"{path}.{key}")
    elif isinstance(expected, list):
        test.assertIsInstance(actual, list, path)
        test.assertEqual(len(actual), len(expected), path)
        for index, (actual_item, expected_item) in enumerate(zip(actual, expected)):
            assert_json_close(test, actual_item, expected_item, f"{path}[{index}]")
    elif isinstance(expected, (float, int)) and not isinstance(expected, bool):
        test.assertIsInstance(actual, (float, int), path)
        test.assertTrue(math.isclose(actual, expected, rel_tol=1e-10, abs_tol=1e-9),
                        f"{path}: {actual!r} != {expected!r}")
    else:
        test.assertEqual(actual, expected, path)


class NativeTargetOverlayTests(unittest.TestCase):
    def test_matches_pre_port_python_goldens(self):
        cases = json.loads(FIXTURE.read_text(encoding="utf-8"))
        for case in cases:
            with self.subTest(case=case["name"]):
                actual = build_target_overlay_curves(
                    case["data"], case["reference_curves"], case["loaded_target"]
                )
                assert_json_close(self, actual, case["expected"])

    def test_does_not_evaluate_unused_or_absolute_crossover_routes(self):
        data = {
            "channels": {"LFE": {"target_curve": {"freq": [20.0], "spl": [80.0]}}},
            "metadata": {
                "bass_management": {
                    "routing_graph": {
                        "routes": [{
                            "source_channel": "LFE",
                            "route_kind": "lfe_lowpass_to_sub",
                            "crossover_type": "unsupported",
                            "low_pass_hz": 120.0,
                        }]
                    }
                }
            },
        }
        references = {"LFE": {"freq": [20.0], "spl": [70.0]}}
        with patch("scripts.src.native_target_overlay._crossover_response", side_effect=AssertionError):
            self.assertEqual(_route_crossover_db(data, references, None), {})
            self.assertEqual(_route_crossover_db(data, references, {"freq": [20.0]}), {})


if __name__ == "__main__":
    unittest.main()
