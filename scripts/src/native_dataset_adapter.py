"""Prepare standalone report inputs for the shared dataset composers.

This module owns only the Python boundary preparation that is not part of the
renderer-neutral Rust composer: file-backed target capture for standalone
loads, legacy curve preparation through existing DSP helpers, and comparison
figure context arrays. It does not assemble report sections.
"""

from __future__ import annotations

import csv
import datetime
import json
import math
from pathlib import Path
from typing import Any

from .native_acoustic_t60 import _json_safe
from .native_report import produce


def _target_snapshot(data: Any, json_path: Path | None) -> tuple[dict, dict | None]:
    """Return the captured target state and parsed shape without reopening it."""
    captured = getattr(data, "native_target_snapshot", None)
    if isinstance(captured, dict):
        return captured, captured.get("target") if captured.get("state") == "available" else None

    from .target_overlay import _configured_target_path, load_target_shape

    path = _configured_target_path(data, json_path)
    if path is None:
        return {
            "schema_version": 1,
            "state": "not_configured",
            "target": None,
        }, None
    try:
        target = load_target_shape(data, json_path)
    except (OSError, UnicodeError, ValueError, csv.Error) as error:
        return {
            "schema_version": 1,
            "state": "unavailable",
            "configured_path": str(path),
            "selected_path": str(path),
            "resolution": "standalone_loader",
            "source_verification": "external_unverified",
            "source_integrity": "unavailable",
            "target": None,
            "error_kind": "csv_error" if isinstance(error, csv.Error) else "warning",
            "error": str(error),
        }, None
    if target is None:
        return {
            "schema_version": 1,
            "state": "unavailable",
            "configured_path": str(path),
            "selected_path": str(path),
            "resolution": "standalone_loader",
            "source_verification": "external_unverified",
            "source_integrity": "unavailable",
            "target": None,
            "error_kind": "warning",
            "error": "configured target curve could not be loaded",
        }, None
    return {
        "schema_version": 1,
        "state": "available",
        "configured_path": str(path),
        "selected_path": str(path),
        "resolution": "standalone_loader",
        "source_verification": "external_unverified",
        "source_integrity": "standalone_file_read",
        "target": target,
    }, target


def _native_replay(data: Any) -> dict | None:
    if getattr(data, "native_dsp_replay_present", False) is not True:
        return None
    replay = getattr(data, "native_dsp_replay", None)
    if not isinstance(replay, dict):
        replay = {}
    context = {key: value for key, value in replay.items() if key != "sample_rate_hz"}
    rate = getattr(data, "native_playback_sample_rate", None)
    if type(rate) is not int:
        rate = replay.get("sample_rate_hz")
    if type(rate) is not int or rate <= 0 or rate > 0xFFFFFFFF:
        rate = None
    return {"sample_rate_hz": rate, **context}


def _native_source_curves(replay: dict | None) -> dict[str, dict]:
    """Select only routed responses captured by the native preparation."""
    if not isinstance(replay, dict):
        return {}
    sources = replay.get("routed_sources")
    if not isinstance(sources, dict):
        return {}
    return {
        name: state["curve"]
        for name, state in sources.items()
        if isinstance(name, str)
        and isinstance(state, dict)
        and state.get("status") == "available"
        and isinstance(state.get("curve"), dict)
    }


def _prepared_symmetric_pairs(data: Any, graph: dict, replay: dict | None) -> dict | None:
    """Prepare legacy coherent-pair evidence with the existing DSP owner.

    Native report input is immutable and authoritative, so it never falls back
    to Python phase summation. Markerless standalone input retains the legacy
    calculation as prepared data for the shared Rust figure composer.
    """
    pairs = _json_safe(getattr(data, "symmetric_pairs", None))
    if replay is not None or not isinstance(pairs, dict):
        return pairs

    channels = graph.get("channels")
    if not isinstance(channels, dict):
        return pairs
    from .acoustic_report import symmetric_groups
    from .dsp import symmetric_complex_sum

    pair_groups, _ = symmetric_groups(channels)
    for label, members in pair_groups:
        combined = pairs.get(label)
        if not isinstance(combined, dict):
            continue
        target_frequency = combined.get("freq")
        if not isinstance(target_frequency, list):
            combined.pop("complex_spl", None)
            continue
        left_channel = channels.get(members[0])
        right_channel = channels.get(members[1])
        left = left_channel.get("final_curve") if isinstance(left_channel, dict) else None
        right = right_channel.get("final_curve") if isinstance(right_channel, dict) else None
        coherent = symmetric_complex_sum(left, right, target_frequency)
        if coherent is None:
            combined.pop("complex_spl", None)
        else:
            combined["complex_spl"] = _json_safe(coherent)
    return pairs


