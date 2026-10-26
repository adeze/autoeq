"""Thin adapter for the canonical Rust/WASM signal-flow report producer."""

import json

from .native_report import produce


def signal_flow_sections(data):
    """Return Rust-produced signal-flow report sections for saved RoomEQ data."""
    result = produce({"operation": "signal_flow_sections", "data": data})
    return json.loads(result)
