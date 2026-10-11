"""Thin adapters for shared response-curve summaries and display arithmetic."""

import json
from .native_report import produce
from .native_acoustic_t60 import _json_safe


def curve_report(data):
    return json.loads(produce({"operation": "acoustic_curves", "data": _json_safe(data or {})}))


def band_mean(freq, spl, lo_hz, hi_hz):
    return json.loads(produce({"operation": "acoustic_band_mean", "curve": _json_safe({"freq": freq, "spl": spl}), "low_hz": lo_hz, "high_hz": hi_hz}))


def smooth_curve(curve, octaves=1.0):
    return json.loads(produce({"operation": "acoustic_smooth_curve", "curve": _json_safe(curve), "octaves": octaves}))


def response_landmarks(channel_data):
    marks = curve_report({"channels": {"_channel": channel_data or {}}})["channels"]["_channel"]["landmarks"]
    if marks is not None:
        marks["peaks"] = [tuple(point) for point in marks["peaks"]]
        marks["notches"] = [tuple(point) for point in marks["notches"]]
    return marks


def deepest_notch_db(channel_data):
    return curve_report({"channels": {"_channel": channel_data or {}}})["channels"]["_channel"]["deepest_notch_db"]


def level_compensation(channels):
    return curve_report({"channels": channels})["level_compensation"]


def level_compensation_html(data):
    return curve_report(data)["level_compensation_html"]


def landmarks_table_html(data):
    return curve_report(data)["landmarks_table_html"]


def notch_cell(notch):
    return json.loads(produce({"operation": "acoustic_notch_cell", "notch": _json_safe(notch)}))
