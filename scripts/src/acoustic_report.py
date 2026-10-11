"""Viewer-side acoustic helpers for the feat-report sections.

Only operations fully determined by the roomeq output JSON live here:
fractional-octave smoothing (reusing the viewer convention), band means,
IR envelopes, and arrival-time tables. Anything needing new measurement
analysis (RT60, reflection picking, waterfall/wavelet, early/late splits)
is a roomeq/math-audio requirement, not a viewer estimate.
"""

import math
from html import escape
from itertools import pairwise

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


def _pair_key(name):
    """Normalise a channel name to a symmetric-pair key, or None."""
    n = str(name).strip().upper()
    pairs = {
        "L": "L+R", "R": "L+R",
        "SL": "SL+SR", "SR": "SL+SR",
        "TFL": "TFL+TFR", "TFR": "TFL+TFR",
        "TBL": "TBL+TBR", "TBR": "TBL+TBR",
        "SSL": "SSL+SSR", "SSR": "SSL+SSR",
        "LBL": "LBL+LBR", "LBR": "LBL+LBR",
    }
    return pairs.get(n)


def symmetric_groups(channels):
    """Group channel names into symmetric monitor pairs.

    Returns (groups, unpaired): groups is a list of (label, [a, b]) with
    both members present; unpaired lists the rest (center, subs, solos).
    """
    buckets: dict[str, list[str]] = {}
    order: list[str] = []
    for name in channels:
        key = _pair_key(name)
        if key is None:
            continue
        if key not in buckets:
            buckets[key] = []
            order.append(key)
        buckets[key].append(str(name))
    groups = [(k, buckets[k]) for k in order if len(buckets[k]) == 2]
    grouped = {n for _, ms in groups for n in ms}
    unpaired = [str(n) for n in channels if str(n) not in grouped]
    return groups, unpaired


def pair_sum_difference(curve_a, curve_b):
    """Magnitude-sum and difference SPL curves for a symmetric pair.

    Curves must share the same frequency grid. Never combine values at
    different frequencies by array index.
    """
    if not curve_a or not curve_b:
        return None
    fa, sa = curve_a.get("freq"), curve_a.get("spl")
    fb, sb = curve_b.get("freq"), curve_b.get("spl")
    points_a = _valid_curve_points(fa, sa)
    points_b = _valid_curve_points(fb, sb)
    if (points_a is None or points_b is None or len(points_a) != len(points_b)
            or any(abs(a[0] - b[0]) > 1e-9 * max(1.0, abs(a[0]), abs(b[0]))
                   for a, b in zip(points_a, points_b))):
        return None
    lin_sum, lin_diff = [], []
    for (_, xa), (_, xb) in zip(points_a, points_b):
        try:
            pa = 10.0 ** (xa / 20.0)
            pb = 10.0 ** (xb / 20.0)
        except OverflowError:
            return None
        if not math.isfinite(pa) or not math.isfinite(pb):
            return None
        lin_sum.append(pa + pb)
        lin_diff.append(abs(pa - pb))
    if not all(math.isfinite(value) for value in (*lin_sum, *lin_diff)):
        return None
    def _to_spl(lin):
        return [20.0 * math.log10(v) if v > 1e-12 else -120.0 for v in lin]
    return {"freq": list(fa), "sum_spl": _to_spl(lin_sum), "diff_spl": _to_spl(lin_diff)}


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
    return f'<td style="background:#eee;color:#888" title="needs roomeq field: {escape(field)}">pending</td>'


_DELIVERED_STATUSES = {"applied", "already_acceptable", "constrained"}


def _derive_summaries_from_decisions(decisions):
    """Derive req-R6 channel summaries from legacy decision records.

    Mirrors the normative rule in
    ``crates/roomeq-model/src/decision_ledger.rs`` (``operational_summaries``):
    scope is final-claim records (stage ``final`` with a non-empty
    ``final_graph_identity``) with action ``equalize``, grouped by
    ``physical_output``; records superseded by another record are excluded
    from both counts; delivered scope carries status ``applied``,
    ``already_acceptable`` or ``constrained``. Channels without decided
    final equalization scope stay absent (rendered as pending, never 100%).
    """
    superseded = set()
    for record in decisions:
        if not isinstance(record, dict):
            continue
        ids = record.get("supersedes_ids") or []
        if isinstance(ids, list):
            superseded.update(i for i in ids if isinstance(i, str))
    decided: dict[str, int] = {}
    delivered: dict[str, int] = {}
    for record in decisions:
        if not isinstance(record, dict):
            continue
        if record.get("stage") != "final":
            continue
        identity = record.get("final_graph_identity")
        if not isinstance(identity, str) or not identity.strip():
            continue
        if record.get("action") != "equalize":
            continue
        decision_id = record.get("decision_id")
        if isinstance(decision_id, str) and decision_id in superseded:
            continue
        channel = record.get("physical_output") or record.get("logical_input")
        if not isinstance(channel, str) or not channel:
            continue
        decided[channel] = decided.get(channel, 0) + 1
        if record.get("status") in _DELIVERED_STATUSES:
            delivered[channel] = delivered.get(channel, 0) + 1
    indexed = {}
    for channel, decided_count in decided.items():
        delivered_count = delivered.get(channel, 0)
        if delivered_count > decided_count:
            continue
        indexed[channel] = (
            100.0 * delivered_count / decided_count,
            decided_count,
            delivered_count,
        )
    return indexed


