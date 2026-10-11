"""Parity checks against pre-port Python bass figure outputs."""

import json
import math
from pathlib import Path
import unittest

from scripts.src.native_bass_figures import bass_figure


FIXTURE = Path(__file__).parent / "tests/fixtures/bass_figures_python_goldens.json"


def assert_json_close(test, actual, expected, path="$"):
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
        test.assertTrue(
            math.isclose(actual, expected, rel_tol=1e-12, abs_tol=1e-12),
            f"{path}: {actual!r} != {expected!r}",
        )
    else:
        test.assertEqual(actual, expected, path)


class NativeBassFigureTests(unittest.TestCase):
    def test_routing_and_headroom_builders_match_python_goldens(self):
        cases = json.loads(FIXTURE.read_text(encoding="utf-8"))
        for case in cases:
            with self.subTest(case=case["name"]):
                actual = bass_figure(case["builder"], case["data"], case["tab"])
                assert_json_close(self, actual, case["expected"])


if __name__ == "__main__":
    unittest.main()