def _prepared_header_provenance(datasets: list[tuple[str, Any]]) -> dict:
    """Prepare raw identity values for the shared provenance formatter."""
    from .report import _roomeq_version

    versions = []
    timestamps = []
    for _, data in datasets:
        version = _roomeq_version(data)
        if version:
            versions.append(version)
        timestamp = (data.get("metadata") or {}).get("timestamp")
        if isinstance(timestamp, str) and timestamp.strip():
            timestamps.append(timestamp)
    return {
        "versions": versions,
        "timestamps": timestamps,
        "generated_at": datetime.datetime.now(datetime.timezone.utc).strftime(
            "%Y-%m-%dT%H:%M:%SZ"
        ),
    }


def _legacy_source_curves(data: Any) -> dict:
    """Prepare logical and per-driver acoustic curves through existing DSP APIs."""
    from .dsp import build_post_dsp_source_curves, per_driver_corrected_curve
    from .data_extract import get_plottable_drivers

    result = dict(build_post_dsp_source_curves(data))
    by_channel: dict[str, list[dict]] = {}
    for channel_name, channel in (data.get("channels") or {}).items():
        items = []
        for display_index, driver in enumerate(get_plottable_drivers(channel)):
            name = driver.get("name")
            index = driver.get("index")
            curve = per_driver_corrected_curve(data, channel_name, display_index)
            state = (
                {"status": "available", "curve": curve}
                if isinstance(curve, dict)
                else {
                    "status": "unavailable",
                    "reason": (
                        "no prepared driver response was supplied"
                        if isinstance(driver.get("initial_curve"), dict)
                        else "no measured driver response is available"
                    ),
                }
            )
            items.append({"name": name, "index": index, "state": state})
        if items:
            by_channel[channel_name] = items
    if by_channel:
        result["drivers"] = by_channel
    return result


def _legacy_driver_eq_curves(data: Any) -> dict:
    """Prepare filter previews with the existing canonical DSP helper."""
    from .dsp import per_driver_effective_eq
    from .data_extract import channel_has_eq, get_plottable_drivers

    result: dict[str, list[dict]] = {}
    for channel_name, channel in (data.get("channels") or {}).items():
        drivers = get_plottable_drivers(channel)
        if not drivers:
            continue
        items = []
        for display_index, driver in enumerate(drivers):
            curve = (
                per_driver_effective_eq(data, channel_name, display_index)
                if channel_has_eq(channel)
                else None
            )
            items.append({
                "name": driver.get("name"),
                "index": driver.get("index"),
                "state": (
                    {"status": "available", "curve": curve}
                    if isinstance(curve, dict)
                    else {"status": "unavailable", "reason": "driver EQ preview is unavailable"}
                ),
            })
        result[channel_name] = items
    return result


def _acceptance_evidence_option(data: Any, graph_identity: Any) -> tuple[bool, str]:
    """Verify the acceptance-evidence binding triple (legacy parity).

    Structural coverage (channel set, source classification, per-view
    provenance) stays with the Rust composer; this gate only binds the
    recorded evidence payload to the verified graph identity.
    """
    from .payload_binding import ALGORITHM, payload_digest

    evidence = (data.get("correction_decisions") or {}).get("acceptance_evidence")
    if evidence is None:
        return False, "no production acceptance views were recorded (legacy or unfinalized output)."
    if evidence.get("version") != "acceptance-evidence-v1":
        return False, "unsupported acceptance-evidence version"
    acceptance_binding = evidence.get("binding") or {}
    try:
        digest = payload_digest(evidence.get("payload"), graph_identity)
    except (ValueError, TypeError, AttributeError, OverflowError, RecursionError) as error:
        return False, f"acceptance-evidence verification unavailable: {error}."
    if (
        acceptance_binding.get("algorithm") != ALGORITHM
        or acceptance_binding.get("graph_identity") != graph_identity
        or digest != acceptance_binding.get("sha256")
    ):
        return False, "acceptance-view payload or graph binding changed"
    return True, "acceptance evidence binding verified (not acoustic verification)."


