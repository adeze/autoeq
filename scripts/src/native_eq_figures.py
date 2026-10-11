"""Thin adapters for canonical Rust EQ presentation builders."""

from __future__ import annotations

import json

from .data_extract import extract_eq_passes
from .dsp import compute_eq_response, generate_freq_points
from .native_ir_group_delay import _json_safe
from .native_report import produce


def _send(builder, **arguments):
    request = {"operation": "eq_figure", "builder": builder}
    request.update(_json_safe(arguments))
    return json.loads(produce(request))


def _valid_precomputed_response(eq_response_data):
    return (
        eq_response_data
        and "freq" in eq_response_data
        and "spl" in eq_response_data
    )


def _frequency_points(sample_rate):
    points = generate_freq_points(20.0, min(20000.0, sample_rate / 2), 500)
    return [min(frequency, sample_rate / 2) for frequency in points]


def create_eq_figure(
    channel_name,
    eq_filters,
    eq_response_data=None,
    sample_rate=48_000.0,
    tab=None,
):
    """Build the EQ figure after preparing transfer arrays with production DSP."""
    if not eq_filters and not eq_response_data:
        return None

    if _valid_precomputed_response(eq_response_data):
        frequencies = eq_response_data["freq"]
        combined_response = eq_response_data["spl"]
    else:
        frequencies = _frequency_points(sample_rate)
        combined_response = compute_eq_response(eq_filters, frequencies, sample_rate)
    if not combined_response:
        return None

    filters = [
        {
            "metadata": filt,
            "response": compute_eq_response([filt], frequencies, sample_rate),
        }
        for filt in eq_filters
    ]
    return _send(
        "single",
        channel_name=channel_name,
        freq_points=frequencies,
        combined_response=combined_response,
        filters=filters,
        tab=tab,
    )


def create_multipass_eq_figure(
    channel_name,
    channel_data,
    eq_response_data=None,
    sample_rate=48_000.0,
    tab=None,
):
    """Build labeled pass traces while leaving all response math in production DSP."""
    passes = extract_eq_passes(channel_data)
    if not passes:
        return None

    if not any(eq_pass["label"] for eq_pass in passes):
        filters = [filt for eq_pass in passes for filt in eq_pass["filters"]]
        return create_eq_figure(
            channel_name, filters, eq_response_data, sample_rate, tab=tab
        )

    frequencies = _frequency_points(sample_rate)
    all_filters = [filt for eq_pass in passes for filt in eq_pass["filters"]]
    if _valid_precomputed_response(eq_response_data):
        combined_frequencies = eq_response_data["freq"]
        combined_response = eq_response_data["spl"]
    else:
        combined_frequencies = frequencies
        combined_response = compute_eq_response(all_filters, frequencies, sample_rate)

    prepared_passes = [
        {
            "display_name": eq_pass["display_name"],
            "color": eq_pass["color"],
            "response": compute_eq_response(eq_pass["filters"], frequencies, sample_rate),
        }
        for eq_pass in passes
    ]
    return _send(
        "multipass",
        channel_name=channel_name,
        freq_points=frequencies,
        combined_freq_points=combined_frequencies,
        combined_response=combined_response,
        passes=prepared_passes,
        tab=tab,
    )
