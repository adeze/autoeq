"""Verify standalone adapters use the same canonical Rust report API."""

import json
import tempfile
import unittest
from pathlib import Path

from scripts.src.acoustic_report import tof_html, tof_table
from scripts.src.wasm_report import assemble_html


class NativeReportTests(unittest.TestCase):
    def test_timing_missing_evidence_and_order_survive_the_bridge(self):
        metadata = {"timing_diagnostics": {"channels": [
            {"name": "Sub & rear", "measured_arrival_ms": 8.12549,
             "applied_delay_ms": -4.5},
            {"name": "Front", "final_arrival_ms": 3.25}, None]}}
        rows = tof_table(metadata)
        self.assertEqual([row["name"] for row in rows], ["Sub & rear", "Front"])
        self.assertIsNone(rows[0]["after_ms"])
        html = tof_html(metadata)
        self.assertIn("Sub &amp; rear</td><td>8.125</td><td>-4.500</td><td>n/a", html)
        self.assertIn("not a post-playback capture", html)

    def test_shell_adapter_preserves_payload_and_template_shaped_values(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "report2d.wasm").write_bytes(b"wasm")
            (path / "report2d.js").write_bytes(b"glue")
            template = path / "shell.html"
            template.write_text('<title>{{PAGE_TITLE}}</title><script>{{PAYLOAD_JSON}}</script>{{WASM_2D_B64}}{{GLUE_2D_B64}}')
            payload = {"label": "</script>&{{PAGE_TITLE}}"}
            html = assemble_html("{{PAYLOAD_JSON}}", payload, path, template)
            self.assertIn("<title>{{PAYLOAD_JSON}}</title>", html)
            self.assertEqual(html.count("</script>"), 1)
            embedded = html.split("<script>", 1)[1].split("</script>", 1)[0]
            self.assertEqual(json.loads(embedded), payload)

    def test_shell_adapter_reports_missing_bundle_with_rebuild_recipe(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            template = path / "shell.html"
            template.write_text("<title>t</title>")
            with self.assertRaises(FileNotFoundError) as ctx:
                assemble_html("t", {}, path, template)
            message = str(ctx.exception)
            self.assertIn("report2d.wasm", message)
            self.assertIn("just report-dist", message)

    def test_shell_adapter_names_gpui_recipe_for_missing_gpui_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "report2d.wasm").write_bytes(b"wasm")
            (path / "report2d.js").write_bytes(b"glue")
            template = path / "shell.html"
            template.write_text("<title>t</title>{{WASM_GPUI_B64}}")
            with self.assertRaises(FileNotFoundError) as ctx:
                assemble_html("t", {}, path, template)
            message = str(ctx.exception)
            self.assertIn("reportgpui.wasm", message)
            self.assertIn("just report-dist-gpui", message)
