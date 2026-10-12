import unittest
from unittest.mock import patch

from scripts.src.report import (
    _bass_management_groups_table_html,
    _bass_management_sub_outputs_table_html,
    _bass_management_summary_html,
    _gain_plugins_html,
    build_report_payload,
)


class SharedReportSectionAdapterTests(unittest.TestCase):
    def test_legacy_python_helpers_are_thin_shared_rust_adapters(self):
        gain_data = {
            "global_plugins": [{
                "plugin_type": "gain",
                "parameters": {"gain_db": -2.25, "label": "global"},
            }],
            "channels": {},
        }
        self.assertIn("-2.250 dB", _gain_plugins_html(gain_data))

        bass = {
            "enabled": True,
            "groups": [{"group_id": "L/R", "roles": ["L", "R"]}],
            "sub_outputs": [{"output_role": "Sub A", "gain_db": -1.0}],
        }
        self.assertIn("Bass Management", _bass_management_summary_html(bass))
        self.assertIn("<td>L/R</td>", _bass_management_groups_table_html(bass))
        self.assertIn("<td>Sub A</td>", _bass_management_sub_outputs_table_html(bass))

    def test_present_empty_shared_values_suppress_standalone_generation(self):
        shared = {
            "gain_summary_html": "",
            "crossover_config_html": "",
            "time_of_flight_html": "",
            "bass_management_summary_html": "",
            "bass_management_groups_table_html": "",
            "bass_management_sub_outputs_table_html": "",
            "signal_flow_sections": [],
        }
        with patch(
            "scripts.src.report._rust_shared_sections",
            side_effect=AssertionError("provided empty values must override generation"),
        ):
            payload = build_report_payload(
                {"channels": {}, "metadata": {}},
                shared_sections=shared,
            )
        self.assertEqual(payload["schema"], "autoeq-report-data-v1")


if __name__ == "__main__":
    unittest.main()
