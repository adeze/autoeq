"""Thin JSON bridge to the canonical native/WASM report producer."""

import json
import os
from pathlib import Path
import shutil
import subprocess


def produce(request):
    """Run the shared Rust producer; never duplicate its calculations here.

    AUTOEQ_REPORT_PRODUCER can select a prebuilt report-producer executable.
    Source checkouts otherwise use Cargo's cached native build.
    """
    executable = os.environ.get("AUTOEQ_REPORT_PRODUCER") or shutil.which("report-producer")
    if executable:
        command = [executable]
    else:
        root = Path(__file__).resolve().parents[2]
        cargo = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo")
        command = [cargo, "run", "--quiet", "--manifest-path", str(root / "Cargo.toml"),
                   "-p", "autoeq-report-wasm", "--bin", "report-producer", "--"]
    result = subprocess.run(command, input=json.dumps(request, ensure_ascii=False, allow_nan=False),
                            text=True, capture_output=True, check=False, timeout=120)
    if result.returncode:
        raise RuntimeError("Shared AutoEQ report producer failed: " + result.stderr.strip())
    return result.stdout
