"""Thin adapters for canonical Rust bass-management figure builders."""

from __future__ import annotations

import json

from .native_acoustic_t60 import _json_safe
from .native_report import produce


def bass_figure(builder, data, tab=None):
    """Build one routing or headroom figure through the shared producer."""
    return json.loads(
        produce(
            {
                "operation": "bass_figure",
                "builder": builder,
                "data": _json_safe(data or {}),
                "tab": tab,
            }
        )
    )


def routing_figure(data, tab=None):
    """Build the renderer-neutral saved bass routing graph."""
    return bass_figure("routing", data, tab)


def headroom_figure(data, tab=None):
    """Build the renderer-neutral per-output headroom chart."""
    return bass_figure("headroom", data, tab)
