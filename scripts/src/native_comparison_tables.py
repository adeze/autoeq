"""Thin adapter for canonical comparison-table presentation."""

from __future__ import annotations

import json

from .native_acoustic_t60 import _json_safe
from .native_report import produce


def epa_comparison_tables(channels):
    """Render every prepared per-channel EPA comparison table in one call."""
    return json.loads(produce({
        "operation": "epa_comparison_tables",
        "prepared": _json_safe({"channels": channels}),
    }))


def score_comparison_figure(scores, loss_types=None, tab=None):
    """Build the comparison score chart from prepared labels and values."""
    return json.loads(produce({
        "operation": "comparison_score_figure",
        "prepared": _json_safe({
            "scores": scores,
            "loss_types": loss_types or [],
            "tab": tab,
        }),
    }))
