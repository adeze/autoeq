"""Thin adapter for the canonical Rust EQ section HTML builders."""

from __future__ import annotations

import json

from .native_acoustic_t60 import _json_safe
from .native_report import produce


def render_eq_section(builder, prepared):
    """Render an EQ filter line, pass list, table, or aggregate filter section."""
    return json.loads(
        produce(
            {
                "operation": "eq_section_html",
                "builder": builder,
                "prepared": _json_safe(prepared or {}),
            }
        )
    )
