"""HTML report generation for roomeq visualization."""

import datetime
import json
import math
import re
from html import escape
from pathlib import Path
from typing import Any

from . import wasm_report
from .native_report_provenance import report_provenance as _rust_report_provenance
from .acceptance_views import acceptance_views_html, waveform_status_html
from .acoustic_report import (
    early_reflection_figures,
    early_reflections_html,
    landmarks_table_html,
    level_compensation_html,
    response_landmarks,
    room_t60_rows,
    room_t60_table_html,
    smooth_curve,
    summary_table_html,
    symmetric_groups,
    t60_itu_note,
    t60_itu_reference,
    t60_rows,
    t60_table_html,
    tof_html,
)
from .capture_clock_views import capture_clock_qa_html, gated_lr_channel
from .capture_reflection_views import capture_reflections_html
from .capture_views import (
    capture_views_html,
    optimization_waterfall_html,
    optimization_wavelet_html,
    resonance_summary_html,
)
from .correction_explanation import correction_explanation_html
from .data_extract import (
    channel_has_eq,
    clip_curve_to_measured_band,
    display_channel_entries,
    driver_display_names,
    extract_eq_passes,
    get_channel_sort_key,
    get_plottable_drivers,
)
from .dsp import (
    build_post_dsp_source_curves,
    driver_destination_route,
    kautz_sections,
    per_driver_chain_plugins,
    per_driver_corrected_curve,
    per_driver_effective_eq,
    split_driver_eq_plugins,
    sum_driver_initial_curves,
    symmetric_complex_sum,
)
from .native_driver_replay import resolve_native_driver_curve
from .figures import (
    ComparisonFigureBatch,
    _mode_color,
    _mode_label,
    add_channel_response_overlays,
    create_bass_management_headroom_figure,
    create_bass_management_routing_figure,
    create_channel_figure,
    create_combined_figure,
    create_comparison_eq_overlay_figure,
    create_comparison_group_delay_figure,
    create_comparison_ir_figure,
    create_comparison_overlay_figure,
    create_comparison_phase_figure,
    create_comparison_zoomed_figure,
    create_early_late_figure,
    create_eq_figure,
    create_ir_figure,
    create_mode_subplots_figure,
    create_multipass_eq_figure,
    create_score_comparison_figure,
    create_symmetric_pair_figure,
    create_t60_octaves_figure,
)
from .loaders import RoomEqData
from .native_eq_sections import render_eq_section as _render_eq_section
from .payload_binding import verify_payload_binding
from .native_evidence_sections import evidence_section as _evidence_section
from .native_report_assembly import report_assembly as _report_assembly
from .native_comparison_tables import epa_comparison_tables as _epa_comparison_tables
from .signal_flow import signal_flow_sections
from .target_overlay import (
    TARGET_LEVEL_MATCH_BAND_HZ,
    build_target_overlay_curves,
    global_target_offset_for_pair,
    shift_target_to_reference_band_mean,
)


def _emit_html(sections: list[dict], html: str | None, tab: str | None = None,
             flat: bool = False) -> None:
    """Append an HTML fragment as a raw-HTML section (skips empties)."""
    if html:
        sections.append(wasm_report.html_section(html, tab=tab, flat=flat))


def _emit_fig(sections: list[dict], fig: dict | list | None) -> None:
    """Append a figure section (or list of sections); skips None entries."""
    if fig is None:
        return
    if isinstance(fig, list):
        sections.extend(s for s in fig if s is not None)
    else:
        sections.append(fig)


def _target_crossover_unavailability_html(data: Any) -> str:
    """Explain native target-route omissions without hiding available targets."""
    unavailable = getattr(data, "native_target_crossover_unavailability", None)
    if not isinstance(unavailable, dict) or not unavailable:
        return ""
    details = "".join(
        "<li>" + escape(str(source)) + ": " + escape(str(reason)) + "</li>"
        for source, reason in sorted(unavailable.items(), key=lambda item: str(item[0]))
    )
    return (
        '<section class="epa-footer"><h3>Target crossover overlay unavailable</h3>'
        "<p>The routed target response was omitted because the required playback "
        "rate evidence is unavailable. Any independently available channel target "
        "remains unchanged.</p><ul>"
        + details
        + "</ul></section>"
    )

