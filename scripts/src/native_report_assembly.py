"""Thin adapter for canonical prepared-section dashboard assembly."""

import json

from .native_acoustic_t60 import _json_safe
from .native_report import produce


def report_assembly(title, status_sections, global_groups, channel_blocks, provenance,
                    relative_levels_html=None, modes_html=None):
    """Order prepared report sections through the shared Rust producer."""
    return json.loads(produce(_json_safe({
        "operation": "report_assembly",
        "title": title,
        "status_sections": status_sections,
        "global_groups": global_groups,
        "channel_blocks": channel_blocks,
        "provenance": provenance,
        "relative_levels_html": relative_levels_html,
        "modes_html": modes_html,
    })))