def _binding_option(data: Any, provenance: dict) -> dict:
    from .payload_binding import slim_graph_digest, verify_payload_binding

    verified, reason, identity = verify_payload_binding(data)
    binding = ((data.get("correction_decisions") or {}).get("payload_binding") or {})
    frozen = provenance.get("slim_graph_sha256")
    try:
        fresh = slim_graph_digest(data)
    except (ValueError, TypeError, AttributeError, OverflowError, RecursionError):
        fresh = None
    if verified:
        if not frozen:
            verified, reason = (
                False,
                "loaded report graph has no frozen slim-graph identity (legacy or unwrapped input).",
            )
        elif fresh is None:
            verified, reason = False, "slim-graph digest recomputation unavailable for this report input."
        elif fresh != frozen:
            verified, reason = (
                False,
                "report input changed after load; recorded decisions are stale "
                "(they describe different bytes — re-load before rendering).",
            )
        elif not binding.get("sha256"):
            verified, reason = False, "verified payload binding has no canonical payload digest"
    if verified:
        acceptance_verified, acceptance_reason = _acceptance_evidence_option(data, identity)
    else:
        acceptance_verified, acceptance_reason = False, "acceptance evidence was not verified"
    return {
        "verified": bool(verified),
        "reason": reason,
        "graph_identity": identity,
        "slim_graph_sha256": fresh,
        "canonical_payload_sha256": binding.get("sha256"),
        "acceptance_evidence_verified": acceptance_verified,
        "acceptance_evidence_reason": acceptance_reason,
    }


def prepare_dataset_input(
    data: Any,
    *,
    output_json_path: Path | None = None,
    smoothed_octaves: float = 1.0,
    capture_verification: dict | None = None,
    header_provenance: dict | None = None,
) -> tuple[dict, dict, dict | None]:
    """Prepare the DTO and presentation options consumed by `report_dataset`."""
    from .native_target_overlay import _route_crossover_db
    target_snapshot, target = _target_snapshot(data, output_json_path)
    provenance = getattr(data, "native_report_provenance", None)
    if not isinstance(provenance, dict):
        provenance = {}
    replay = _native_replay(data)
    graph = _json_safe(dict(data))
    prepared = {
        "schema_version": 1,
        "graph": graph,
        "provenance": _json_safe(provenance) if provenance else None,
        "measurement_index": _json_safe(getattr(data, "measurement_index", None)),
        "symmetric_pairs": _prepared_symmetric_pairs(data, graph, replay),
        "target_snapshot": _json_safe(target_snapshot),
        "replay": _json_safe(replay),
    }
    source_curves = None if replay is not None else _legacy_source_curves(data)
    from .dsp import build_post_dsp_source_curves
    if replay is not None:
        reference_curves = _native_source_curves(replay)
    else:
        from .dsp import build_post_dsp_source_curves

        reference_curves = source_curves or build_post_dsp_source_curves(data)
    route_crossover_db = _route_crossover_db(
        data, reference_curves, target, honor_absolute=True
    )
    verified_capture_views = getattr(data, "native_capture_view_bindings", None)
    options = {
        "smoothed_octaves": smoothed_octaves,
        "payload_binding": _binding_option(data, provenance),
        "capture_verification": capture_verification,
        "capture_view_bindings": verified_capture_views,
        "header_provenance": header_provenance or _prepared_header_provenance([("", data)]),
        "route_crossover_db": route_crossover_db,
        "driver_eq_curves": None if replay is not None else _legacy_driver_eq_curves(data),
        "legacy_source_curves": source_curves,
    }
    return _json_safe(prepared), _json_safe(options), target


def build_dataset_payload(
    data: Any,
    *,
    output_json_path: Path | None = None,
    smoothed_octaves: float = 1.0,
    capture_verification: dict | None = None,
    header_provenance: dict | None = None,
) -> dict:
    """Delegate complete dataset section assembly to the shared producer."""
    prepared, options, _ = prepare_dataset_input(
        data,
        output_json_path=output_json_path,
        smoothed_octaves=smoothed_octaves,
        capture_verification=capture_verification,
        header_provenance=header_provenance,
    )
    result = json.loads(produce({
        "operation": "report_dataset",
        "data": prepared,
        "options": options,
    }))
    payload = result.get("payload")
    if not isinstance(payload, dict):
        raise ValueError("shared report dataset producer returned no payload")
    return payload


