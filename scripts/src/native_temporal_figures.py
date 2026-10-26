"""Thin adapters for canonical Rust temporal/acoustic figure builders."""

from __future__ import annotations

import json

from .native_ir_group_delay import _json_safe
from .native_report import produce


def _build(builder, **arguments):
    request = {"operation": "temporal_figure", "builder": builder}
    request.update(_json_safe(arguments))
    return json.loads(produce(request))


def to_db(amplitude):
    """Convert linear impulse-response amplitudes through the shared producer."""
    return _build("to_db", amplitude=amplitude)


def finite_or_gap(values):
    """Replace non-finite samples with gaps through the shared producer."""
    return _build("finite_or_gap", values=values)


def create_ir_figure(channel_name, pre_ir, post_ir, display_ms=100.0, tab=None):
    return _build(
        "create_ir_figure",
        channel_name=channel_name,
        pre_ir=pre_ir,
        post_ir=post_ir,
        display_ms=display_ms,
        tab=tab,
    )


def create_tof_figure(tof_rows, after=False, tab=None):
    return _build("create_tof_figure", tof_rows=tof_rows, after=after, tab=tab)


def create_early_late_figure(label, report, tab=None):
    return _build("create_early_late_figure", label=label, report=report, tab=tab)


def create_t60_octaves_figure(
    label, rows, tab=None, window=None, itu_reference=None
):
    return _build(
        "create_t60_octaves_figure",
        label=label,
        rows=rows,
        tab=tab,
        window=window,
        itu_reference=itu_reference,
    )


def create_symmetric_pair_figure(label, freq, abs_spl, complex_spl=None, tab=None):
    return _build(
        "create_symmetric_pair_figure",
        label=label,
        freq=freq,
        abs_spl=abs_spl,
        complex_spl=complex_spl,
        tab=tab,
    )
