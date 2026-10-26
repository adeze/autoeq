"""Parity tests for the canonical target-CSV parser adapter."""

from __future__ import annotations

import contextlib
import csv
import io
import json
from pathlib import Path
import tempfile
import unittest

from scripts.src.native_target_csv import parse_target_csv
from scripts.src.target_overlay import load_target_shape


GOLDENS = (
    Path(__file__).parent
    / "../crates/autoeq-report-wasm/tests/fixtures/target_csv_python_goldens.json"
).resolve()


class NativeTargetCsvTests(unittest.TestCase):
    @staticmethod
    def _csv_text(case):
        if "csv" in case:
            return case["csv"]
        return (
            case.get("csv_prefix", "")
            + case.get("repeat_char", "") * case.get("repeat_count", 0)
            + case.get("csv_suffix", "")
        )

    def test_matches_pre_port_python_goldens(self):
        for case in json.loads(GOLDENS.read_text(encoding="utf-8")):
            with self.subTest(case=case["name"]):
                csv_text = self._csv_text(case)
                if "target" in case:
                    self.assertEqual(parse_target_csv(csv_text), case["target"])
                else:
                    error_type = csv.Error if case["error"]["kind"] == "csv" else ValueError
                    with self.assertRaises(error_type) as raised:
                        parse_target_csv(csv_text)
                    self.assertEqual(str(raised.exception), case["error"]["message"])

    def test_path_loader_preserves_warning_for_bad_numeric_values(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "target.csv"
            path.write_text("frequency,spl\ninvalid,70\n", encoding="utf-8")
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                target = load_target_shape(
                    {"metadata": {"effective_config": {"target_curve": str(path)}}}
                )
        self.assertIsNone(target)
        self.assertTrue(
            stdout.getvalue().rstrip().endswith(
                ": could not convert string to float: 'invalid'"
            )
        )

    def test_path_loader_bounds_file_reads_before_native_transport(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "target.csv"
            path.write_bytes(b"frequency,spl\n" + b"100,80\n" * 1_500_000)
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                target = load_target_shape(
                    {"metadata": {"effective_config": {"target_curve": str(path)}}}
                )
        self.assertIsNone(target)
        self.assertTrue(
            stdout.getvalue().rstrip().endswith(": target CSV exceeds the 8 MiB size limit")
        )

    def test_leading_blank_line_is_kept_as_the_legacy_header(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "target.csv"
            path.write_text("\nfrequency,spl\n100,80\n", encoding="utf-8")
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                target = load_target_shape(
                    {"metadata": {"effective_config": {"target_curve": str(path)}}}
                )
        self.assertIsNone(target)
        self.assertTrue(
            stdout.getvalue().rstrip().endswith(
                ": target CSV needs frequency/freq and spl/spl_db columns"
            )
        )

    def test_field_limit_precedes_numeric_conversion_and_counts_characters(self):
        csv_text = "frequency,spl,note\ninvalid,70," + "é" * 131_073 + "\n"
        with self.assertRaises(csv.Error) as raised:
            parse_target_csv(csv_text)
        self.assertEqual(
            str(raised.exception), "field larger than field limit (131072)"
        )

        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "target.csv"
            path.write_text(csv_text, encoding="utf-8")
            with self.assertRaises(csv.Error) as loader_error:
                load_target_shape(
                    {"metadata": {"effective_config": {"target_curve": str(path)}}}
                )
        self.assertEqual(str(loader_error.exception), str(raised.exception))


if __name__ == "__main__":
    unittest.main()
