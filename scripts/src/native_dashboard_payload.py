"""Thin adapter for canonical dashboard grouping and document assembly."""

import json

from .native_acoustic_t60 import _json_safe
from .native_report import produce


def dashboard_payload(title, status_sections, groups, provenance,
                      relative_levels_html=None, modes_html=None):
    return json.loads(produce(_json_safe({
        "operation": "dashboard_payload", "title": title,
        "status_sections": status_sections, "groups": groups,
        "provenance": provenance, "relative_levels_html": relative_levels_html,
        "modes_html": modes_html,
    })))
