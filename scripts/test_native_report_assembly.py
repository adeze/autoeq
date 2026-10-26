"""Standard-library tests for native report section assembly."""

import json
import unittest
from pathlib import Path
from unittest.mock import patch

from scripts.src import report


FIXTURE = (
    Path(__file__).parents[1]
    / "crates"
    / "autoeq-report-wasm"
    / "tests"
    / "fixtures"
    / "report_assembly_python_goldens.json"
)


class NativeReportAssemblyTests(unittest.TestCase):
    def test_build_report_payload_delegates_once_to_shared_composer(self):
        from scripts.src import native_dataset_adapter

        cases = json.loads(FIXTURE.read_text(encoding="utf-8"))
        case = cases[0]
        expected = {"title": "shared composer sentinel", "sections": []}
        with patch.object(
            native_dataset_adapter, "build_dataset_payload", return_value=expected
        ) as shared:
            actual = report.build_report_payload(
                case["source_data"],
                capture_verification=case["options"]["capture_verification"],
            )
        self.assertIs(actual, expected)
        shared.assert_called_once()
        self.assertEqual(shared.call_args.args[0], case["source_data"])
        self.assertEqual(
            shared.call_args.kwargs["capture_verification"],
            case["options"]["capture_verification"],
        )


if __name__ == "__main__":
    unittest.main()
