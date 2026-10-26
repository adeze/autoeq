"""Viewer-side acoustic helpers for the feat-report sections.

Only operations fully determined by the roomeq output JSON live here:
fractional-octave smoothing (reusing the viewer convention), band means,
IR envelopes, and arrival-time tables. Anything needing new measurement
analysis (RT60, reflection picking, waterfall/wavelet, early/late splits)
is a roomeq/math-audio requirement, not a viewer estimate.
"""

import math

from . import native_acoustic_summary as _native_summary
from . import native_acoustic_curves as _native_curves
from . import native_acoustic_t60 as _native_t60
from . import native_acoustic_reflections as _native_reflections
from .native_report import produce
import json

SPEED_OF_SOUND_M_S = 343.0

# Colour thresholds from reviews/feat-report.md (green, yellow, red).
# Each entry: (green_predicate, yellow_predicate); else red.
SUMMARY_THRESHOLDS = {
    "operational_room_response_pct": {"green": lambda v: v > 90.0,
                                      "yellow": lambda v: v >= 80.0},
    "t60_flatness_pct": {"green": lambda v: v > 80.0,
                          "yellow": lambda v: v >= 50.0},
    "t60_itu_pct": {"green": lambda v: v > 80.0,
                    "yellow": lambda v: v >= 50.0},
    "notch_db": {"green": lambda v: v > -10.0, "yellow": lambda v: v >= -20.0},
    "early_late_ratio_db": {"green": lambda v: v > 3.0, "yellow": lambda v: v >= 0.0},
    "early_reflection_level_db": {"green": lambda v: v < -10.0,
                                  "yellow": lambda v: v <= -3.0},
}

SUB_NAMES = {"sub", "lfe", "sub1", "sub2"}
T60_OCTAVE_CENTERS_HZ = (63, 125, 250, 500, 1000, 2000, 4000, 8000, 16000)

# Default ±tolerance (seconds) for the Section 1 T60 flatness share when the
# operator declares none. ITU-R BS.1116-2 §8.2.3.1, Fig. 1 bounds the room
# reverberation time about its mean Tm at ±0.05 s across the 200 Hz–4 kHz
# midband (wider at the edges: +0.3 s leniency at 63 Hz, ±0.1 s at 8 kHz;
# 16 kHz lies above the figure's 8 kHz bound). The viewer applies the midband
# bound uniformly as a documented simplification; declare
# reporting.t60_flatness_tolerance_s to override it.
DEFAULT_T60_FLATNESS_TOLERANCE_S = 0.05


def band_mean(freq, spl, lo_hz, hi_hz):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_curves.band_mean(freq, spl, lo_hz, hi_hz)


def _valid_curve_points(freq, spl):
    """Reject malformed curve axes before pairing samples by frequency."""
    if (not isinstance(freq, (list, tuple)) or not isinstance(spl, (list, tuple))
            or not freq or len(freq) != len(spl)):
        return None
    points = []
    previous = 0.0
    for f, s in zip(freq, spl):
        if (isinstance(f, bool) or not isinstance(f, (int, float))
                or not math.isfinite(f) or f <= previous
                or isinstance(s, bool) or not isinstance(s, (int, float))
                or not math.isfinite(s)):
            return None
        points.append((f, s))
        previous = f
    return points


def smooth_curve(curve, octaves=1.0):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_curves.smooth_curve(curve, octaves)


def is_sub_channel(name):
    return str(name).strip().lower() in SUB_NAMES or str(name).lower().startswith("sub")


def _predicted_band_mean(channel, lo_hz, hi_hz):
    """Band mean of the predicted post-DSP curve, or None when absent."""
    if not isinstance(channel, dict):
        return None
    curve = channel.get("final_curve")
    if not curve or not curve.get("freq"):
        return None
    return band_mean(curve["freq"], curve.get("spl"), lo_hz, hi_hz)


def level_compensation(channels):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_curves.level_compensation(channels)