# Synthetic channel name used for the complex L+R sum tab in the
# comparison report. Picked so it cannot collide with a real recording
# channel id (those are short upper-case codes like L / R / LFE / Cs).
LR_SUM_CHANNEL = "L+R"


def _has_redirected_bass_route(data: dict, source_name: str) -> bool:
    """Return whether ``source_name`` feeds the physical subwoofer output."""
    bass_management = (data.get("metadata", {}) or {}).get("bass_management", {}) or {}
    graph = bass_management.get("routing_graph", {}) or {}
    return any(
        route.get("source_channel") == source_name
        and route.get("route_kind") == "redirected_bass_lowpass_to_sub"
        for route in (graph.get("routes", []) or [])
    )


def _comparison_source_label(channel_name: str, includes_redirected_bass: bool) -> str:
    """Describe comparison curves as logical sources, not physical outputs."""
    label = f"Logical source {channel_name}"
    if includes_redirected_bass:
        label += f" (physical {channel_name} + redirected bass)"
    return label


def _channel_display_final_curve(
    channel_name: str,
    physical_sub: str,
    channel_data: dict,
    post_dsp_curves: dict[str, dict],
    native_replay_present: bool = False,
) -> dict | None:
    """Select the logical response rendered on an individual channel tab.

    The physical-sub ``final_curve`` is the aggregate bass bus. It is not the
    response produced when only the LFE logical input is driven and may contain
    cancellations from unrelated redirected-main routes.
    """
    if channel_name == physical_sub:
        logical_curve = post_dsp_curves.get(channel_name)
        if native_replay_present:
            return logical_curve
        return logical_curve or channel_data.get("final_curve")
    return channel_data.get("final_curve")


def _report_sample_rate(data: Any) -> float | None:
    """Use explicit host replay context before legacy plotting defaults."""
    if getattr(data, "native_dsp_replay_present", False) is True:
        rate = getattr(data, "native_playback_sample_rate", None)
        if type(rate) is int and 0 < rate <= 0xFFFFFFFF:
            return float(rate)
        replay = getattr(data, "native_dsp_replay", None)
        rate = replay.get("sample_rate_hz") if isinstance(replay, dict) else None
        if type(rate) is int and 0 < rate <= 0xFFFFFFFF:
            return float(rate)
        return None
    try:
        rate = float(data.get("sample_rate", 48_000.0) or 48_000.0)
    except (OverflowError, TypeError, ValueError):
        return 48_000.0
    return rate if math.isfinite(rate) and rate > 0.0 else 48_000.0


