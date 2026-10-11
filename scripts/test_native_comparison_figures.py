"""Golden tests for prepared comparison figure payload assembly."""

import json
import math
from pathlib import Path
import unittest

from scripts.src.figures import (
    ComparisonFigureBatch,
    create_comparison_eq_overlay_figure,
    create_comparison_group_delay_figure,
    create_comparison_ir_figure,
    create_comparison_overlay_figure,
    create_comparison_phase_figure,
    create_comparison_zoomed_figure,
    create_mode_subplots_figure,
)


FIXTURE = Path(__file__).parent / "tests" / "fixtures" / "comparison_figures_python_goldens.json"


def _assert_same_figure_structure(test_case, actual, expected, path=()):
    """Compare schemas exactly and allow only JSON parser-scale trace drift."""
    if isinstance(expected, dict):
        test_case.assertIsInstance(actual, dict, path)
        test_case.assertEqual(actual.keys(), expected.keys(), path)
        for key in expected:
            _assert_same_figure_structure(test_case, actual[key], expected[key], path + (key,))
        return
    if isinstance(expected, list):
        test_case.assertIsInstance(actual, list, path)
        test_case.assertEqual(len(actual), len(expected), path)
        for index, (actual_item, expected_item) in enumerate(zip(actual, expected)):
            _assert_same_figure_structure(
                test_case, actual_item, expected_item, path + (index,)
            )
        return
    in_trace_arrays = "series" in path and any(key in ("x", "y") for key in path)
    if (
        in_trace_arrays
        and isinstance(actual, (int, float))
        and not isinstance(actual, bool)
        and isinstance(expected, (int, float))
        and not isinstance(expected, bool)
    ):
        tolerance = max(1e-12, abs(expected) * 1e-15)
        test_case.assertTrue(
            math.isclose(actual, expected, rel_tol=0.0, abs_tol=tolerance),
            f"{path}: {actual!r} differs from {expected!r} by more than {tolerance:g}",
        )
        return
    test_case.assertEqual(actual, expected, path)


class NativeComparisonFigureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
        cls.mode_data = [tuple(entry) for entry in cls.fixture["mode_data"]]
        cls.target = cls.fixture["target"]

    def _build(self):
        return [
            create_comparison_overlay_figure(
                "L", self.mode_data, target_curve=self.target, tab="L"
            ),
            create_comparison_zoomed_figure("L", self.mode_data, tab="L"),
            create_comparison_phase_figure("L", self.mode_data, tab="L"),
            create_comparison_group_delay_figure("L", self.mode_data, tab="L"),
            create_comparison_ir_figure("L", self.mode_data, tab="L"),
            create_mode_subplots_figure(
                "L", self.mode_data, target_curve=self.target, tab="L"
            ),
            create_comparison_eq_overlay_figure("L", self.mode_data, tab="L"),
        ]

    def test_individual_comparison_figures_match_pre_port_goldens(self):
        expected = self.fixture["figures"]
        actual = self._build()
        for key, value in zip(
            ("overlay", "zoom", "phase", "group_delay", "ir", "mode", "eq"), actual
        ):
            with self.subTest(figure=key):
                _assert_same_figure_structure(self, value, expected[key])

    def test_report_batch_matches_the_same_goldens_and_keeps_missing_states(self):
        with ComparisonFigureBatch() as batch:
            pending = self._build()
            missing = [
                create_comparison_phase_figure("Empty", [("unknown", {})], tab="Empty"),
                create_comparison_group_delay_figure("Empty", [("unknown", {})], tab="Empty"),
                create_comparison_ir_figure("Empty", [("unknown", {})], tab="Empty"),
            ]
        resolved = batch.resolve(pending)
        resolved_missing = batch.resolve(missing)
        for key, value in zip(
            ("overlay", "zoom", "phase", "group_delay", "ir", "mode", "eq"), resolved
        ):
            with self.subTest(figure=key):
                _assert_same_figure_structure(self, value, self.fixture["figures"][key])
        self.assertEqual(resolved_missing, [None, None, None])


if __name__ == "__main__":
    unittest.main()
