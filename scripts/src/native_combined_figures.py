"""Thin adapter for the canonical combined overview figure constructor."""

from __future__ import annotations

import json

from .native_acoustic_t60 import _json_safe
from .native_report import produce


def combined_figures(prepared, tab=None):
    """Build the three combined overview sections from prepared display traces."""
    return json.loads(
        produce(
            {
                "operation": "combined_figures",
                "prepared": _json_safe(prepared or {}),
                "tab": tab,
            }
        )
    )
