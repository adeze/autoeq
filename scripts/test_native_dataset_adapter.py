"""Contract tests for Python-to-shared dataset composition."""

import json
from pathlib import Path
import unittest
from unittest.mock import patch

from scripts.src import report
from scripts.src.native_report import using_producer
from scripts.src.loaders import RoomEqData
from scripts.src.native_dataset_adapter import _binding_option, prepare_dataset_input
from scripts.src.payload_binding import ALGORITHM, payload_digest


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


class BindingOptionTests(unittest.TestCase):
    def _sealed_fixture(self):
        identity = "test-graph-identity"
        slim = {"channels": {"L": {"gain_db": -1.0}}, "metadata": {}, "global_plugins": []}
        acceptance_payload = {"channels": {"L": {"source_id": "L"}}}
        return RoomEqData(
            {
                **slim,
                "correction_decisions": {
                    "ledger_version": "1.0.0",
                    "payload_binding": {
                        "algorithm": ALGORITHM,
                        "graph_identity": identity,
                        "sha256": payload_digest(dict(slim), identity),
                    },
                    "decisions": [],
                    "acceptance_evidence": {
                        "version": "acceptance-evidence-v1",
                        "payload": acceptance_payload,
                        "binding": {
                            "algorithm": ALGORITHM,
                            "graph_identity": identity,
                            "sha256": payload_digest(dict(acceptance_payload), identity),
                        },
                    },
                },
            },
            Path("."),
        )

    def _option(self, data):
        provenance = getattr(data, "native_report_provenance", None) or {}
        return _binding_option(data, provenance)

    def test_sealed_binding_verifies_with_acceptance(self):
        data = self._sealed_fixture()
        option = self._option(data)
        self.assertTrue(option["verified"], option["reason"])
        self.assertIn("verified", option["reason"])
        self.assertEqual(
            option["slim_graph_sha256"], data.native_report_provenance["slim_graph_sha256"]
        )
        self.assertTrue(option["acceptance_evidence_verified"], option["acceptance_evidence_reason"])

    def test_post_seal_payload_tamper_is_rejected(self):
        data = self._sealed_fixture()
        data["channels"]["L"]["gain_db"] = 99.0
        option = self._option(data)
        self.assertFalse(option["verified"])
        self.assertIn("Delivered payload changed", option["reason"])

    def test_resealed_ledger_after_mutation_still_flags_load_skew(self):
        data = self._sealed_fixture()
        data["channels"]["L"]["gain_db"] = 99.0
        identity = data["correction_decisions"]["payload_binding"]["graph_identity"]
        slim = {key: value for key, value in data.items() if key != "correction_decisions"}
        data["correction_decisions"]["payload_binding"]["sha256"] = payload_digest(slim, identity)
        option = self._option(data)
        self.assertFalse(option["verified"])
        self.assertIn("changed after load", option["reason"])

    def test_unsealed_legacy_input_stays_unverified(self):
        data = dict(self._sealed_fixture())
        option = self._option(data)
        self.assertFalse(option["verified"])
        self.assertIn("no frozen slim-graph identity", option["reason"])

    def test_acceptance_tamper_rejects_evidence_only(self):
        data = self._sealed_fixture()
        evidence = data["correction_decisions"]["acceptance_evidence"]
        evidence["payload"]["channels"]["L"]["injected"] = True
        option = self._option(data)
        self.assertTrue(option["verified"], option["reason"])
        self.assertFalse(option["acceptance_evidence_verified"])
        self.assertIn("acceptance-view payload or graph binding changed", option["acceptance_evidence_reason"])

    def test_missing_acceptance_evidence_stays_precise(self):
        data = self._sealed_fixture()
        del data["correction_decisions"]["acceptance_evidence"]
        option = self._option(data)
        self.assertTrue(option["verified"], option["reason"])
        self.assertFalse(option["acceptance_evidence_verified"])
        self.assertIn("no production acceptance views were recorded", option["acceptance_evidence_reason"])


if __name__ == "__main__":
    unittest.main()