def _driver_shaping_summary_html(
    data: dict, channel_name: str, driver_index: int
) -> str:
    """Render the per-sub DSP chain summary for a driver tab.

    Lists the driver alignment (gain / delay / low-pass) plus the
    destination-matched route transfer, i.e. everything the tab's EQ
    plot combines on top of the shared channel EQ filters.
    """
    channel = (data.get("channels") or {}).get(channel_name) or {}
    drivers = get_plottable_drivers(channel)
    if driver_index < 0 or driver_index >= len(drivers):
        return ""
    driver = drivers[driver_index]

    gain_db = 0.0
    delay_ms = 0.0
    inverted = False
    low_pass_hz: float | None = None
    low_pass_type: str | None = None
    for plugin in driver.get("plugins") or []:
        if not isinstance(plugin, dict):
            continue
        params = plugin.get("parameters") or {}
        kind = str(plugin.get("plugin_type", "")).lower()
        if kind == "gain":
            try:
                gain_db += float(params.get("gain_db", 0.0))
            except (TypeError, ValueError):
                pass
            inverted = inverted or bool(params.get("invert", False))
        elif kind == "delay":
            try:
                delay_ms += float(params.get("delay_ms", 0.0))
            except (TypeError, ValueError):
                pass
        elif kind == "crossover" and str(params.get("output", "")).lower().startswith(
            "low"
        ):
            try:
                candidate = float(params.get("frequency", 0.0))
            except (TypeError, ValueError):
                continue
            if candidate > 0.0:
                low_pass_hz = candidate
                raw_type = params.get("type")
                low_pass_type = str(raw_type) if raw_type is not None else None

    parts = [f"driver gain {gain_db:+.1f} dB"]
    if inverted:
        parts.append("polarity inverted")
    if abs(delay_ms) > 1e-9:
        parts.append(f"delay {delay_ms:.3f} ms")
    if low_pass_hz is not None:
        lp_label = f" {low_pass_type}" if low_pass_type else ""
        parts.append(f"low-pass{lp_label} @ {low_pass_hz:.1f} Hz")

    route = driver_destination_route(data, str(driver.get("name")))
    route_gain: float | None = None
    route_lp: float | None = None
    if route is not None and route.get("route_kind") == "lfe_lowpass_to_sub":
        try:
            route_gain = float(route.get("gain_db", 0.0))
        except (TypeError, ValueError):
            route_gain = None
        low = route.get("low_pass_hz")
        try:
            route_lp = float(low) if low is not None else None
        except (TypeError, ValueError):
            route_lp = None
    if route_gain is not None:
        parts.append(f"LFE route gain {route_gain:+.1f} dB")
    if route_lp is not None:
        parts.append(f"LFE route low-pass @ {route_lp:.1f} Hz")

    return (
        '<div class="filters-section">\n'
        "    <h3>Sub DSP Chain</h3>\n"
        f'    <div class="filter-list">{escape("; ".join(parts))}</div>\n'
        "</div>\n"
    )


def _eq_filter_descriptor(filt: dict) -> dict:
    """Prepare serialized filter values; Rust owns display formatting."""
    descriptor = {
        "topology": filt.get("topology"),
        "filter_type": str(filt.get("filter_type", "peak")),
        "freq": filt.get("freq", 0),
        "q": filt.get("q", 1),
        "db_gain": filt.get("db_gain", 0),
    }
    if descriptor["topology"] == "kautz_filter":
        descriptor["sections"] = [
            {"pole": pole, "q": q, "weight": gain}
            for pole, q, gain in kautz_sections(filt, math.inf)
        ]
    return descriptor


def _eq_pass_descriptors(passes: list[dict]) -> list[dict]:
    """Prepare validated pass metadata and filter rows for the Rust renderer."""
    return [
        {
            "label": p.get("label", ""),
            "display_name": p.get("display_name", ""),
            "color": p.get("color", ""),
            "filters": [_eq_filter_descriptor(filt) for filt in p.get("filters", [])],
        }
        for p in passes
    ]


def _format_eq_filter_line(filt: dict, number: int) -> str:
    """Render one numbered EQ filter row through the canonical producer."""
    return _render_eq_section(
        "filter_line", {"filter": _eq_filter_descriptor(filt), "number": number}
    )


def _eq_passes_list_html(passes: list[dict]) -> str:
    """Render EQ passes, grouped by pass label when labels are present.

    Numbering restarts at 1 within each rendered group; callers splitting
    filters by origin render one group per origin.
    """
    return _render_eq_section("passes", {"passes": _eq_pass_descriptors(passes)})


def _driver_eq_filters_html(
    data: dict, channel_name: str, driver_index: int, id_prefix: str
) -> str:
    """Render a driver tab's EQ filters split by ownership.

    A driver tab's deployed chain merges the physical driver's own EQ with
    the shared channel input-chain EQ. Listing that merge as one flat
    1..N+M list hides which PEQ lives where, so each origin gets its own
    inner tab (styled like the outer channel tabs) with a 1..N numbering.
    Returns an empty string when neither origin carries EQ filters,
    mirroring the legacy empty-EQ behavior.
    """
    split = split_driver_eq_plugins(data, channel_name, driver_index)
    if split is None:
        return ""
    driver_plugins, shared_plugins = split
    names = driver_display_names(data, channel_name)
    if 0 <= driver_index < len(names):
        driver_name = names[driver_index]
    else:
        drivers = get_plottable_drivers(
            (data.get("channels") or {}).get(channel_name) or {}
        )
        driver_name = (
            str(drivers[driver_index].get("name") or driver_index)
            if 0 <= driver_index < len(drivers)
            else str(driver_index)
        )
    driver_passes = extract_eq_passes({"plugins": driver_plugins})
    shared_passes = extract_eq_passes({"plugins": shared_plugins})
    driver_count = sum(len(p["filters"]) for p in driver_passes)
    shared_count = sum(len(p["filters"]) for p in shared_passes)
    if driver_count == 0 and shared_count == 0:
        return ""
    return _render_eq_section(
        "driver_filters",
        {
            "driver_name": driver_name,
            "channel_name": str(channel_name),
            "id_prefix": id_prefix,
            "driver_passes": _eq_pass_descriptors(driver_passes),
            "shared_passes": _eq_pass_descriptors(shared_passes),
        },
    )