def operational_summaries_by_channel(data):
    """Index emitted req-R6 channel summaries by delivered output identity.

    Uses the engine-emitted ``channel_summaries`` when present; legacy
    outputs carry only ``decisions``, from which the same shares are
    derived with the normative ledger rule.
    """
    ledger = data.get("correction_decisions", {}) or {}
    summaries = ledger.get("channel_summaries", []) or []
    indexed = {}
    for entry in summaries:
        if not isinstance(entry, dict):
            continue
        channel = entry.get("channel")
        pct = entry.get("operational_response_pct")
        decided = entry.get("decided_equalize")
        delivered = entry.get("delivered_equalize")
        if (not isinstance(channel, str) or not channel
                or not isinstance(pct, (int, float)) or isinstance(pct, bool)
                or not math.isfinite(pct)):
            continue
        indexed[channel] = (float(pct), decided, delivered)
    if indexed:
        return indexed
    decisions = ledger.get("decisions", []) or []
    if isinstance(decisions, list) and decisions:
        return _derive_summaries_from_decisions(decisions)
    return indexed


def operational_response_cell(summaries_by_channel, name):
    """Section 1 operational-response cell from the emitted ledger share."""
    hit = summaries_by_channel.get(name)
    if hit is None:
        return ('<td style="background:#eee;color:#666" '
                'title="No final EQ assessment is present in the saved decision ledger">'
                'Not assessed</td>')
    pct, decided, delivered = hit
    th = SUMMARY_THRESHOLDS["operational_room_response_pct"]
    color = "#2ecc71" if th["green"](pct) else "#f1c40f" if th["yellow"](pct) else "#e74c3c"
    detail = f"{delivered}/{decided} final EQ scope delivered" if (
        isinstance(decided, int) and isinstance(delivered, int)) else "final EQ scope share"
    return (f'<td style="background:{color}33" title="operational room response: {escape(detail)} '
            f"(share of decided final equalization records delivered: Applied/AlreadyAcceptable/Constrained; "
            f"empty scope stays pending, never 100%)\">{pct:.1f}</td>")


def summary_table_html(data):
    """Section 1 summary from emitted evidence, with pending unknown metrics."""
    channels = data.get("channels", {}) or {}
    summaries_by_channel = operational_summaries_by_channel(data)
    names = sorted(channels.keys())
    parts = [
        '<div class="filters-section">\n<h3>Results summary</h3>\n',
        ('<p class="epa-footer">Acoustic values use the imported room impulse response. '
        'Operational response is the share of final EQ decisions confirmed as delivered; '
        '“Not assessed” means the saved ledger has no final EQ assessment. '
        'Missing acoustic values require a measured room IR and valid analysis.</p>\n'),
        ('<table class="epa-table"><thead><tr><th>Speaker</th>'
        "<th>Operational room response (%)</th>"
        "<th>Early reflection level (dB)</th>"
        "<th>Early vs late ratio (dB)</th>"
        "<th>T60 flatness in window (%)</th>"
        "<th>T60 within ITU recommendation (%)</th>"
        "<th>Deepest notch &lt; 300 Hz (dB)</th>"
        "</tr></thead><tbody>\n"),
    ]
    for name in names:
        notch = deepest_notch_db(channels[name])
        parts.append(
            f"<tr><td>{escape(str(name))}</td>"
            f"{operational_response_cell(summaries_by_channel, name)}"
            f"{early_reflection_level_cell(channels[name])}"
            f"{early_late_ratio_cell(channels[name])}"
            f"{t60_flatness_cell(data, name)}"
            f"{t60_itu_cell(data, name)}"
            f"{notch_cell(notch)}</tr>\n"
        )
    parts.append("</tbody></table></div>\n")
    return "".join(parts)


def level_compensation_html(data):
    """Delegate to the canonical shared Rust/WASM report producer."""
    return _native_curves.level_compensation_html(data)


def tof_html(metadata):
    """Render time-of-flight evidence using the shared native/WASM producer."""
    return json.loads(produce({"operation": "time_of_flight", "metadata": metadata or {}}))


def _native_reflection_level(channel_data):
    level = _native_reflections.reflection_report(channel_data, "")["early_reflection_level_db"]
    return None if level is None else (level["level_db"], level["upper_bound"])
