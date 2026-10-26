"""Parity tests for canonical report channel and driver tab entries."""

import json
from pathlib import Path
import unittest

from scripts.src.data_extract import display_channel_entries, driver_display_entries


FIXTURE = (
    Path(__file__).resolve().parents[1]
    / "crates/autoeq-report-wasm/tests/fixtures/channel_display_entries_python.json"
)


class NativeChannelDisplayEntryTests(unittest.TestCase):
    def test_matches_captured_pre_port_python_goldens(self):
        cases = json.loads(FIXTURE.read_text(encoding="utf-8"))
        for case in cases:
            with self.subTest(case=case["name"]):
                self.assertEqual(display_channel_entries(case["data"]), case["expected"])

    def test_sparse_source_indices_do_not_replace_filtered_display_positions(self):
        cases = json.loads(FIXTURE.read_text(encoding="utf-8"))
        case = next(case for case in cases if case["name"] == "sparse_driver_display_ordinals")

        entries = display_channel_entries(case["data"])

        self.assertEqual([entry["label"] for entry in entries], ["Front Sub", "42"])
        self.assertEqual([entry["driver"] for entry in entries], [0, 1])

    def test_driver_identity_keeps_raw_position_and_index_separate(self):
        cases = json.loads(FIXTURE.read_text(encoding="utf-8"))
        for case in cases:
            with self.subTest(case=case["name"]):
                self.assertEqual(
                    driver_display_entries(case["data"]),
                    case["driver_identity_expected"],
                )


if __name__ == "__main__":
    unittest.main()
