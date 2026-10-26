"""Thin Python adapters for the canonical Rust measured-T60 report code."""

import json
import math

from .native_report import produce


def _json_safe(value):
    """Replace non-finite numeric evidence with JSON null for Rust validation."""
    if isinstance(value, float) and not math.isfinite(value):
        return None
    if isinstance(value, dict):
        return {key: _json_safe(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_json_safe(item) for item in value]
    if isinstance(value, tuple):
        return [_json_safe(item) for item in value]
    return value


def t60_report(data):
    """Return the complete T60 report summary for serialized report data."""
    return json.loads(produce({"operation": "t60_report", "data": _json_safe(data or {})}))


def t60_rows(channel_data):
    """Validate and return one channel's measured octave-band rows."""
    result = produce({"operation": "t60_rows", "channel": _json_safe(channel_data or {})})
    return json.loads(result)


def t60_itu_note(reference):
    """Return the BS.1116 explanatory note for a reference or no evidence."""
    return json.loads(produce({"operation": "t60_itu_note", "reference": reference}))


def t60_flatness_tolerance_s(data):
    value = t60_report(data)["tolerance"]
    return value["seconds"], value["declared"]


def t60_flatness_pct(data, channel_name):
    return t60_report(data)["channels"].get(channel_name, {}).get("flatness_pct")


def t60_flatness_window(data):
    channels = t60_report(data)["channels"]
    if not channels:
        return None
    return next(iter(channels.values())).get("flatness_window_s")


def t60_itu_reference(data):
    return t60_report(data).get("reference")


def room_t60_rows(data):
    return t60_report(data).get("room_rows")


def room_t60_table_html(data):
    return t60_report(data).get("room_t60_table_html", "")


def t60_table_html(channel_data):
    summary = t60_report({"channels": {"_channel": channel_data or {}}})
    return summary["channels"]["_channel"]["t60_table_html"]


def t60_itu_pct(data, channel_name):
    return t60_report(data)["channels"].get(channel_name, {}).get("itu_pct")


def t60_itu_cell(data, channel_name):
    return t60_report(data)["channels"].get(channel_name, {}).get("itu_cell_html")


def t60_flatness_cell(data, channel_name):
    return t60_report(data)["channels"].get(channel_name, {}).get("flatness_cell_html")
