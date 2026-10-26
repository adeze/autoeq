"""Thin adapters for canonical response-figure payload builders."""

import json

from .native_acoustic_t60 import _json_safe
from .native_report import produce


def response_figure_report(data):
    """Return the standard response figures from the shared Rust producer."""
    return json.loads(produce({"operation": "response_figures", "data": _json_safe(data or {})}))


def add_channel_response_overlays(figure, channel_name, target_curve, lfe_plus_channel_curve):
    """Append response overlays through the shared Rust figure builder."""
    updated = json.loads(produce({
        "operation": "add_response_overlays",
        "figure": _json_safe(figure),
        "channel_name": channel_name,
        "target_curve": _json_safe(target_curve),
        "lfe_plus_channel_curve": _json_safe(lfe_plus_channel_curve),
    }))
    # The legacy helper mutates and returns the caller's figure object.
    figure.clear()
    figure.update(updated)
    return figure


def create_channel_figure(channel_name, initial_curve, final_curve, title_suffix="", tab=None):
    return response_figure_report({
        "channel_name": channel_name,
        "initial_curve": initial_curve,
        "final_curve": final_curve,
        "title_suffix": title_suffix,
        "tab": tab,
    })["channel_figure"]


def create_zoomed_figure(
    channel_name, initial_curve, final_curve,
    min_freq=20.0, max_freq=1200.0, y_range=25.0, tab=None,
):
    return response_figure_report({
        "channel_name": channel_name,
        "initial_curve": initial_curve,
        "final_curve": final_curve,
        "min_freq": min_freq,
        "max_freq": max_freq,
        "y_range": y_range,
        "tab": tab,
    })["zoomed_figure"]


def create_smoothed_figure(channel_name, initial_curve, final_curve, octaves=1.0, tab=None):
    return response_figure_report({
        "channel_name": channel_name,
        "initial_curve": initial_curve,
        "final_curve": final_curve,
        "smoothed_octaves": octaves,
        "tab": tab,
    })["smoothed_figure"]


def resample_spl_onto_grid(source_freq, source_spl, target_freq):
    """Resample display SPL values using shared log-frequency interpolation."""
    result = produce({
        "operation": "resample_spl_onto_grid",
        "source_freq": _json_safe(source_freq),
        "source_spl": _json_safe(source_spl),
        "target_freq": _json_safe(target_freq),
    })
    return json.loads(result)
