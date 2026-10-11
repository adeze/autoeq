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
) -> dict | None:
    """Select the logical response rendered on an individual channel tab.

    The physical-sub ``final_curve`` is the aggregate bass bus. It is not the
    response produced when only the LFE logical input is driven and may contain
    cancellations from unrelated redirected-main routes.
    """
    if channel_name == physical_sub:
        return post_dsp_curves.get(channel_name) or channel_data.get("final_curve")
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
    """Build every RoomEQ dashboard without loading HTML or WASM assets.

    This function assembles one dataset; cross-mode comparison assembly remains
    in :func:`create_comparison_html_report`.

    Args:
        data: Output JSON data (roomeq result)
        output_json_path: Path to output JSON (for resolving relative paths)
        smoothed_octaves: Fractional-octave smoothing for the Section 2
            smoothed-response overlays (feat-report asks for 1 octave).
        capture_verification: Optional matched verification report. Its graph
            identity must match the verified saved optimization payload.
        shared_tof_html: Optional snapshot already produced by the shared Rust
            time-of-flight API, used by native hosts without a subprocess.
        shared_sections: Optional section values produced by the shared Rust
            renderer. Present keys, including empty values or arrays, override
            standalone fallback generation.
    """
    if output_json_path is not None and not isinstance(data, RoomEqData):
        data = RoomEqData(data, output_json_path.resolve().parent)
    channels_dict = data.get("channels", {})
    metadata = data.get("metadata", {})
    generated_sections: dict[str, Any] | None = None

    def shared_section(name: str) -> Any:
        nonlocal generated_sections
        if shared_sections is not None and name in shared_sections:
            return shared_sections[name]
        if generated_sections is None:
            generated_sections = _rust_shared_sections(data)
        return generated_sections.get(name)

    # Sort channels by classical order
    sorted_channel_names = sorted(channels_dict.keys(), key=get_channel_sort_key)
    channels = [(name, channels_dict[name]) for name in sorted_channel_names]

    page_title = "Room EQ"

    # Build HTML content
    # Sections in document order; the shell renders them and builds the
    # per-channel tab bar from the section `tab` fields.
    sections: list[dict] = []
    _emit_html(sections, _playback_status_html(metadata, data=data), flat=True)
    status_sections = sections
    why, overview, speakers, timing, symmetric, time_domain, epa = ([] for _ in range(7))
    channel_blocks = []
    landmark_figures = []
    sections = why
    _emit_html(sections, correction_explanation_html(data))
    _emit_html(sections, acceptance_views_html(data))
    _emit_html(sections, waveform_status_html(data))
    _emit_html(overview, summary_table_html(data))
    for name, channel in channels:
        marks = response_landmarks(channel)
        curve = smooth_curve(channel.get("initial_curve"), 1 / 3)
        if marks and curve:
            annotations = [wasm_report.annotation(f, level, f"{kind}: {f:.0f} Hz")
                           for kind in ("peaks", "notches") for f, level in marks[kind]]
            lines = ([wasm_report.vline(marks["lf_extension_hz"], "#777", dash="dash")]
                     if marks["lf_extension_hz"] is not None else [])
            _emit_fig(landmark_figures, wasm_report.figure(
                f"Frequency landmarks — {name} (1/3 octave)",
                wasm_report.axis("Frequency (Hz)", "log", 20, 20000),
                wasm_report.axis("Measured level (dB)"),
                series_list=[wasm_report.series(name, curve["freq"], curve["spl"])],
                annotations=annotations, vlines=lines, tab=name))


    _emit_html(time_domain, capture_clock_qa_html(data))
    _emit_html(time_domain, capture_reflections_html(data))
    mixed_phase_html = _mixed_phase_summary_html(metadata, channels_dict)
    if mixed_phase_html:
        _emit_html(sections, mixed_phase_html)

    # Combined plot
    sections = overview
    _emit_fig(sections, create_combined_figure(data, output_json_path))

    # Single-screen summaries: the full PEQ listing and the crossover
    # configuration, so nothing requires switching per-channel tabs.
    _emit_html(sections, shared_section("gain_summary_html"))
    _emit_html(sections, shared_section("crossover_config_html"))
    sections = time_domain
    room_decay = room_t60_rows(data)
    decay_reference = t60_itu_reference(data)
    if room_decay:
        _emit_html(sections, room_t60_table_html(data))
        room_decay_fig = create_t60_octaves_figure("Room mean", room_decay, itu_reference=decay_reference)
        if room_decay_fig:
            _emit_fig(sections, room_decay_fig)
            _emit_html(sections,
                '<p class="epa-footer">Arithmetic mean of valid measured-room T60 '
                'estimates at each octave. Speaker coverage varies by band; invalid '
                'fits are excluded.</p>\n' + t60_itu_note(decay_reference)
            )
        _emit_html(sections, 
            '<p class="epa-footer">Room T60 contributing speakers by octave: '
            + ", ".join(f"{row['centre_hz']} Hz: {row['speaker_count']}"
                        for row in room_decay)
            + '</p>\n'
        )

    # Time of flight (feat-report Section 3): table only. The before/after
    # bar charts were removed; the table carries both arrival columns.
    sections = timing
    tof_snapshot = shared_section("time_of_flight_html")
    if tof_snapshot is None:
        tof_snapshot = shared_tof_html
    _emit_html(sections, tof_html(metadata) if tof_snapshot is None else tof_snapshot)

    # Symmetric-monitor summing, magnitude domain (feat-report Section 2).
    sections = symmetric
    # The complex pressure sum needs phase data roomeq does not emit yet.
    pair_groups, unpaired = symmetric_groups(channels_dict)
    if pair_groups or unpaired:
        # Bare paragraphs: the group summary already titles this section and
        # the shell wraps html sections in its own card.
        symmetric_head = (
            '<p>Rust aligns final curves within their shared frequency range. '
            'The absolute sum assumes equal phase; it is an upper bound, not a prediction '
            'of interference from simultaneous playback. The complex sum is the coherent '
            'pressure sum from measured phase; the sum difference between them is the '
            'cancellation loss.</p>'
        )
        if unpaired:
            symmetric_head += (
                '<p>Unpaired channels (not summed): '
                + ", ".join(escape(name) for name in unpaired)
                + '</p>'
            )
        _emit_html(sections, symmetric_head)
    for label, members in pair_groups:
        group_start = len(sections)
        combo = (getattr(data, "symmetric_pairs", None) or {}).get(label)
        if combo is None:
            _emit_html(sections,
                '<p>Symmetric pair ' + escape(label)
                + ': no Rust pair export in this output bundle. Regenerate with the updated RoomEQ exporter.</p>', tab=label
            )
            continue
        abs_freq = list(combo["freq"])
        complex_spl = symmetric_complex_sum(
            (channels_dict.get(members[0]) or {}).get("final_curve"),
            (channels_dict.get(members[1]) or {}).get("final_curve"),
            abs_freq,
        )
        _emit_fig(sections, create_symmetric_pair_figure(
            label, abs_freq, list(combo["sum_spl"]), complex_spl
        ))
        if complex_spl is None:
            _emit_html(sections,
                '<p class="epa-footer">Symmetric pair ' + escape(label)
                + ': complex sum unavailable without finite measured phase on both '
                + escape(members[0]) + ' and ' + escape(members[1])
                + ' final curves; showing the absolute (magnitude) sum only.</p>',
                tab=label,
            )
        for name in members:
            for landmark in landmark_figures:
                if landmark.get("tab") == name:
                    sections.append({**landmark, "tab": label})
        for section in sections[group_start:]:
            section["tab"] = label

    # Bass-management routing/headroom section. This is driven by the
    # route-level #14 schema, not the deprecated single matrix summary.
    bass_management = metadata.get("bass_management") or {}
    sections = overview
    if bass_management:
        _emit_html(sections, shared_section("bass_management_summary_html"))
        routing_fig = create_bass_management_routing_figure(data)
        headroom_fig = create_bass_management_headroom_figure(data)
        _emit_fig(sections, routing_fig)
        _emit_fig(sections, headroom_fig)
        _emit_html(sections, shared_section("bass_management_groups_table_html"))
        _emit_html(sections, shared_section("bass_management_sub_outputs_table_html"))

    # Reconstruct each logical input after bass management once. For a main
    # channel this is the crossover-aware complex sum of its high-passed main
    # output and its own low-passed route through the physical subwoofer.
    post_dsp_curves = build_post_dsp_source_curves(data)
    routed_unavailable = getattr(data, "native_routed_source_unavailability", None)
    if isinstance(routed_unavailable, dict) and routed_unavailable:
        details = "".join(
            "<li>" + escape(str(source)) + ": " + escape(str(reason)) + "</li>"
            for source, reason in sorted(routed_unavailable.items(), key=lambda item: str(item[0]))
        )
        _emit_html(
            overview,
            '<section class="epa-footer"><h3>Corrected routed response unavailable</h3>'
            "<p>Required evidence is missing; corrected response omitted for these "
            "logical sources.</p><ul>"
            + details + "</ul></section>",
        )
    bass_report = metadata.get("bass_management") or {}
    physical_sub = (
        bass_report.get("physical_sub_output")
        or bass_report.get("lfe_channel")
        or "LFE"
    )
    target_references = dict(post_dsp_curves)
    target_curves = build_target_overlay_curves(
        data, target_references, output_json_path
    )

    # Individual channel sections in tabs. Multi-driver channels (e.g.
    # two subwoofers on one LFE bus) expand into one tab per physical
    # driver so each subwoofer gets its own plots and filter details.
    # Individual channel sections in shell tabs (the shell builds the tab
    # bar from the section `tab` fields, so no tab buttons are emitted).
    tab_entries = display_channel_entries(data)

    for i, entry in enumerate(tab_entries):
        channel_name = entry["channel"]
        driver_index = entry["driver"]
        tab_label = entry["label"]
        tab_groups = {"speakers": [], "time_domain": [], "epa": []}
        channel_blocks.append({"label": tab_label, "groups": tab_groups})
        sections = tab_groups["speakers"]
        safe_label = escape(tab_label)
        channel_data = channels_dict[channel_name]
        is_driver_tab = driver_index is not None

        if is_driver_tab:
            # Physical sub output: its own measurement replayed through
            # its full deployed chain (shared EQ plus per-sub
            # gain/crossover/route).
            drivers = get_plottable_drivers(channel_data)
            driver = (
                drivers[driver_index]
                if 0 <= driver_index < len(drivers)
                else {}
            )
            chain = per_driver_chain_plugins(data, channel_name, driver_index) or []
            eq_source: dict = {"plugins": chain}
            # Raw measurements stop at the recorded band; the stored curve
            # continues past it for DSP replay and must not be drawn as data.
            initial_curve = clip_curve_to_measured_band(driver.get("initial_curve"), driver)
            native_handled, native_curve, native_reason = resolve_native_driver_curve(
                data, channel_name, driver
            )
            if native_handled:
                final_curve = native_curve
            else:
                final_curve = clip_curve_to_measured_band(
                    per_driver_corrected_curve(data, channel_name, driver_index), driver
                )
            passes = extract_eq_passes(eq_source)
            # The tab EQ plot appears only when EQ exists (mirroring the
            # legacy empty-EQ behavior); otherwise the tab keeps its
            # response plots without an EQ section.
            eq_response_view = (
                None
                if native_handled
                else (
                    per_driver_effective_eq(data, channel_name, driver_index)
                    if channel_has_eq(channel_data)
                    else None
                )
            )
            target_view = None
            lfe_plus_channel = None
            acoustic_data = driver.get("measured_acoustics") or {}
            ir_pre = acoustic_data.get("pre_ir") or driver.get("pre_ir")
            ir_post = driver.get("post_ir")
            if native_handled and native_reason:
                caption_html = (
                    f'<p class="epa-footer">Raw measurement only; corrected response '
                    f'unavailable for {escape(tab_label)}: {escape(native_reason)}.</p>\n'
                )
            elif native_handled:
                caption_html = (
                    f'<p class="epa-footer">Corrected physical-driver response for '
                    f'{escape(channel_name)} includes this driver after shared EQ, gain, '
                    "and crossover. Total driver EQ preview unavailable; "
                    "individual filter details remain below.</p>\n"
                )
            else:
                caption_html = (
                    f'<p class="epa-footer">Physical driver output of {escape(channel_name)}: '
                    "its measurement through this driver's own DSP chain "
                    "(shared EQ + driver gain/crossover/route). The EQ plot below "
                    "shows this total driver shaping.</p>\n"
                )
        else:
            acoustic_data = channel_data
            initial_curve = channel_data.get("initial_curve")
            if channel_data.get("drivers"):
                # A multi-driver aggregate initial is level-relative optimizer
                # state; the summed driver measurements are the acoustic baseline
                # matching the logical-input corrected curve.
                driver_baseline = sum_driver_initial_curves(channel_data)
                if driver_baseline is not None:
                    initial_curve = driver_baseline
            final_curve = _channel_display_final_curve(
                channel_name, physical_sub, channel_data, post_dsp_curves
            )
            eq_source = channel_data
            passes = extract_eq_passes(channel_data)
            eq_response_view = channel_data.get("eq_response")
            target_view = target_curves.get(channel_name)
            lfe_plus_channel = None
            if channel_name != physical_sub and _has_redirected_bass_route(data, channel_name):
                lfe_plus_channel = post_dsp_curves.get(channel_name)
            ir_pre = channel_data.get("pre_ir")
            ir_post = channel_data.get("post_ir")
            caption_html = ""

        # Extract EQ filters (grouped by pass for 3-pass pipeline)
        eq_filters = []
        for p in passes:
            eq_filters.extend(p["filters"])

        _emit_html(sections,
            f"<h2>Channel: {safe_label}</h2>\n{caption_html}",
            tab=tab_label,
        )

        # Full range plot
        fig_full = create_channel_figure(
            tab_label, initial_curve, final_curve, " (Full Range)",
            tab=tab_label,
        )
        add_channel_response_overlays(
            fig_full,
            tab_label,
            target_view,
            lfe_plus_channel,
        )
        _emit_fig(sections, fig_full)
        _emit_html(sections, landmarks_table_html({"channels": {tab_label: {
            **channel_data, "initial_curve": initial_curve}}}), tab=tab_label)

        # EQ response plot (uses per-pass breakdown when 3-pass labels are present)
        eq_sample_rate = _report_sample_rate(data)
        native_without_rate = (
            getattr(data, "native_dsp_replay_present", False) is True
            and eq_sample_rate is None
        )
        if native_without_rate:
            fig_eq = None
            _emit_html(
                sections,
                    '<p class="epa-footer">EQ response plot unavailable: no explicit '
                    "playback sample rate is available; the plot was omitted.</p>",
                tab=tab_label,
            )
        else:
            assert eq_sample_rate is not None
            fig_eq = create_multipass_eq_figure(
                tab_label, eq_source, eq_response_view,
                sample_rate=eq_sample_rate,
                tab=tab_label,
            )
            if fig_eq is None:
                fig_eq = create_eq_figure(
                    tab_label, eq_filters, eq_response_view,
                    sample_rate=eq_sample_rate,
                    tab=tab_label,
                )
        if fig_eq:
            fig_full["figure"]["y2"] = wasm_report.axis("EQ gain (dB)", "linear", -25.0, 25.0)
            for trace in fig_eq["figure"]["series"]:
                fig_full["figure"]["series"].append({**trace, "y_axis": 1})

        # IR waveform plot
        sections = tab_groups["time_domain"]
        if is_driver_tab and acoustic_data:
            rate = acoustic_data.get("sample_rate_hz")
            _emit_html(sections,
                f'<p>Independent measured driver IR: {escape(tab_label)}; '
                f'native sample rate {escape(str(rate))} Hz. This is not the summed '
                'parent-speaker response. No measured post-DSP capture is implied.</p>',
                tab=tab_label)
            if isinstance(rate, (int, float)) and rate <= 16000:
                _emit_html(sections,
                    '<p>1–8 kHz reflection analysis is unavailable at this native '
                    'sample rate. T60 bands beyond Nyquist are marked unavailable.</p>',
                    tab=tab_label)
        _emit_fig(sections, create_ir_figure(
            tab_label,
            ir_pre,
            ir_post,
            tab=tab_label,
        ))

        if not is_driver_tab or acoustic_data:
            _emit_html(sections, early_reflections_html(acoustic_data, tab_label),
                       tab=tab_label)
            _emit_fig(sections, early_reflection_figures(acoustic_data, tab_label, tab=tab_label))
            early_late = acoustic_data.get("early_late_curves")
            early_late_fig = create_early_late_figure(
                tab_label, early_late, tab=tab_label
            )
            if early_late_fig:
                _emit_fig(sections, early_late_fig)
                _emit_html(sections,
                    '<p class="epa-footer">Third-octave early and late energy contributions '
                    'share the full curve\'s peak-band reference. Full is an incoherent '
                    'energy sum, not a coherent pressure response.</p>\n',
                    tab=tab_label,
                )
            else:
                _emit_html(sections,
                    '<p class="epa-footer">Early vs late sound: pending '
                    'roomeq field early_late_curves with a shared level reference.</p>\n',
                    tab=tab_label,
                )

            # Channel tabs reuse the room-level T60 block above (its table
            # already carries per-speaker columns). Only driver tabs keep
            # their own T60: driver measured-acoustics fits exist nowhere else.
            if is_driver_tab:
                t60 = t60_rows(acoustic_data)
                t60_fig = create_t60_octaves_figure(tab_label, t60, tab=tab_label, itu_reference=decay_reference)
                _emit_html(sections, t60_table_html(acoustic_data), tab=tab_label)
                _emit_fig(sections, t60_fig)
                if t60_fig:
                    _emit_html(sections, t60_itu_note(decay_reference), tab=tab_label)
            waterfall_html = optimization_waterfall_html(
                acoustic_data.get("waterfall"), acoustic_data.get("resonance_decays"))
            if waterfall_html:
                _emit_html(sections, waterfall_html, tab=tab_label)
            else:
                _emit_html(sections,
                    '<p class="epa-footer">Waterfall and resonance decay: pending '
                    'roomeq fields waterfall and resonance_decays from a measured '
                    'room impulse response.</p>\n',
                    tab=tab_label,
                )
            wavelet_html = optimization_wavelet_html(acoustic_data.get("wavelet"))
            if wavelet_html:
                _emit_html(sections, wavelet_html, tab=tab_label)
            else:
                _emit_html(sections,
                    '<p class="epa-footer">Three-cycle wavelet: pending roomeq '
                    'field wavelet from a measured room impulse response.</p>\n',
                    tab=tab_label,
                )

        # EPA psychoacoustic scores (pre/post) for this channel
        sections = tab_groups["epa"]
        epa_per_channel = metadata.get("epa_per_channel") or {}
        epa_html = _epa_channel_table_html(epa_per_channel.get(channel_name))
        if epa_html:
            _emit_html(sections, epa_html, tab=tab_label)

        # Filter details (grouped by pass when 3-pass labels are present).
        sections = tab_groups["speakers"]
        # Driver tabs merge the driver EQ with the shared channel EQ, so
        # they render per-origin inner tabs instead of one flat list.
        has_labeled = any(p["label"] for p in passes)
        if is_driver_tab:
            _emit_html(sections,
                _driver_eq_filters_html(data, channel_name, driver_index, f"eq_{i}"),
                tab=tab_label,
            )
        elif has_labeled and passes:
            filter_html = _render_eq_section(
                "filter_section",
                {
                    "title": "EQ Filters (3-Pass Pipeline)",
                    "labeled": True,
                    "passes": _eq_pass_descriptors(passes),
                },
            )
            _emit_html(sections, filter_html, tab=tab_label)
        elif eq_filters:
            filter_html = _render_eq_section(
                "filter_section",
                {
                    "title": "EQ Filters",
                    "labeled": False,
                    "passes": _eq_pass_descriptors([{"filters": eq_filters}]),
                },
            )
            _emit_html(sections, filter_html, tab=tab_label)

        if is_driver_tab:
            shaping_html = _driver_shaping_summary_html(data, channel_name, driver_index)
            if shaping_html:
                _emit_html(sections, shaping_html, tab=tab_label)

    if capture_verification is not None:
        sections = why
        verified, reason, graph_identity = verify_payload_binding(data)
        capture_graph = (capture_verification.get("graph_id")
                         if isinstance(capture_verification, dict) else None)
        if not verified:
            _emit_html(sections, 
                '<section class="capture-views"><h1>Capture acceptance diagnostics</h1>'
                '<p>Unavailable: saved optimization graph binding is invalid: '
                + escape(reason) + '</p></section>\n'
            )
        elif capture_graph != graph_identity:
            _emit_html(sections, 
                '<section class="capture-views"><h1>Capture acceptance diagnostics</h1>'
                '<p>Unavailable: verification candidate graph does not match the saved '
                'optimization graph.</p></section>\n'
            )
        else:
            _emit_html(sections, capture_views_html(capture_verification))

    # The shared producer owns document layout, grouping, and unavailable states.
    flow_snapshot = shared_section("signal_flow_sections")
    if flow_snapshot is None:
        flow_snapshot = signal_flow_sections(data)
    return _report_assembly(
        page_title,
        status_sections,
        {"why": why, "overview": overview, "speakers": speakers,
         "timing": timing, "symmetric": symmetric, "time_domain": time_domain,
         "epa": epa, "flow": flow_snapshot},
        channel_blocks,
        _report_provenance([("", data)]),
        relative_levels_html=level_compensation_html(data),
        modes_html=resonance_summary_html(data),
    )


