"""Adapter tests for canonical operational summary report operations."""

import json
import math
import unittest
from unittest.mock import patch

from scripts.src import native_acoustic_summary


class NativeAcousticSummaryAdapterTests(unittest.TestCase):
    def test_summary_operations_decode_shared_json_results(self):
        requests = []

        def produce(request):
            requests.append(request)
            if request["operation"] == "acoustic_summary_report":
                return json.dumps({"summary_table_html": "table"})
            if request["operation"] == "operational_summaries_by_channel":
                return json.dumps({"L": [50.0, 2, 1]})
            return json.dumps("<td>shared</td>")

        with patch.object(native_acoustic_summary, "produce", side_effect=produce):
            self.assertEqual(native_acoustic_summary.operational_summary_report({"channels": {}}),
                             {"summary_table_html": "table"})
            self.assertEqual(native_acoustic_summary.operational_summaries_by_channel({"data": 1}),
                             {"L": (50.0, 2, 1)})
            self.assertEqual(native_acoustic_summary.operational_response_cell({"L": (50.0, 2, 1)}, "L"),
                             "<td>shared</td>")
            self.assertEqual(native_acoustic_summary.summary_table_html({"channels": {}}), "<td>shared</td>")
            self.assertEqual(native_acoustic_summary.pending_cell("missing"), "<td>shared</td>")

        self.assertEqual(
            [request["operation"] for request in requests],
            ["acoustic_summary_report", "operational_summaries_by_channel",
             "operational_response_cell", "summary_table_html", "pending_cell"],
        )
        self.assertEqual(requests[2]["summaries_by_channel"]["L"], [50.0, 2, 1])

    def test_nonfinite_inputs_are_null_normalized_for_strict_json_transport(self):
        requests = []

        def produce(request):
            requests.append(request)
            return "{}"

        with patch.object(native_acoustic_summary, "produce", side_effect=produce):
            native_acoustic_summary.operational_summary_report({
                "correction_decisions": {"channel_summaries": [
                    {"channel": "L", "operational_response_pct": math.nan}
                ]}
            })
            native_acoustic_summary.pending_cell("<field>")

        self.assertIsNone(
            requests[0]["data"]["correction_decisions"]["channel_summaries"][0]["operational_response_pct"]
        )
        json.dumps(requests, allow_nan=False)


if __name__ == "__main__":
    unittest.main()