def _eq_filter_table_html(passes: list[dict]) -> str:
    """Render EQ passes as compact tables (one per pass when labeled)."""
    return _render_eq_section(
        "filter_table", {"passes": _eq_pass_descriptors(passes)}
    )


def _gain_plugins_html(data: dict) -> str:
    """Compatibility adapter for the canonical Rust gain summary."""
    return json.loads(wasm_report.produce({
        "operation": "gain_summary_html",
        "data": data,
    }))


def _all_eq_filters_html(data: dict) -> str:
    """Render every EQ filter list in one summary section.

    Mirrors the per-tab filter sections (driver tabs split by origin with a
    1..N numbering) so the whole correction is visible without switching
    tabs. Channels render as a button set: clicking a channel button shows
    that channel's EQ tables. Returns an empty string when no channel
    carries EQ filters.
    """
    channels = data.get("channels") or {}
    panels: list[dict] = []
    for entry in display_channel_entries(data):
        channel_name = entry["channel"]
        driver_index = entry["driver"]
        groups: list[dict] = []
        if driver_index is not None:
            split = split_driver_eq_plugins(data, channel_name, driver_index)
            if split is None:
                continue
            names = driver_display_names(data, channel_name)
            driver_name = (
                names[driver_index]
                if 0 <= driver_index < len(names)
                else str(driver_index)
            )
            driver_passes = extract_eq_passes({"plugins": split[0]})
            shared_passes = extract_eq_passes({"plugins": split[1]})
            if any(p["filters"] for p in driver_passes):
                groups.append({
                    "origin": f"Driver: {driver_name}",
                    "passes": _eq_pass_descriptors(driver_passes),
                })
            if any(p["filters"] for p in shared_passes):
                groups.append({
                    "origin": f"Shared channel {channel_name}",
                    "passes": _eq_pass_descriptors(shared_passes),
                })
        else:
            passes = extract_eq_passes(channels.get(channel_name) or {})
            if any(p["filters"] for p in passes):
                groups.append({
                    "origin": f"Channel {channel_name}",
                    "passes": _eq_pass_descriptors(passes),
                })
        if not groups:
            continue
        panels.append({"label": str(entry["label"]), "groups": groups})
    return _render_eq_section("all_filters", {"panels": panels})


def _crossover_config_html(data: dict) -> str:
    """Delegate the crossover section to the canonical Rust renderer."""
    return json.loads(wasm_report.produce({
        "operation": "crossover_config_html",
        "data": data,
    }))


def _epa_channel_table_html(channel_epa: dict | None) -> str:
    """Render saved before/after EPA evidence through the shared producer."""
    return _evidence_section("epa_channel", epa=channel_epa)


def _epa_comparison_table_html(
    ch_name: str,
    mode_datasets: list[tuple[str, dict]],
) -> str:
    """Render one prepared comparison table through the canonical producer."""
    return _epa_comparison_tables({ch_name: _epa_comparison_entries(ch_name, mode_datasets)}).get(
        ch_name, ""
    )