def create_html_report(
    data: dict,
    output_path: Path,
    output_json_path: Path | None = None,
    smoothed_octaves: float = 1.0,
    capture_verification: dict | None = None,
) -> None:
    """Write the shared report document using the canonical dashboard payload.

    Native consumers can use :func:`build_report_payload` and assemble the same
    payload with the shared Rust/WASM library. Dashboard calculations remain
    owned here until their upstream Rust replacements have numerical parity.
    """
    payload = build_report_payload(
        data, output_json_path, smoothed_octaves, capture_verification
    )
    wasm_report.write_report(output_path, payload["title"], payload)
    print(f"HTML report written to: {output_path}")


def create_comparison_html_report(
    mode_datasets: list[tuple[str, dict]],
    output_path: Path,
) -> None:
    """Create an HTML report comparing multiple processing modes.

    Args:
        mode_datasets: List of (mode_name, roomeq_output_data) tuples.
        output_path: Path to write HTML report.
    """
    # Compare what reaches the microphone, including each source channel's
    # redirected-bass route. Plotting an isolated high-passed main below its
    # crossover creates a false SPL collapse and makes different crossover
    # choices look like different RoomEQ magnitude targets.
    comparison_channels: dict[int, dict[str, dict]] = {}
    for _, data in mode_datasets:
        routed_curves = build_post_dsp_source_curves(data)
        comparison_channels[id(data)] = {
            name: {
                **channel,
                "final_curve": routed_curves.get(name, channel.get("final_curve")),
            }
            for name, channel in (data.get("channels", {}) or {}).items()
        }

    # Collect all channel names across modes (union)
    all_channel_names: set[str] = set()
    for _, data in mode_datasets:
        all_channel_names.update(comparison_channels[id(data)].keys())
    sorted_channels = sorted(all_channel_names, key=get_channel_sort_key)

    # If both L and R are present in at least one mode, append a
    # synthetic "L+R" tab whose curves are the complex (coherent) sum
    # of the per-mode L and R curves. The tab is only emitted when at
    # least one mode actually has both channels — otherwise the sum
    # would be undefined and the tab would be empty.
    has_lr_pair = any(
        "L" in comparison_channels[id(data)] and "R" in comparison_channels[id(data)]
        for _, data in mode_datasets
    )
    if has_lr_pair:
        sorted_channels.append(LR_SUM_CHANNEL)

    mode_names = [name for name, _ in mode_datasets]
    page_title = f"RoomEQ Mode Comparison: {', '.join(_mode_label(n) for n in mode_names)}"

    # Sections in document order; the shell renders them and builds the
    # per-channel tab bar from the section `tab` fields.
    sections: list[dict] = []

    for mode_name, data in mode_datasets:
        _emit_html(sections, _playback_status_html(data.get("metadata") or {}, mode_name, data=data),
                   flat=True)
        _emit_html(sections, capture_clock_qa_html(data))
        _emit_html(sections, capture_reflections_html(data))
        _emit_html(sections, correction_explanation_html(data, mode_name))
        _emit_html(sections, acceptance_views_html(data, mode_name))
        _emit_html(sections, waveform_status_html(data, mode_name))

    # --- Summary table ---
    summary_parts = ['<h2>Summary</h2>\n<table class="summary-table">\n<thead><tr>']
    has_gd_summary = any(
        (data.get("metadata") or {}).get("group_delay") for _, data in mode_datasets
    )
    has_auto_summary = any("_auto" in mode_name for mode_name, _ in mode_datasets)
    summary_parts.append(
        "<th>Mode</th><th>Loss</th>"
        '<th title="Flat loss before EQ (always computed via compute_flat_loss, regardless of which objective was minimized)">Pre flat-loss</th>'
        '<th title="Flat loss after EQ (always computed via compute_flat_loss, regardless of which objective was minimized)">Post flat-loss</th>'
        "<th>Improvement</th>"
        "<th>EPA Pref (pre)</th><th>EPA Pref (post)</th><th>EPA Δ</th>"
    )
    if has_auto_summary:
        summary_parts.append(
            '<th title="Emitted EQ sections per channel, including driver-local sections; '
            'Kautz banks count all basis sections and shared entries count once; '
            'ranges show min-max across channels">EQ sections</th>'
        )
    if has_gd_summary:
        summary_parts.append(
            '<th title="Group-delay optimization advisory">GD advisory</th>'
            '<th title="Whether GD controls were inserted into exported DSP">GD applied</th>'
            '<th title="Summed in-band GD RMS before and after GD optimization">GD RMS</th>'
            '<th title="GD RMS improvement, 20*log10(pre/post)">GD Δ</th>'
            '<th title="Total emitted all-pass filters; coh is mean in-band coherence">GD AP/coh</th>'
        )
    summary_parts.append("</tr></thead>\n<tbody>\n")

    mode_scores: list[tuple[str, float, float]] = []
    loss_types: list[str | None] = []
    for mode_name, data in mode_datasets:
        meta = data.get("metadata", {})
        pre = meta.get("pre_score", 0)
        post = meta.get("post_score", 0)
        improv = ((pre - post) / pre * 100) if pre > 0 else 0
        mode_scores.append((mode_name, pre, post))
        color = _mode_color(mode_name)

        loss_type = meta.get("loss_type")
        loss_types.append(loss_type)
        loss_cell = f"<td>{loss_type}</td>" if loss_type else '<td style="color:#999">-</td>'

        epa_pre_avg, epa_post_avg = _epa_summary_pref(meta)
        if epa_pre_avg is not None and epa_post_avg is not None:
            epa_delta = epa_post_avg - epa_pre_avg
            epa_color = "#2ecc71" if epa_delta >= 0 else "#e74c3c"
            epa_cells = (
                f"<td>{epa_pre_avg:.2f}</td><td>{epa_post_avg:.2f}</td>"
                f'<td style="color:{epa_color};font-weight:600">{epa_delta:+.2f}</td>'
            )
        else:
            epa_cells = '<td style="color:#999">-</td><td style="color:#999">-</td><td style="color:#999">-</td>'

        summary_parts.append(
            f'<tr><td style="color:{color};font-weight:600">{_mode_label(mode_name)}</td>'
            f"{loss_cell}"
            f"<td>{pre:.4f}</td><td>{post:.4f}</td>"
            f'<td class="improvement">{improv:.1f}%</td>'
            f"{epa_cells}"
            f"{_eq_filter_summary_html(data) if has_auto_summary else ''}"
            f"{_gd_summary_cells_html(meta) if has_gd_summary else ''}</tr>\n"
        )
    summary_parts.append("</tbody></table>\n")
    _emit_html(sections, "".join(summary_parts))

    # When the report mixes loss functions, clarify what the Pre/Post
    # flat-loss columns actually measure — readers might otherwise
    # assume an EPA-loss run's "score" reflects the EPA composite.
    distinct_losses = sorted({lt for lt in loss_types if lt})
    if len(distinct_losses) >= 2:
        _emit_html(sections, 
            '<p style="color:#555;font-size:0.9em;margin-top:10px;">'
            "ℹ This report mixes runs that minimized different loss "
            f"functions ({', '.join(distinct_losses)}). The "
            "<strong>Pre flat-loss</strong> and <strong>Post flat-loss</strong> "
            "columns are <em>always</em> computed via the flat-loss helper "
            "(<code>compute_flat_loss</code> over <code>[min_freq, max_freq]</code>), "
            "regardless of which objective each mode actually minimized — "
            "this keeps every mode on the same scale so the columns answer "
            "<em>“how flat is the response?”</em> for all modes. For "
            "<em>perceptual</em> outcomes across loss types, the "
            "<strong>EPA Pref</strong> columns are the right place to look."
            "</p>\n"
        )
    if has_gd_summary:
        _emit_html(sections, 
            '<p style="color:#555;font-size:0.9em;margin-top:10px;">'
            "GD columns come from <code>metadata.group_delay</code>. "
            "A non-success advisory is expected for recordings that do not "
            "carry coherence or independent sweep realisations; it means the "
            "production safety gates downgraded or skipped the requested GD path."
            "</p>\n"
        )
    if has_auto_summary:
        _emit_html(sections, 
            '<p style="color:#555;font-size:0.9em;margin-top:10px;">'
            "Auto columns show emitted EQ filter counts from the output JSON. "
            "Resolved automatic Q and gain bounds are logged by roomeq during the run "
            "but are not currently persisted in comparison JSON."
            "</p>\n"
        )

    # Score bar chart (passes loss_types so the chart can label/warn correctly)
    _emit_fig(sections, create_score_comparison_figure(mode_scores, loss_types))

    # Global design-target level anchor. Serialized absolute targets carry
    # the design shape but not the measured level, so match the main (L/R)
    # target shape to the measured per-channel mean of the L/R pair over
    # 100 Hz - 10 kHz once. The single offset applies to every channel tab,
    # preserving designed inter-channel target differences. Without an L/R
    # pair there is no anchor and targets render as serialized.
    _match_lo, _match_hi = TARGET_LEVEL_MATCH_BAND_HZ
    global_target_offset: float | None = None
    main_target_shape: dict | None = None
    for _, anchor_data in mode_datasets:
        anchor_channels = comparison_channels[id(anchor_data)]
        anchor_l = anchor_channels.get("L") or {}
        anchor_r = anchor_channels.get("R") or {}
        ref_l = anchor_l.get("final_curve") or anchor_l.get("initial_curve")
        ref_r = anchor_r.get("final_curve") or anchor_r.get("initial_curve")
        anchor_shape = anchor_l.get("target_curve") or anchor_r.get("target_curve")
        offset = global_target_offset_for_pair(
            ref_l, ref_r, anchor_shape, _match_lo, _match_hi
        )
        if offset is None:
            continue
        global_target_offset = offset
        main_target_shape = anchor_shape
        break

    # --- Per-channel tabs ---
    # Per-channel sections in shell tabs (tab bar built by the shell).
    epa_tables = _epa_comparison_tables({
        channel_name: _epa_comparison_entries(channel_name, mode_datasets)
        for channel_name in sorted_channels
    })
    for i, ch_name in enumerate(sorted_channels):
        is_lr = (ch_name == LR_SUM_CHANNEL)
        includes_redirected_bass = not is_lr and any(
            _has_redirected_bass_route(data, ch_name) for _, data in mode_datasets
        )
        source_label = _comparison_source_label(ch_name, includes_redirected_bass)
        if is_lr:
            _emit_html(sections,
                "<h2>Logical source L+R</h2>\n"
                "<p>Complex sums require valid per-microphone clock evidence and measured phase. "
                "Otherwise the plot uses a magnitude-only power sum, which is not a coherent pressure prediction.</p>\n",
                tab=ch_name,
            )
        else:
            _emit_html(sections, f"<h2>{source_label}</h2>\n", tab=ch_name)
            if includes_redirected_bass:
                _emit_html(sections,
                    '<p style="color:#666;font-size:0.9em;margin:-10px 0 15px 0;">'
                    "The response traces are the coherent microphone prediction for this "
                    "input source: its high-passed physical speaker output plus its "
                    "low-passed route through the physical subwoofer. They are not the "
                    f"isolated {ch_name} loudspeaker output."
                    "</p>\n",
                    tab=ch_name,
                )

        # Build mode_data for this channel. The synthetic L+R channel
        # is computed on the fly from each mode's L and R curves; all
        # other channels are read straight out of the JSON.
        mode_data: list[tuple[str, dict]] = []
        for mode_name, data in mode_datasets:
            channels = comparison_channels[id(data)]
            if is_lr:
                lr, fallback_reason = gated_lr_channel(data, channels.get("L"), channels.get("R"))
                if fallback_reason:
                    _emit_html(sections, f"<p><strong>{escape(str(mode_name))}: magnitude-only fallback.</strong> {escape(fallback_reason)}</p>\n", tab=ch_name)
                if lr:
                    mode_data.append((mode_name, lr))
            else:
                ch_data = channels.get(ch_name, {})
                if ch_data:
                    mode_data.append((mode_name, ch_data))

        if not mode_data:
            _emit_html(sections, "<p>No data for this channel.</p>\n", tab=ch_name)
            continue

        # Shared design target: first mode carrying a serialized absolute
        # target wins (compared modes normally share one fixture target).
        # No fallback flat line — it would misstate the design slope.
        # The synthetic L+R channel carries no target of its own, so it
        # reuses the main (L/R) target shape interpolated onto its grid.
        # Every tab's target is then shifted by the global L+R level anchor
        # so the drawn target mean matches the measured midband level.
        comparison_target = None
        if is_lr and main_target_shape is not None:
            lr_grid = None
            for _, ch_data in mode_data:
                lr_grid = (ch_data or {}).get("final_curve") or (ch_data or {}).get(
                    "initial_curve"
                )
                if lr_grid:
                    break
            if lr_grid is not None:
                comparison_target = shift_target_to_reference_band_mean(
                    main_target_shape, lr_grid, _match_lo, _match_hi
                )
        else:
            for _, ch_data in mode_data:
                candidate = (ch_data or {}).get("target_curve")
                if (
                    isinstance(candidate, dict)
                    and candidate.get("freq")
                    and candidate.get("spl")
                ):
                    comparison_target = {
                        "freq": list(candidate["freq"]),
                        "spl": list(candidate["spl"]),
                    }
                    break
        if (
            comparison_target is not None
            and not is_lr
            and global_target_offset is not None
        ):
            comparison_target = {
                "freq": list(comparison_target["freq"]),
                "spl": [
                    level + global_target_offset
                    for level in comparison_target["spl"]
                ],
            }

        # 1. Overlay plot (full range) + Zoomed (bass)
        with ComparisonFigureBatch() as figure_batch:
            comparison_figures = [
                create_comparison_overlay_figure(
                    source_label, mode_data, target_curve=comparison_target,
                    tab=ch_name,
                ),
                create_comparison_zoomed_figure(
                    source_label, mode_data, tab=ch_name
                ),
                # Phase, delay, and impulse response data are prepared by
                # their existing numeric routines before schema assembly.
                create_comparison_phase_figure(ch_name, mode_data, tab=ch_name),
                create_comparison_group_delay_figure(ch_name, mode_data, tab=ch_name),
                create_comparison_ir_figure(ch_name, mode_data, tab=ch_name),
                create_mode_subplots_figure(
                    ch_name, mode_data, target_curve=comparison_target, tab=ch_name
                ),
                create_comparison_eq_overlay_figure(
                    ch_name,
                    mode_data,
                    sample_rates={
                        name: float(output.get("sample_rate", 48_000.0))
                        for name, output in mode_datasets
                    },
                    tab=ch_name,
                ),
            ]
        comparison_figures = figure_batch.resolve(comparison_figures)
        for comparison_figure in comparison_figures:
            _emit_fig(sections, comparison_figure)

        # 7. EPA psychoacoustic scores per mode
        epa_html = epa_tables.get(ch_name, "")
        if epa_html:
            _emit_html(sections, epa_html, tab=ch_name)

    payload = wasm_report.payload(page_title, sections,
                                  _report_provenance(mode_datasets))
    wasm_report.write_report(output_path, page_title, payload)

    print(f"Comparison report written to: {output_path}")
