import datetime
import json
import unittest
from pathlib import Path
from unittest.mock import patch

from scripts.src.native_report_provenance import report_provenance
from scripts.src.report import _report_provenance


class FrozenDatetime(datetime.datetime):
    @classmethod
    def now(cls, tz=None):
        return cls(2026, 10, 11, 12, 34, 56, tzinfo=tz)


class NativeReportProvenanceTests(unittest.TestCase):
    def test_pre_port_python_goldens_through_producer(self):
        fixture = json.loads(
            (Path(__file__).parent / "tests/fixtures/report_provenance_python_goldens.json")
            .read_text(encoding="utf-8")
        )
        for case in fixture["cases"]:
            with self.subTest(case=case["name"]):
                self.assertEqual(
                    report_provenance(case["prepared"]),
                    case["expected"],
                )

    def test_report_helper_keeps_python_collection_and_clock_boundaries(self):
        datasets = [
            ("one", {"metadata": {"producer_version": " 1.2.0 ", "timestamp": " t1 "}}),
            ("two", {"metadata": {"roomeq_version": "2.0.0", "timestamp": "t2"}}),
            ("three", {"metadata": {"tool_version": "1.2.0", "timestamp": "t1"}}),
        ]
        with patch("scripts.src.report.datetime.datetime", FrozenDatetime):
            self.assertEqual(
                _report_provenance(datasets),
                {
                    "roomeq_version": "1.2.0 / 2.0.0",
                    "data_timestamp": "t1 / t2",
                    "generated_at": "2026-10-11T12:34:56Z",
                },
            )

    def test_report_helper_preserves_legacy_raw_timestamp_dedup_order(self):
        datasets = [
            ("plain", {"metadata": {"producer_version": "1.0", "timestamp": "t1"}}),
            ("padded", {"metadata": {"producer_version": "1.0", "timestamp": " t1 "}}),
        ]
        with patch("scripts.src.report.datetime.datetime", FrozenDatetime):
            self.assertEqual(
                _report_provenance(datasets),
                {
                    "roomeq_version": "1.0",
                    "data_timestamp": "t1 / t1",
                    "generated_at": "2026-10-11T12:34:56Z",
                },
            )


if __name__ == "__main__":
    unittest.main()