def deepest_notch_db(channel_data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_curves.deepest_notch_db(channel_data)


def response_landmarks(channel_data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_curves.response_landmarks(channel_data)


def landmarks_table_html(data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_curves.landmarks_table_html(data)


def symmetric_groups(channels):
    """Delegate ordered symmetric grouping to the canonical Rust producer."""
    groups, unpaired = json.loads(produce({"operation": "symmetric_groups", "names": [str(name) for name in channels]}))
    return [(label, members) for label, members in groups], unpaired


def pair_sum_difference(curve_a, curve_b):
    """Delegate validated equal-grid pair arithmetic to the canonical Rust producer."""
    from .native_acoustic_t60 import _json_safe
    return json.loads(produce({"operation": "pair_sum_difference", "a": _json_safe(curve_a), "b": _json_safe(curve_b)}))


def tof_table(metadata):
    """Read canonical shared time-of-flight rows, retaining missing values."""
    return json.loads(produce({"operation": "time_of_flight_rows", "metadata": metadata or {}}))


def t60_rows(channel_data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_t60.t60_rows(channel_data)


def t60_flatness_tolerance_s(data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_t60.t60_flatness_tolerance_s(data)


def t60_flatness_pct(data, channel_name):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_t60.t60_flatness_pct(data, channel_name)


def t60_itu_reference(data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_t60.t60_itu_reference(data)


def t60_itu_note(reference):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_t60.t60_itu_note(reference)


def t60_flatness_window(data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_t60.t60_flatness_window(data)


def room_t60_table_html(data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_t60.room_t60_table_html(data)


def t60_table_html(channel_data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_t60.t60_table_html(channel_data)


def early_late_ratio_db(channel_data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_reflections.reflection_report(channel_data, "")["early_late_ratio_db"]


def _validated_early_reflections(channel_data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_reflections.reflection_report(channel_data, "")["validated_report"]


def early_reflection_level_db(channel_data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_reflection_level(channel_data)


def early_reflections_html(channel_data, label):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_reflections.reflection_report(channel_data, label)["html"]


def early_reflection_figures(channel_data, label, tab=None):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_reflections.reflection_report(channel_data, label, tab)["figures"]


def room_t60_rows(data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_t60.room_t60_rows(data)


def notch_cell(notch):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_curves.notch_cell(notch)


def early_late_ratio_cell(channel_data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_reflections.reflection_report(channel_data, "")["early_late_ratio_cell"]


def early_reflection_level_cell(channel_data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_reflections.reflection_report(channel_data, "")["early_reflection_level_cell"]


def t60_itu_pct(data, channel_name):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_t60.t60_itu_pct(data, channel_name)


def t60_itu_cell(data, channel_name):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_t60.t60_itu_cell(data, channel_name)


def t60_flatness_cell(data, channel_name):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_t60.t60_flatness_cell(data, channel_name)


def pending_cell(field):
    """Delegate missing-evidence rendering to the shared report producer."""
    return _native_summary.pending_cell(field)


def operational_summaries_by_channel(data):
    """Delegate emitted or final-ledger scope shares to the shared producer."""
    return _native_summary.operational_summaries_by_channel(data)


def operational_response_cell(summaries_by_channel, name):
    """Delegate operational scope rendering to the shared producer."""
    return _native_summary.operational_response_cell(summaries_by_channel, name)


def summary_table_html(data):
    """Delegate measured acoustic and operational summaries to the shared producer."""
    return _native_summary.summary_table_html(data)


def level_compensation_html(data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_curves.level_compensation_html(data)


def tof_html(metadata):
    """Render time-of-flight evidence using the shared native/WASM producer."""
    return json.loads(produce({"operation": "time_of_flight", "metadata": metadata or {}}))


def _native_reflection_level(channel_data):
    level = _native_reflections.reflection_report(channel_data, "")["early_reflection_level_db"]
    return None if level is None else (level["level_db"], level["upper_bound"])
