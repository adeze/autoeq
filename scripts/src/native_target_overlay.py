"""Thin adapter for the canonical Rust target-overlay report math."""

from __future__ import annotations

import json
import math

from .dsp import _crossover_response
from .native_acoustic_t60 import _json_safe
from .native_report import produce


def _route_crossover_db(data, reference_curves, loaded_target, honor_absolute=True):
    """Pass the existing canonical DSP low-pass response to the Rust builder.

    The report crate intentionally does not duplicate crossover coefficient
    generation. This small boundary preserves the existing DSP implementation
    while Rust owns interpolation, routing decisions, alignment, and output.
    """
    metadata = data.get("metadata") or {}
    bass = metadata.get("bass_management") or {}
    graph = bass.get("routing_graph") or {}
    routes = graph.get("routes") or []
    effective_config = metadata.get("effective_config") or {}
    sample_rate = float(effective_config.get("sample_rate", 48_000.0))
    result = {}
    if loaded_target is None:
        return result
    channels = data.get("channels") or {}
    for channel, reference in reference_curves.items():
        absolute = (channels.get(channel) or {}).get("target_curve") or {}
        if honor_absolute and absolute.get("freq") and absolute.get("spl"):
            continue
        route = next(
            (
                candidate
                for candidate in routes
                if candidate.get("source_channel") == channel
                and candidate.get("route_kind") == "lfe_lowpass_to_sub"
            ),
            None,
        )
        if route is None:
            continue
        cutoff_hz = route.get("low_pass_hz")
        frequencies = list((reference or {}).get("freq") or [])
        if not isinstance(cutoff_hz, (int, float)) or cutoff_hz <= 0.0 or not frequencies:
            continue
        response = _crossover_response(
            str(route.get("crossover_type") or "LR24"),
            "low",
            float(cutoff_hz),
            frequencies,
            sample_rate,
        )
        result[channel] = [20.0 * math.log10(max(abs(value), 1.0e-10)) for value in response]
    return result


def build_target_overlay_curves(data, reference_curves, loaded_target=None):
    """Build aligned overlays using loaded target data without file access.

    ``loaded_target`` is the result of the existing upstream CSV loader. File
    lookup and parsing remain outside the Rust report producer.
    """
    data = data or {}
    reference_curves = reference_curves or {}
    request = {
        "operation": "target_overlay_curves",
        "data": _json_safe(data),
        "reference_curves": _json_safe(reference_curves),
        "loaded_target": _json_safe(loaded_target),
        "route_crossover_db": _json_safe(
            _route_crossover_db(data, reference_curves, loaded_target)
        ),
    }
    return json.loads(produce(request))


def target_overlay_helper(helper, **arguments):
    """Route one in-memory target-overlay utility to the Rust implementation."""
    return json.loads(
        produce(
            {
                "operation": "target_overlay_helper",
                "helper": helper,
                **_json_safe(arguments),
            }
        )
    )


def target_shape_for_channel(data, channel, target, reference):
    route_db = _route_crossover_db(
        data or {}, {channel: reference or {}}, target, honor_absolute=False
    )
    return target_overlay_helper(
        "target_shape_for_channel",
        data=data or {},
        channel=channel,
        target=target,
        reference=reference,
        route_crossover_db=route_db,
    )
