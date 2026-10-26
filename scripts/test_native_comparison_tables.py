"""Parity tests for canonical EPA comparison table presentation."""

import json
from pathlib import Path
import unittest

from scripts.src.report import (
    _epa_comparison_entries,
    _epa_comparison_table_html,
    _epa_comparison_tables,
)


FIXTURE = Path(__file__).parent / "tests" / "fixtures" / "epa_comparison_python_goldens.json"


class NativeComparisonTableTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))

    def _datasets(self, name):
        return [tuple(row) for row in self.fixture["cases"][name]]

    def test_pre_port_goldens_for_missing_partial_and_numeric_evidence(self):
        for name, datasets in self.fixture["cases"].items():
            with self.subTest(case=name):
                expected = self.fixture["html"][name]
                self.assertEqual(_epa_comparison_table_html("L", self._datasets(name)), expected)

    def test_batch_operation_matches_per_channel_html(self):
        comparison = self._datasets("comparison")
        partial = self._datasets("partial")
        actual = _epa_comparison_tables({
            "L": _epa_comparison_entries("L", comparison),
            "R": _epa_comparison_entries("L", partial),
        })
        self.assertEqual(actual["L"], self.fixture["html"]["comparison"])
        self.assertEqual(actual["R"], self.fixture["html"]["partial"])

    def test_mode_label_is_escaped(self):
        data = self._datasets("comparison")[:1]
        html = _epa_comparison_table_html("L", [("<img src=x onerror=alert(1)>", data[0][1])])
        self.assertIn("&lt;img src=x onerror=alert(1)&gt; (post)", html)
        self.assertNotIn("<img", html)


if __name__ == "__main__":
    unittest.main()