def _curve_from_single_mode_figure(figure_list: Any, series_name: str) -> dict | None:
    if not isinstance(figure_list, list):
        return None
    for section in figure_list:
        figure = section.get("figure") if isinstance(section, dict) else None
        for series in (figure or {}).get("series", []):
            if isinstance(series, dict) and series.get("name") == series_name:
                return {"freq": series.get("x"), "spl": series.get("y")}
    return None


def _comparison_figure_context(mode_name: str, data: Any, routed: dict) -> dict:
    """Prepare the numeric comparison context used by the Rust composer."""
    from .capture_clock_views import gated_lr_channel
    from .figures import (
        create_comparison_eq_overlay_figure,
        create_comparison_group_delay_figure,
    )
    from .report import _eq_filter_counts

    channels = data.get("channels", {}) or {}
    mode_channels = {}
    for name, channel in channels.items():
        mode_channels[name] = {
            **channel,
            "final_curve": routed.get(name, channel.get("final_curve")),
        }

    lr, fallback = gated_lr_channel(data, mode_channels.get("L"), mode_channels.get("R"))
    if lr:
        mode_channels["L+R"] = lr

    from .report import _report_sample_rate
    rate = _report_sample_rate(data)
    if rate is not None and (not math.isfinite(rate) or rate <= 40.0):
        rate = None

    group_delay: dict[str, Any] = {}
    eq_response: dict[str, Any] = {}
    for channel_name, channel in mode_channels.items():
        group_figures = create_comparison_group_delay_figure(
            channel_name, [(mode_name, channel)], tab=channel_name
        )
        before = _curve_from_single_mode_figure(group_figures, "Before EQ")
        after = _curve_from_single_mode_figure(group_figures, _mode_label(mode_name))
        if before is not None or after is not None:
            group_delay[channel_name] = {"before": before, "after": after}

        if rate is not None:
            eq_figure = create_comparison_eq_overlay_figure(
                channel_name,
                [(mode_name, channel)],
                sample_rates={mode_name: rate},
                tab=channel_name,
            )
            curve = _curve_from_single_mode_figure(
                [eq_figure] if eq_figure else None, _mode_label(mode_name)
            )
            if curve is not None:
                eq_response[channel_name] = curve

    return {
        "lr_channel": mode_channels.get("L+R", {}),
        "lr_fallback_reason": fallback,
        "group_delay": group_delay,
        "eq_filter_counts": _eq_filter_counts(data),
        "eq_response": eq_response,
        "channels": {
            name: {key: channel.get(key) for key in ("pre_ir", "post_ir") if channel.get(key) is not None}
            for name, channel in mode_channels.items()
        },
    }


def _deduplicate_comparison_ir_context(context: dict, graph: dict) -> dict:
    """Keep overrides, omitting waveforms already present in the graph.

    The native composer falls back to the graph for omitted context waveforms.
    The graph and its evidence bindings remain intact, and computed L+R views
    are retained because that logical channel is not a physical graph channel.
    """
    channels = graph.get("channels") or {}
    for name, views in (context.get("channels") or {}).items():
        channel = channels.get(name)
        if not isinstance(channel, dict) or not isinstance(views, dict):
            continue
        for key in ("pre_ir", "post_ir"):
            if key in views and key in channel and views[key] == channel[key]:
                views.pop(key)
    return context


def build_comparison_payload(mode_datasets: list[tuple[str, Any]]) -> dict:
    """Delegate complete cross-mode report assembly to the shared producer."""
    from .report import _mode_label

    modes = []
    for mode_name, data in mode_datasets:
        prepared, options, _target = prepare_dataset_input(data)
        replay = prepared.get("replay")
        if replay is not None:
            routed = _native_source_curves(replay)
        else:
            from .dsp import build_post_dsp_source_curves

            routed = build_post_dsp_source_curves(data)
        contexts = _deduplicate_comparison_ir_context(
            _json_safe(_comparison_figure_context(mode_name, data, routed)),
            prepared["graph"],
        )
        modes.append({
            "name": mode_name,
            "prepared": prepared,
            "options": options,
            "figure_context": contexts,
        })
    result = json.loads(produce({
        "operation": "comparison_dataset",
        "modes": modes,
        "provenance": _prepared_header_provenance(mode_datasets),
    }))
    payload = result.get("payload")
    if not isinstance(payload, dict):
        raise ValueError("shared report comparison producer returned no payload")
    return payload


def _mode_label(mode_name: str) -> str:
    from .figures import _mode_label as label

    return label(mode_name)
