"""Parity checks for measured early-reflection report sections."""

import json
from pathlib import Path
import unittest

from scripts.src.native_acoustic_reflections import reflection_report


class AcousticReflectionsTests(unittest.TestCase):
    def test_matches_pre_port_python_goldens(self):
        fixture_dir = Path(__file__).parent / "tests" / "fixtures"
        for name in ("valid", "no_candidates", "invalid_geometry", "missing_evidence"):
            with self.subTest(name=name):
                fixture = json.loads(
                    (fixture_dir / f"acoustic_reflections_{name}.json").read_text()
                )
                actual = reflection_report(
                    fixture["channel"], fixture["label"], fixture["tab"]
                )
                self.assertEqual(actual, fixture["expected"])

    def test_invalid_geometry_and_basis_never_produce_reflection_figures(self):
        fixture_dir = Path(__file__).parent / "tests" / "fixtures"
        valid = json.loads((fixture_dir / "acoustic_reflections_valid.json").read_text())
        channel = valid["channel"]
        channel["early_reflections"]["basis"] = "reconstructed_transfer_ir"
        result = reflection_report(channel, "<unsafe>")
        self.assertIsNone(result["validated_report"])
        self.assertIsNone(result["early_reflection_level_db"])
        self.assertEqual(result["figures"], [])
        self.assertEqual(
            result["html"],
            '<p class="epa-footer">1–8 kHz early reflections: pending roomeq field early_reflections from a measured room IR.</p>',
        )

    def test_boolean_and_nonfinite_event_values_are_rejected(self):
        fixture_dir = Path(__file__).parent / "tests" / "fixtures"
        fixture = json.loads((fixture_dir / "acoustic_reflections_valid.json").read_text())
        for invalid in (True, float("nan"), float("inf"), float("-inf")):
            with self.subTest(invalid=invalid):
                channel = json.loads(json.dumps(fixture["channel"]))
                channel["early_reflections"]["post"][0]["gain_dbfs"] = invalid
                result = reflection_report(channel, fixture["label"], fixture["tab"])
                self.assertIsNone(result["validated_report"])
                self.assertIsNone(result["early_reflection_level_db"])
                self.assertEqual(result["figures"], [])
                self.assertIn("pending roomeq field early_reflections", result["html"])

    def test_nonfinite_early_late_curve_is_rejected_without_hiding_reflections(self):
        fixture_dir = Path(__file__).parent / "tests" / "fixtures"
        fixture = json.loads((fixture_dir / "acoustic_reflections_valid.json").read_text())
        channel = fixture["channel"]
        channel["early_late_curves"]["early"]["spl"][1] = float("nan")
        result = reflection_report(channel, fixture["label"], fixture["tab"])
        self.assertIsNotNone(result["validated_report"])
        self.assertIsNone(result["early_late_ratio_db"])
        self.assertIn("needs roomeq field: early_late_curves", result["early_late_ratio_cell"])


if __name__ == "__main__":
    unittest.main()
