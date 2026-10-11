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
    def test_build_report_payload_matches_captured_pre_assembly_golden(self):
        cases = json.loads(FIXTURE.read_text(encoding="utf-8"))
        for index, case in enumerate(cases):
            with self.subTest(case=index):
                with patch.object(
                    report,
                    "_report_provenance",
                    return_value=case["expected"].get("provenance", {}),
                ):
                    actual = report.build_report_payload(
                        case["source_data"],
                        capture_verification=case["options"]["capture_verification"],
                    )
                self.assertEqual(actual, case["expected"])


if __name__ == "__main__":
    unittest.main()
