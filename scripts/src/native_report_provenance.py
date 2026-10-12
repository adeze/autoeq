"""Thin adapter for canonical report provenance assembly."""

import json

from . import wasm_report


def report_provenance(prepared: dict) -> dict:
    """Assemble prepared provenance fields through the shared Rust producer."""
    return json.loads(wasm_report.produce({
        "operation": "report_provenance",
        "prepared": prepared,
    }))