def _epa_comparison_entries(
    ch_name: str,
    mode_datasets: list[tuple[str, dict]],
) -> list[dict]:
    """Prepare saved EPA values and display labels without formatting HTML."""
    entries: list[dict] = []
    for mode_name, data in mode_datasets:
        ch_epa = (data.get("metadata") or {}).get("epa_per_channel", {}).get(ch_name)
        if not ch_epa:
            continue
        pre = ch_epa.get("pre") or {}
        post = ch_epa.get("post") or {}
        if not pre and not post:
            continue
        entries.append({"mode_label": _mode_label(mode_name), "pre": pre, "post": post})
    return entries


def _epa_summary_pref(metadata: dict) -> tuple[float | None, float | None]:
    """Return shared saved-channel preference means without inferring evidence."""
    return tuple(_evidence_section("epa_summary", metadata=metadata))


def _fmt_db(value: object, suffix: str = " dB") -> str:
    return f"{value:+.2f}{suffix}" if isinstance(value, (int, float)) else "-"


def _fmt_hz(value: object) -> str:
    return f"{value:.1f} Hz" if isinstance(value, (int, float)) else "-"


def _fmt_ms(value: object) -> str:
    return f"{value:.3f} ms" if isinstance(value, (int, float)) else "-"


def _eq_filter_counts(data: dict) -> list[int]:
    """Count emitted sections, including driver-local entries, per channel.

    Kautz banks contribute every basis section, including zero weights. Shared
    channel entries count once, not once per destination or driver. Route-owned
    descriptive markers are excluded consistently with physical-driver replay.
    """
    channels = data.get("channels") or {}
    if isinstance(channels, dict):
        channel_values = channels.values()
    elif isinstance(channels, list):
        channel_values = channels
    else:
        return []

    counts: list[int] = []
    for channel in channel_values:
        if not isinstance(channel, dict):
            continue
        total = 0
        owners = [channel] + [driver for driver in channel.get("drivers") or []
                              if isinstance(driver, dict)]
        plugins = [plugin for owner in owners for plugin in owner.get("plugins") or []]
        for plugin in plugins:
            if not isinstance(plugin, dict) or plugin.get("plugin_type") != "eq":
                continue
            params = plugin.get("parameters") or {}
            if params.get("room_eq_stage") == "route_owned":
                continue
            filters = params.get("filters") or plugin.get("filters") or []
            if isinstance(filters, list):
                total += sum(
                    len(kautz_sections(filt, math.inf))
                    if filt.get("topology") == "kautz_filter" else 1
                    for filt in filters
                )
        counts.append(total)
    return counts


def _eq_filter_summary_html(data: dict) -> str:
    counts = _eq_filter_counts(data)
    return _render_eq_section("summary", {"counts": counts})


def _gd_summary_cells_html(metadata: dict) -> str:
    """Render saved group-delay summary cells through the shared producer."""
    return _evidence_section("group_delay", metadata=metadata)


def _mixed_phase_summary_html(metadata: dict, channels: dict | list) -> str:
    """Render serialized mixed-phase/FIR timing through the shared producer."""
    return _evidence_section("mixed_phase", metadata=metadata, channels=channels)


def _bass_management_summary_html(report: dict) -> str:
    """Compatibility adapter for the canonical Rust bass summary."""
    return _rust_bass_management_sections(report)["summary_html"]


def _bass_management_groups_table_html(report: dict) -> str:
    """Compatibility adapter for the canonical Rust group table."""
    return _rust_bass_management_sections(report)["groups_table_html"]


def _bass_management_sub_outputs_table_html(report: dict) -> str:
    """Compatibility adapter for the canonical Rust output table."""
    return _rust_bass_management_sections(report)["sub_outputs_table_html"]


def _rust_bass_management_sections(report: dict) -> dict[str, str]:
    return json.loads(wasm_report.produce({
        "operation": "bass_management_sections",
        "report": report,
    }))


def _rust_shared_sections(data: dict) -> dict[str, Any]:
    """Run all shared producer sections once for a standalone report."""
    return json.loads(wasm_report.produce({
        "operation": "shared_sections",
        "data": data,
    }))


# Verdict severities shared by the three playback-status boxes.

# Top-level report buckets rendered as centered shell tabs. DSP analysis
# carries the correction rationale and the signal flow; Acoustics analysis
# carries every measured-room section; the psychoacoustic bucket is EPA only.
BUCKET_DSP = "DSP analysis"
BUCKET_ACOUSTICS = "Acoustics analysis"
BUCKET_PSYCHOACOUSTIC = "Psychoacoustic report"


