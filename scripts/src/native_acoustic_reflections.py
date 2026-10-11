"""Thin bridge to the canonical Rust early-reflection report producer."""

import json
import math

from .native_report import produce


def reflection_report(channel, label, tab=None):
    """Return Rust-produced reflection metrics and report sections."""
    result = produce({
        "operation": "acoustic_reflections",
        "channel": _json_safe(channel),
        "label": label,
        "tab": tab,
    })
    return json.loads(result)


def _json_safe(value):
    """Map non-finite measurements to null so Rust validation can reject them."""
    if isinstance(value, float) and not math.isfinite(value):
        return None
    if isinstance(value, dict):
        return {key: _json_safe(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_json_safe(item) for item in value]
    if isinstance(value, tuple):
        return [_json_safe(item) for item in value]
    return value
