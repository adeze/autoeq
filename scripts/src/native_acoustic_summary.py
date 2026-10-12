"""Thin adapters for canonical operational and acoustic summary rendering."""

import json
import math

from .native_report import produce


def _json_safe(value):
    """Normalize non-finite measurements before strict JSON transport."""
    if isinstance(value, float) and not math.isfinite(value):
        return None
    if isinstance(value, dict):
        return {key: _json_safe(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_json_safe(item) for item in value]
    return value


def operational_summary_report(data):
    """Return the operational summaries, cells, and summary-table HTML."""
    result = produce({"operation": "acoustic_summary_report", "data": _json_safe(data or {})})
    return json.loads(result)


def operational_summaries_by_channel(data):
    """Return emitted or ledger-derived shares keyed by output channel."""
    result = produce({"operation": "operational_summaries_by_channel", "data": _json_safe(data or {})})
    summaries = json.loads(result)
    return {name: tuple(values) for name, values in summaries.items()}


def operational_response_cell(summaries_by_channel, name):
    """Render one ledger-response cell using canonical HTML formatting."""
    result = produce({
        "operation": "operational_response_cell",
        "summaries_by_channel": _json_safe(summaries_by_channel or {}),
        "name": name,
    })
    return json.loads(result)


def summary_table_html(data):
    """Render the summary table from emitted acoustic and ledger evidence."""
    result = produce({"operation": "summary_table_html", "data": _json_safe(data or {})})
    return json.loads(result)


def pending_cell(field):
    """Render a pending cell while escaping an evidence-field label."""
    result = produce({"operation": "pending_cell", "field": field})
    return json.loads(result)