def _workspace_roomeq_version() -> str | None:
    """Version of the `roomeq` binary built from this checkout.

    The report viewer ships in the same workspace, so
    `[workspace.package] version` in the root `Cargo.toml` is the
    report-generator version. DSP outputs predate producer stamping and
    carry no tool version; see `_roomeq_version`.
    """
    try:
        text = (wasm_report.REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8")
    except OSError:
        return None
    package = text.split("[workspace.package]", 1)
    if len(package) != 2:
        return None
    match = re.search(r'(?m)^\s*version\s*=\s*"([^"]+)"',
                      package[1].split("[", 1)[0])
    if not match:
        return None
    return match.group(1).strip() or None


def _roomeq_version(data: dict) -> str | None:
    """RoomEQ version for the header: stamped producer version if present.

    Falls back to this checkout's workspace version, which is exact when
    the report is rendered from the same checkout that produced the DSP.
    """
    metadata = data.get("metadata") or {}
    for key in ("producer_version", "roomeq_version", "tool_version"):
        value = metadata.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    value = data.get("roomeq_version")
    if isinstance(value, str) and value.strip():
        return value.strip()
    return _workspace_roomeq_version()


def _report_provenance(datasets: list[tuple[str, dict]]) -> dict:
    """Header provenance: RoomEQ version, DSP data date, report render date."""
    versions, stamps = [], []
    for _, data in datasets:
        version = _roomeq_version(data)
        if version:
            versions.append(version)
        stamp = (data.get("metadata") or {}).get("timestamp")
        if isinstance(stamp, str) and stamp.strip():
            stamps.append(stamp)
    return _rust_report_provenance({
        "versions": versions,
        "timestamps": stamps,
        "generated_at": datetime.datetime.now(datetime.timezone.utc).strftime(
            "%Y-%m-%dT%H:%M:%SZ"
        ),
    })




def _playback_status_html(metadata: dict, label: str = "", *, data: dict | None = None) -> str:
    """Render playback eligibility using the authoritative payload binding result."""
    verified, binding_reason, _ = verify_payload_binding(data or {})
    return _evidence_section("playback_status", metadata=metadata, label=label,
                             binding_verified=verified, binding_reason=binding_reason)


def build_report_payload(
    data: dict,
    output_json_path: Path | None = None,
    smoothed_octaves: float = 1.0,
    capture_verification: dict | None = None,
    shared_tof_html: str | None = None,
    shared_sections: dict[str, Any] | None = None,
) -> dict:
    """Build one dashboard through the shared report dataset composer.

    The legacy shared-section arguments remain accepted for API compatibility;
    the Rust dataset composer now owns those canonical sections and recomputes
    them from the prepared inputs instead of consuming rendered HTML snapshots.
    """
    from .native_dataset_adapter import build_dataset_payload

    if output_json_path is not None and not isinstance(data, RoomEqData):
        data = RoomEqData(data, output_json_path.resolve().parent)
    return build_dataset_payload(
        data,
        output_json_path=output_json_path,
        smoothed_octaves=smoothed_octaves,
        capture_verification=capture_verification,
    )


def create_html_report(
    data: dict,
    output_path: Path,
    output_json_path: Path | None = None,
    smoothed_octaves: float = 1.0,
    capture_verification: dict | None = None,
) -> None:
    """Write the report document from the shared dataset-composed payload."""
    payload = build_report_payload(
        data, output_json_path, smoothed_octaves, capture_verification
    )
    wasm_report.write_report(output_path, payload["title"], payload)
    print(f"HTML report written to: {output_path}")


def create_comparison_html_report(
    mode_datasets: list[tuple[str, dict]],
    output_path: Path,
) -> None:
    """Write the comparison document from the shared comparison composer."""
    from .native_dataset_adapter import build_comparison_payload

    payload = build_comparison_payload(mode_datasets)
    wasm_report.write_report(output_path, payload["title"], payload)
    print(f"Comparison report written to: {output_path}")
