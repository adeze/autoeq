"""Contract tests for Python-to-shared dataset composition."""

import json
from pathlib import Path
import unittest
from unittest.mock import patch

from scripts.src import report
from scripts.src.native_report import using_producer
from scripts.src.loaders import RoomEqData
from scripts.src.native_dataset_adapter import prepare_dataset_input


class NativeDatasetAdapterTests(unittest.TestCase):
    def test_single_dataset_delegates_sections_and_layout_to_report_dataset(self):
        requests = []

        def producer(request):
            requests.append(request)
            self.assertEqual(request["operation"], "report_dataset")
            return json.dumps({"payload": {
                "schema": "autoeq-report-data-v1",
                "title": "shared payload",
                "sections": [],
            }, "preparation_status": []})

        with (
            using_producer(producer),
            patch.object(report, "_report_provenance", return_value={
                "roomeq_version": "test", "generated_at": "fixed",
            }),
            patch.object(report, "_report_assembly", side_effect=AssertionError("legacy assembly")),
        ):
            payload = report.build_report_payload(
                {"channels": {}, "metadata": {}},
                shared_sections={"crossover_config_html": "ignored legacy snapshot"},
            )

        self.assertEqual(payload["title"], "shared payload")
        self.assertEqual(len(requests), 1)
        request = requests[0]
        self.assertEqual(request["data"]["schema_version"], 1)
        self.assertEqual(request["data"]["graph"], {"channels": {}, "metadata": {}})
        self.assertEqual(request["data"]["target_snapshot"]["state"], "not_configured")
        self.assertNotIn("ignored legacy snapshot", json.dumps(request))

    def test_markerless_phase_sum_is_prepared_by_existing_dsp_owner(self):
        requests = []

        def producer(request):
            requests.append(request)
            if request["operation"] == "symmetric_groups":
                return json.dumps([[ ["L+R", ["L", "R"]] ], []])
            raise AssertionError(f"unexpected operation {request['operation']}")

        data = RoomEqData(
            {
                "channels": {
                    "L": {"final_curve": {"freq": [100.0, 1000.0], "spl": [80.0, 80.0], "phase": [0.0, 0.0]}},
                    "R": {"final_curve": {"freq": [100.0, 1000.0], "spl": [80.0, 80.0], "phase": [0.0, 0.0]}},
                },
                "metadata": {},
            },
            Path("."),
        )
        data.symmetric_pairs = {
            "L+R": {"freq": [100.0, 1000.0], "sum_spl": [86.0, 86.0], "diff_spl": [None, None]}
        }

        with using_producer(producer):
            prepared, _, _ = prepare_dataset_input(data)

        expected = [86.02059991327963, 86.02059991327963]
        for actual, target in zip(prepared["symmetric_pairs"]["L+R"]["complex_spl"], expected):
            self.assertAlmostEqual(actual, target, places=12)
        self.assertEqual([request["operation"] for request in requests], ["symmetric_groups"])

    def test_native_phase_sum_does_not_fall_back_to_python(self):
        data = RoomEqData(
            {
                "channels": {
                    "L": {"final_curve": {"freq": [100.0, 1000.0], "spl": [80.0, 80.0], "phase": [0.0, 0.0]}},
                    "R": {"final_curve": {"freq": [100.0, 1000.0], "spl": [80.0, 80.0], "phase": [0.0, 0.0]}},
                },
                "metadata": {},
            },
            Path("."),
        )
        data.symmetric_pairs = {
            "L+R": {"freq": [100.0, 1000.0], "sum_spl": [86.0, 86.0], "diff_spl": [None, None]}
        }
        data.native_dsp_replay_present = True
        data.native_dsp_replay = {"sample_rate_hz": None}

        with patch(
            "scripts.src.dsp.symmetric_complex_sum",
            side_effect=AssertionError("native input must not use Python phase summation"),
        ):
            prepared, _, _ = prepare_dataset_input(data)

        self.assertNotIn("complex_spl", prepared["symmetric_pairs"]["L+R"])


if __name__ == "__main__":
    unittest.main()
