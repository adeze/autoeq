import json
import math
import unittest
from unittest.mock import patch

from scripts.src import native_acoustic_t60


class NativeAcousticT60AdapterTests(unittest.TestCase):
    def test_report_and_legacy_helpers_delegate_to_shared_operations(self):
        aggregate = {
            "tolerance": {"seconds": 0.05, "declared": False},
            "reference": None,
            "room_rows": None,
            "room_t60_table_html": "",
            "channels": {
                "L": {
                    "rows": None,
                    "flatness_pct": None,
                    "flatness_window_s": None,
                    "itu_pct": None,
                    "t60_table_html": "pending",
                    "flatness_cell_html": "pending-flatness",
                    "itu_cell_html": "pending-itu",
                }
            },
        }
        calls = []

        def produce(request):
            calls.append(request)
            if request["operation"] == "t60_report":
                result = aggregate
                if "_channel" in request["data"].get("channels", {}):
                    result = {**aggregate, "channels": {"_channel": {
                        **aggregate["channels"]["L"], "t60_table_html": "pending"}}}
                return json.dumps(result)
            if request["operation"] == "t60_rows":
                return "null"
            if request["operation"] == "t60_itu_note":
                return json.dumps("note")
            raise AssertionError(request)

        with patch.object(native_acoustic_t60, "produce", side_effect=produce):
            source = {"channels": {"L": {}}}
            self.assertEqual(native_acoustic_t60.t60_report(source), aggregate)
            self.assertEqual(native_acoustic_t60.t60_rows({}), None)
            self.assertEqual(native_acoustic_t60.t60_itu_note(None), "note")
            self.assertEqual(native_acoustic_t60.t60_flatness_tolerance_s(source), (0.05, False))
            self.assertEqual(native_acoustic_t60.t60_flatness_pct(source, "L"), None)
            self.assertEqual(native_acoustic_t60.t60_flatness_window(source), None)
            self.assertEqual(native_acoustic_t60.t60_itu_reference(source), None)
            self.assertEqual(native_acoustic_t60.room_t60_rows(source), None)
            self.assertEqual(native_acoustic_t60.room_t60_table_html(source), "")
            self.assertEqual(native_acoustic_t60.t60_table_html({}), "pending")
            self.assertEqual(native_acoustic_t60.t60_itu_pct(source, "L"), None)
            self.assertEqual(native_acoustic_t60.t60_itu_cell(source, "L"), "pending-itu")
            self.assertEqual(native_acoustic_t60.t60_flatness_cell(source, "L"), "pending-flatness")

        self.assertEqual(calls[0], {"operation": "t60_report", "data": source})
        self.assertEqual(calls[1], {"operation": "t60_rows", "channel": {}})
        self.assertEqual(calls[2], {"operation": "t60_itu_note", "reference": None})

    def test_nonfinite_evidence_is_normalized_before_strict_json_encoding(self):
        calls = []

        def produce(request):
            calls.append(request)
            return "{}" if request["operation"] == "t60_report" else "null"

        with patch.object(native_acoustic_t60, "produce", side_effect=produce):
            native_acoustic_t60.t60_report({"channels": {"L": {"metric": math.nan}}})
            native_acoustic_t60.t60_rows({"t60_octaves": {"min_r2": math.inf}})

        self.assertIsNone(calls[0]["data"]["channels"]["L"]["metric"])
        self.assertIsNone(calls[1]["channel"]["t60_octaves"]["min_r2"])
        json.dumps(calls, allow_nan=False)


if __name__ == "__main__":
    unittest.main()
