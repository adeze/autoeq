"""Thin adapter for native impulse-response group-delay calculations."""

import json
import math

from .native_report import produce


def _json_safe(value):
    """Replace non-finite samples with null before crossing the JSON boundary."""
    if isinstance(value, float) and not math.isfinite(value):
        return None
    if isinstance(value, dict):
        return {key: _json_safe(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_json_safe(item) for item in value]
    return value


def compute_group_delay_from_ir(
    ir,
    min_freq=20.0,
    max_freq=20_000.0,
    point_count=512,
):
    """Return the native group's valid frequency and delay arrays as a tuple."""
    if not ir:
        return [], []
    request = {
        "operation": "ir_group_delay",
        "time_ms": _json_safe(ir.get("time_ms", [])),
        "amplitude": _json_safe(ir.get("amplitude", [])),
        "min_freq": min_freq,
        "max_freq": max_freq,
        "point_count": int(point_count),
    }
    result = json.loads(produce(_json_safe(request)))
    return result["frequency_hz"], result["group_delay_ms"]
