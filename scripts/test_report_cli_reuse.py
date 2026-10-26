"""Smoke checks for the legacy display-roomeq compatibility entry point."""

import importlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / "ui" / "display-roomeq"


class ReportCliReuseTests(unittest.TestCase):
    def _env(self):
        env = os.environ.copy()
        env.pop("PYTHONPATH", None)
        return env

    def _run(self, args, cwd):
        return subprocess.run(
            [sys.executable, "-S", *map(str, args)],
            cwd=cwd,
            env=self._env(),
            text=True,
            capture_output=True,
            timeout=120,
            check=False,
        )

    def test_help_and_direct_module_import_from_external_directory(self):
        with tempfile.TemporaryDirectory(prefix="autoeq-display-cli-") as directory:
            help_result = self._run([CLI, "--help"], directory)
            self.assertEqual(help_result.returncode, 0, help_result.stderr)
            self.assertIn("Display roomeq optimization results", help_result.stdout)

            import_script = (
                "import importlib, importlib.util, pathlib, sys; "
                f"root = pathlib.Path({str(ROOT)!r}); "
                "sys.path.insert(0, str(root / 'ui' / 'display-roomeq')); "
                "names = ('acceptance_views', 'acoustic_report', 'capture_clock_views', "
                "'capture_reflection_views', 'capture_views', 'correction_explanation', "
                "'data_extract', 'dsp', 'figures', 'loaders', 'payload_binding', "
                "'report', 'signal_flow', 'target_overlay', 'wasm_report'); "
                "[(lambda legacy, canonical: (_ for _ in ()).throw(AssertionError(name)) "
                "if legacy is not canonical else None)(importlib.import_module(name), "
                "importlib.import_module('scripts.src.' + name)) for name in names]; "
                "assert hasattr(importlib.import_module('report'), '_emit_fig'); "
                "spec = importlib.util.spec_from_file_location('legacy_direct_report', "
                "root / 'ui' / 'display-roomeq' / 'report.py'); "
                "module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module); "
                "assert module.create_html_report is importlib.import_module('scripts.src.report').create_html_report; "
                "assert sys.modules['legacy_direct_report'] is importlib.import_module('scripts.src.report'); "
                "spec = importlib.util.spec_from_file_location('legacy_display_init', "
                "root / 'ui' / 'display-roomeq' / '__init__.py'); "
                "module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module); "
                "canonical = importlib.import_module('scripts.src'); "
                "assert module.DEFAULT_SMOOTHING == canonical.DEFAULT_SMOOTHING; "
                "assert sys.modules['legacy_display_init'] is canonical"
            )
            imported = self._run(["-c", import_script], directory)
            self.assertEqual(imported.returncode, 0, imported.stderr)

    def test_ir_bearing_full_report_and_comparison_work_without_site_packages(self):
        source = {
            "metadata": {"roomeq_version": "test"},
            "channels": {
                "L": {
                    "initial_curve": {
                        "freq": [20.0, 100.0, 1000.0, 10000.0, 20000.0],
                        "spl": [58.0, 62.0, 60.0, 55.0, 52.0],
                    },
                    "final_curve": {
                        "freq": [20.0, 100.0, 1000.0, 10000.0, 20000.0],
                        "spl": [57.0, 61.0, 60.0, 56.0, 53.0],
                    },
                    "pre_ir": {
                        "time_ms": [0.0, 0.1, 0.2, 0.3],
                        "amplitude": [1.0, 0.35, 0.08, 0.01],
                    },
                    "post_ir": {
                        "time_ms": [0.0, 0.1, 0.2, 0.3],
                        "amplitude": [0.9, 0.25, 0.04, 0.005],
                    },
                }
            },
        }
        with tempfile.TemporaryDirectory(prefix="autoeq-display-report-") as directory:
            work = Path(directory)
            first = work / "iir.json"
            second = work / "fir.json"
            first.write_text(json.dumps(source), encoding="utf-8")
            changed = json.loads(json.dumps(source))
            changed["channels"]["L"]["final_curve"]["spl"] = [56.0, 60.0, 59.0, 55.0, 52.0]
            second.write_text(json.dumps(changed), encoding="utf-8")

            single_html = work / "single.html"
            single = self._run([CLI, first, single_html], work)
            self.assertEqual(single.returncode, 0, single.stderr)
            self.assertTrue(
                "Impulse Response: L" in single_html.read_text(encoding="utf-8"),
                "single-mode HTML omitted its impulse-response figure",
            )

            comparison_html = work / "comparison.html"
            comparison = self._run([CLI, "--compare", first, second, "-o", comparison_html], work)
            self.assertEqual(comparison.returncode, 0, comparison.stderr)
            comparison_text = comparison_html.read_text(encoding="utf-8")
            self.assertTrue("RoomEQ Mode Comparison" in comparison_text,
                            "comparison HTML omitted its report title")
            self.assertTrue("L: Impulse Response Before / After EQ" in comparison_text,
                            "comparison HTML omitted its impulse-response figures")


if __name__ == "__main__":
    unittest.main()
