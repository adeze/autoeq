"""Schema section builders for roomeq visualization.

Ported from Plotly: the same data logic, colors, titles, and default
smoothing, emitting ``autoeq-report-data-v1`` render-only section dicts
(see ``crates/autoeq-report-wasm/SCHEMA.md``) assembled by
:mod:`scripts.src.wasm_report`. Plotly-only interactivity (smoothing
dropdowns, Linear/dB toggles, subplot grids, hover customdata, marker
symbols) is folded into static equivalents documented per builder.
"""

import math

from . import DEFAULT_SMOOTHING
from .dsp import (
    smooth_octave,
    compute_eq_response,
    compute_group_delay,
    compute_group_delay_from_ir,
    generate_freq_points,
    build_post_dsp_source_curves,
    per_driver_effective_eq,
    wrap_phase,
)
from .data_extract import (
    channel_has_eq,
    clip_curve_to_measured_band,
    compute_y_range,
    compute_average_spl_in_range,
    driver_display_names,
    get_all_crossover_frequencies,
    get_channel_sort_key,
    get_plottable_drivers,
)
from .target_overlay import build_target_overlay_curves
from .native_bass_figures import (
    headroom_figure as _native_bass_headroom_figure,
    routing_figure as _native_bass_routing_figure,
)
from .native_eq_figures import create_eq_figure, create_multipass_eq_figure
from .native_comparison_tables import score_comparison_figure as _native_score_comparison_figure
from .native_comparison_figures import (
    ComparisonFigureBatch,
    comparison_figure_section as _native_comparison_figure_section,
)
from .native_combined_figures import combined_figures as _native_combined_figures
from .native_response_figures import (
    add_channel_response_overlays as _native_add_channel_response_overlays,
    create_channel_figure as _native_create_channel_figure,
    create_smoothed_figure as _native_create_smoothed_figure,
    create_zoomed_figure as _native_create_zoomed_figure,
)
from .native_temporal_figures import (
    create_early_late_figure,
    create_ir_figure,
    create_symmetric_pair_figure,
    create_t60_octaves_figure,
    create_tof_figure,
    finite_or_gap as _finite_or_gap,
    to_db as _to_db,
)
from .wasm_report import (
    axis,
    series,
    hline,
    vline,
    figure,
)


def _comparison_figure(
    title,
    x,
    y,
    series_list=None,
    hlines=None,
    vlines=None,
    xranges=None,
    annotations=None,
    legend=True,
    y2=None,
    tab=None,
):
    """Prepare a comparison plot descriptor for canonical schema assembly."""
    return _native_comparison_figure_section({
        "title": title,
        "x": x,
        "y": y,
        "y2": y2,
        "series": list(series_list or []),
        "hlines": list(hlines or []),
        "vlines": list(vlines or []),
        "xranges": list(xranges or []),
        "annotations": list(annotations or []),
        "legend": bool(legend),
        "tab": tab,
    })


def _comparison_series(name, x, y, color=None, width=2.0, dash="solid"):
    """Prepare trace arrays without applying DSP or final schema defaults."""
    return {
        "name": name,
        "x": list(x),
        "y": list(y),
        "color": color,
        "width": width,
        "dash": dash,
    }

def create_channel_figure(
    channel_name: str,
    initial_curve: dict | None,
    final_curve: dict | None,
    title_suffix: str = "",
    tab=None,
):
    """Build a channel Before/After EQ section (default smoothing only)."""
    return _native_create_channel_figure(
        channel_name, initial_curve, final_curve, title_suffix, tab
    )


def add_channel_response_overlays(
    fig,
    channel_name: str,
    target_curve: dict | None,
    lfe_plus_channel_curve: dict | None,
):
    """Append Target/LFE series to a channel figure section dict."""
    return _native_add_channel_response_overlays(
        fig, channel_name, target_curve, lfe_plus_channel_curve
    )


def create_zoomed_figure(
    channel_name: str,
    initial_curve: dict | None,
    final_curve: dict | None,
    min_freq: float = 20.0,
    max_freq: float = 1200.0,
    y_range: float = 25.0,
    tab=None,
):
    """Build a zoomed channel section (20-1200Hz, 50 dB centered y-axis)."""
    return _native_create_zoomed_figure(
        channel_name, initial_curve, final_curve, min_freq, max_freq, y_range, tab
    )


def _get_driver_initial_curves(channel_data: dict) -> list[tuple[str, dict]] | None:
    """Extract per-driver initial curves from a channel's driver chains.

    Returns:
        List of (driver_name, curve_data) tuples, or None if no per-driver curves exist.
        Each curve_data has "freq" and "spl" keys.
    """
    drivers = channel_data.get("drivers", [])
    if not drivers:
        return None

    result = []
    for driver in drivers:
        initial_curve = driver.get("initial_curve")
        if initial_curve and "freq" in initial_curve and "spl" in initial_curve:
            name = driver.get("name", f"driver_{driver.get('index', '?')}")
            # Raw measurements stop at the recorded band; the stored curve
            # continues past it for DSP replay and must not be drawn as data.
            initial_curve = clip_curve_to_measured_band(initial_curve, driver)
            result.append((name, initial_curve))

    return result if result else None


def _combined_panel_data(data: dict, json_path=None) -> dict:
    """Collect the shared per-channel curves for the combined overview.

    Returns channel order and prepared numeric traces for Rust presentation.
    """
    channels_dict = data.get("channels", {})

    # Generate frequency points for EQ response
    freq_points = generate_freq_points(20.0, 20000.0, 500)

    # Collect all curves for y-range computation
    all_initial_curves: list[dict | None] = []
    all_corrected_curves: list[dict | None] = []

    # Per-driver data for channels that are speaker groups
    per_driver_initial: dict[str, list[tuple[str, dict]]] = {}

    post_dsp_curves = build_post_dsp_source_curves(data)
    target_curves = build_target_overlay_curves(data, post_dsp_curves, json_path)

    # Sort channels by classical order
    sorted_channel_names = sorted(channels_dict.keys(), key=get_channel_sort_key)
    channels = [(name, channels_dict[name]) for name in sorted_channel_names]

    for channel_name, channel_data in channels:
        driver_curves = _get_driver_initial_curves(channel_data)
        if driver_curves:
            per_driver_initial[channel_name] = driver_curves
            for _, dcurve in driver_curves:
                all_initial_curves.append(dcurve)
        else:
            all_initial_curves.append(channel_data.get("initial_curve"))

        final_curve = post_dsp_curves.get(channel_name)
        if final_curve and "freq" in final_curve and "spl" in final_curve:
            all_corrected_curves.append(final_curve)
        target_curve = target_curves.get(channel_name)
        if target_curve and "freq" in target_curve and "spl" in target_curve:
            all_corrected_curves.append(target_curve)

    # Compute y-ranges (shared by the Before/Corrected panels)
    spl_y_min, spl_y_max = compute_y_range(
        all_initial_curves + all_corrected_curves
    )

    # EQ traces, one per channel (multi-driver channels expand into one
    # effective per-driver shaping curve). Entries are (trace name, freq,
    # spl, color index, driver index or None).
    eq_row_traces: list[tuple[str, list, list, int, int | None]] = []
    for channel_index, (channel_name, channel_data) in enumerate(channels):
        drivers = get_plottable_drivers(channel_data)
        if drivers and getattr(data, "native_dsp_replay_present", False):
            # Native driver tabs explain the unavailable electrical preview.
            continue
        if drivers and channel_has_eq(channel_data):
            labels = driver_display_names(data, channel_name)
            expanded = False
            for driver_index in range(len(drivers)):
                effective = per_driver_effective_eq(data, channel_name, driver_index)
                if (
                    effective
                    and effective.get("freq")
                    and effective.get("spl")
                ):
                    if driver_index < len(labels):
                        label = labels[driver_index]
                    else:
                        label = f"{channel_name}/{driver_index}"
                    eq_row_traces.append(
                        (
                            f"EQ: {label}",
                            effective["freq"],
                            effective["spl"],
                            channel_index,
                            driver_index,
                        )
                    )
                    expanded = True
            if expanded:
                continue
        eq_response_data = channel_data.get("eq_response")
        if (
            eq_response_data
            and "freq" in eq_response_data
            and "spl" in eq_response_data
        ):
            eq_row_traces.append(
                (
                    f"EQ: {channel_name}",
                    eq_response_data["freq"],
                    eq_response_data["spl"],
                    channel_index,
                    None,
                )
            )
        else:
            plugins = channel_data.get("plugins", [])
            eq_filters = []
            for plugin in plugins:
                if plugin.get("plugin_type") == "eq":
                    filters = plugin.get("parameters", {}).get("filters", [])
                    eq_filters.extend(filters)
            if eq_filters:
                eq_resp = compute_eq_response(eq_filters, freq_points)
                eq_row_traces.append(
                    (
                        f"EQ: {channel_name}",
                        freq_points,
                        eq_resp,
                        channel_index,
                        None,
                    )
                )

    # Compute EQ y-range
    all_eq_values: list[float] = []
    for _, _, eq_spl_values, _, _ in eq_row_traces:
        all_eq_values.extend(eq_spl_values)

    if all_eq_values:
        eq_y_upper = min(20, math.ceil(max(all_eq_values) / 5) * 5 + 5)
        eq_y_lower = max(-20, math.floor(min(all_eq_values) / 5) * 5 - 5)
    else:
        eq_y_upper = 15
        eq_y_lower = -15

    # Crossover positions remain numerical input; Rust builds the marks.
    crossover_freqs = get_all_crossover_frequencies(data)

    return {
        "channels": channels,
        "freq_points": freq_points,
        "per_driver_initial": per_driver_initial,
        "post_dsp_curves": post_dsp_curves,
        "target_curves": target_curves,
        "eq_row_traces": eq_row_traces,
        "spl_y_min": spl_y_min,
        "spl_y_max": spl_y_max,
        "eq_y_lower": eq_y_lower,
        "eq_y_upper": eq_y_upper,
        "crossovers": crossover_freqs,
    }


def create_combined_figure(data: dict, json_path=None, tab=None) -> list:
    """Build the three combined overview panels from prepared response data."""
    channels_dict = data.get("channels", {})
    if not channels_dict:
        return _native_combined_figures({"has_channels": False}, tab)

    panels = _combined_panel_data(data, json_path)
    channels = panels["channels"]
    per_driver_initial = panels["per_driver_initial"]
    post_dsp_curves = panels["post_dsp_curves"]
    target_curves = panels["target_curves"]
    prepared = {
        "has_channels": True,
        "spl_y_min": panels["spl_y_min"],
        "spl_y_max": panels["spl_y_max"],
        "eq_y_lower": panels["eq_y_lower"],
        "eq_y_upper": panels["eq_y_upper"],
        "eq_frequency_extent": [panels["freq_points"][0], panels["freq_points"][-1]],
        "crossovers": panels["crossovers"],
        "before_series": [],
        "eq_series": [],
        "corrected_series": [],
    }

    def add_series(role, name, curve, color_index, *, dash="solid", width=2.0, smooth=False):
        if not curve or "freq" not in curve or "spl" not in curve:
            return
        spl = (
            smooth_octave(curve["freq"], curve["spl"], DEFAULT_SMOOTHING)
            if smooth
            else curve["spl"]
        )
        prepared[role].append({
            "name": name,
            "x": curve["freq"],
            "y": spl,
            "color_index": color_index % 10,
            "dash": dash,
            "width": width,
        })

    for channel_index, (channel_name, channel_data) in enumerate(channels):
        if channel_name in per_driver_initial:
            for driver_name, curve in per_driver_initial[channel_name]:
                add_series(
                    "before_series",
                    f"{channel_name}/{driver_name} (before)",
                    curve,
                    channel_index,
                    smooth=True,
                )
        else:
            add_series(
                "before_series",
                f"{channel_name} (before)",
                channel_data.get("initial_curve"),
                channel_index,
                smooth=True,
            )
        target = target_curves.get(channel_name)
        add_series(
            "before_series", f"{channel_name} target", target, channel_index,
            dash="dot",
        )

        final_curve = post_dsp_curves.get(channel_name)
        add_series(
            "corrected_series", f"{channel_name} (corrected)", final_curve,
            channel_index, smooth=True,
        )
        add_series(
            "corrected_series", f"{channel_name} target", target, channel_index,
            dash="dot",
        )

    for trace_name, eq_freq, eq_spl, color_index, _driver_index in panels["eq_row_traces"]:
        prepared["eq_series"].append({
            "name": trace_name,
            "x": eq_freq,
            "y": eq_spl,
            "color_index": color_index % 10,
            "dash": "solid",
            "width": 2.0,
        })
    return _native_combined_figures(prepared, tab)


def _driver_low_pass(driver: dict) -> tuple[float | None, str | None]:
    """Extract the deployed per-driver low-pass (LP_i) of a sub driver, if any.

    Reads the last low-pass ``crossover`` plugin in the driver chain, matching
    serial DSP order. Returns ``(frequency_hz, crossover_type)`` or
    ``(None, None)`` when the driver carries no low-pass (legacy
    single-crossover chains).
    """
    frequency: float | None = None
    crossover_type: str | None = None
    for plugin in driver.get("plugins", []) or []:
        if not isinstance(plugin, dict):
            continue
        if str(plugin.get("plugin_type", "")).lower() != "crossover":
            continue
        params = plugin.get("parameters", {}) or {}
        if str(params.get("output", "")).lower() not in ("low", "lowpass", "lp"):
            continue
        try:
            candidate = float(params.get("frequency", 0.0))
        except (TypeError, ValueError):
            continue
        if candidate > 0.0:
            frequency = candidate
            raw_type = params.get("type")
            crossover_type = str(raw_type) if raw_type is not None else None
    return frequency, crossover_type


def create_bass_management_routing_figure(data: dict, tab=None):
    """Build the shared renderer-neutral bass-management routing graph."""
    return _native_bass_routing_figure(data, tab)


def create_bass_management_headroom_figure(data: dict, tab=None):
    """Build the shared renderer-neutral bass-bus headroom chart."""
    return _native_bass_headroom_figure(data, tab)
# ============================================================================
# Mode color/label tables (copied verbatim from figures.py)
# ============================================================================

# Color scheme: distinct colours from the matplotlib tab20 family so
# each processing/loss/auto/GD scenario stands out on busy overlay plots.
MODE_COLORS: dict[str, str] = {
    "iir": "#1f77b4",              # blue
    "iir_epa": "#ff7f0e",          # orange
    "fir": "#2ca02c",              # green
    "fir_epa": "#d62728",          # red
    "hybrid": "#9467bd",           # purple
    "hybrid_epa": "#8c564b",       # brown
    "mixed_phase": "#e377c2",      # pink
    "mixed_phase_epa": "#17becf",  # cyan
    "iir_auto_filters": "#393b79",       # indigo
    "iir_auto_bounds": "#637939",        # olive green
    "iir_auto_all": "#8c6d31",           # ochre
    "mixed_phase_auto_all": "#843c39",   # maroon
    "iir_gd_safety_gate": "#ad494a",     # muted red
    "iir_gd_delay_only": "#bcbd22",        # olive
    "iir_gd_fixed_allpass": "#7f7f7f",     # gray
    "iir_gd_adaptive_allpass": "#1f9e89",  # teal
    "fir_gd_phase_linear": "#ff9896",      # salmon
    "mixed_phase_gd": "#c5b0d5",           # lavender
}

# Display names: every mode includes its loss function in the label so
# legends, summary tables, and per-mode subplot titles always show
# which objective the run minimized at a glance.
MODE_DISPLAY_NAMES: dict[str, str] = {
    "iir": "IIR (flat)",
    "iir_epa": "IIR (EPA)",
    "fir": "FIR (flat)",
    "fir_epa": "FIR (EPA)",
    "hybrid": "Hybrid (flat)",
    "hybrid_epa": "Hybrid (EPA)",
    "mixed_phase": "MixedPhase (flat)",
    "mixed_phase_epa": "MixedPhase (EPA)",
    "iir_auto_filters": "IIR auto filters",
    "iir_auto_bounds": "IIR auto bounds",
    "iir_auto_all": "IIR auto all",
    "mixed_phase_auto_all": "MixedPhase auto all",
    "iir_gd_safety_gate": "IIR GD safety gate",
    "iir_gd_delay_only": "IIR GD delay-only",
    "iir_gd_fixed_allpass": "IIR GD fixed AP",
    "iir_gd_adaptive_allpass": "IIR GD adaptive AP",
    "fir_gd_phase_linear": "FIR GD target",
    "mixed_phase_gd": "MixedPhase + GD",
}
def _mode_color(mode_name: str) -> str:
    return MODE_COLORS.get(mode_name, "#888888")


def _mode_label(mode_name: str) -> str:
    return MODE_DISPLAY_NAMES.get(mode_name, mode_name)


def create_comparison_overlay_figure(
    channel_name: str,
    mode_data: list[tuple[str, dict]],
    title_suffix: str = "",
    target_curve: dict | None = None,
    tab=None,
):
    """Overlay final curves from multiple modes on the same plot.

    ``target_curve`` is the shared design target (freq/spl); compared modes
    normally share one fixture target. When absent no target is drawn —
    a fabricated flat line would misstate the design slope.
    """
    series_list = []

    initial_curve = None
    for _, ch_data in mode_data:
        initial_curve = ch_data.get("initial_curve")
        if initial_curve:
            break

    all_curves: list[dict | None] = [initial_curve]

    if initial_curve:
        spl_smoothed = smooth_octave(
            initial_curve["freq"], initial_curve["spl"], DEFAULT_SMOOTHING
        )
        series_list.append(_comparison_series(
            "Before EQ",
            initial_curve["freq"], spl_smoothed,
            color="rgba(200, 200, 200, 0.6)", width=2,
        ))

    for mode_name, ch_data in mode_data:
        final_curve = ch_data.get("final_curve")
        all_curves.append(final_curve)
        if final_curve:
            spl_smoothed = smooth_octave(
                final_curve["freq"], final_curve["spl"], DEFAULT_SMOOTHING
            )
            series_list.append(_comparison_series(
                _mode_label(mode_name),
                final_curve["freq"], spl_smoothed,
                color=_mode_color(mode_name), width=2,
            ))

    if target_curve and target_curve.get("freq") and target_curve.get("spl"):
        series_list.append(_comparison_series(
            "Target",
            target_curve["freq"], target_curve["spl"],
            color="rgba(40, 40, 40, 0.9)", width=2, dash="dot",
        ))
        all_curves.append(target_curve)

    y_min, y_max = compute_y_range(all_curves)

    return _comparison_figure(
        f"{channel_name}: Mode Comparison{title_suffix}",
        axis("Frequency (Hz)", "log", 20, 20000),
        axis("SPL (dB)", "linear", y_min, y_max),
        series_list=series_list,
        tab=tab,
    )


def create_comparison_zoomed_figure(
    channel_name: str,
    mode_data: list[tuple[str, dict]],
    min_freq: float = 20.0,
    max_freq: float = 500.0,
    y_half_range: float = 25.0,
    tab=None,
):
    """Zoomed overlay of final curves (bass region, 50 dB span like all SPL plots)."""
    series_list = []

    initial_curve = None
    for _, ch_data in mode_data:
        initial_curve = ch_data.get("initial_curve")
        if initial_curve:
            break

    ref_curve = None
    for _, ch_data in mode_data:
        ref_curve = ch_data.get("final_curve")
        if ref_curve:
            break
    if ref_curve is None:
        ref_curve = initial_curve

    avg_spl = compute_average_spl_in_range(ref_curve, min_freq, max_freq) if ref_curve else 0.0

    if initial_curve:
        spl_smoothed = smooth_octave(initial_curve["freq"], initial_curve["spl"], DEFAULT_SMOOTHING)
        series_list.append(_comparison_series(
            "Before EQ",
            initial_curve["freq"], spl_smoothed,
            color="rgba(200, 200, 200, 0.6)", width=2,
        ))

    for mode_name, ch_data in mode_data:
        final_curve = ch_data.get("final_curve")
        if final_curve:
            spl_smoothed = smooth_octave(final_curve["freq"], final_curve["spl"], DEFAULT_SMOOTHING)
            series_list.append(_comparison_series(
                _mode_label(mode_name),
                final_curve["freq"], spl_smoothed,
                color=_mode_color(mode_name), width=2,
            ))

    return _comparison_figure(
        f"{channel_name}: Bass ({int(min_freq)}-{int(max_freq)} Hz)",
        axis("Frequency (Hz)", "log", min_freq, max_freq),
        axis("SPL (dB)", "linear",
             avg_spl - y_half_range, avg_spl + y_half_range),
        series_list=series_list,
        tab=tab,
    )


def create_comparison_eq_overlay_figure(
    channel_name: str,
    mode_data: list[tuple[str, dict]],
    *,
    sample_rates: dict[str, float] | None = None,
    tab=None,
):
    """Overlay EQ response curves from multiple modes."""
    series_list = []
    has_data = False
    freq_points = generate_freq_points(20.0, 20000.0, 500)

    for mode_name, ch_data in mode_data:
        eq_response_data = ch_data.get("eq_response")
        if eq_response_data and "freq" in eq_response_data and "spl" in eq_response_data:
            eq_freq = eq_response_data["freq"]
            eq_spl = eq_response_data["spl"]
        else:
            plugins = ch_data.get("plugins", [])
            eq_filters = []
            for plugin in plugins:
                if plugin.get("plugin_type") == "eq":
                    filters = plugin.get("parameters", {}).get("filters", [])
                    eq_filters.extend(filters)
            sample_rate = float((sample_rates or {}).get(mode_name, 48_000.0))
            if not math.isfinite(sample_rate) or sample_rate <= 40.0:
                raise ValueError("EQ comparison sample rate must support frequencies above 20 Hz")
            eq_freq = [min(f, sample_rate / 2) for f in generate_freq_points(
                20.0, min(20000.0, sample_rate / 2), 500)] if eq_filters else None
            eq_spl = compute_eq_response(eq_filters, eq_freq, sample_rate) if eq_filters else None

        if eq_spl:
            has_data = True
            series_list.append(_comparison_series(
                _mode_label(mode_name),
                eq_freq, eq_spl,
                color=_mode_color(mode_name), width=2,
            ))

    if not has_data:
        return None

    series_list.append(_comparison_series(
        "0 dB",
        [freq_points[0], freq_points[-1]], [0, 0],
        color="rgba(150, 150, 150, 0.5)", width=1, dash="dash",
    ))

    return _comparison_figure(
        f"EQ Response Comparison: {channel_name}",
        axis("Frequency (Hz)", "log", 20, 20000),
        axis("Gain (dB)", "linear", -15, 15),
        series_list=series_list,
        tab=tab,
    )


def create_mode_subplots_figure(
    channel_name: str,
    mode_data: list[tuple[str, dict]],
    target_curve: dict | None = None,
    tab=None,
):
    """One before/after figure section per mode (subplot grid becomes a list).

    ``target_curve`` is drawn dotted in every cell when given; otherwise no
    reference line is drawn (a flat placeholder would misstate the slope).
    """
    all_curves: list[dict | None] = []
    for _, ch_data in mode_data:
        all_curves.append(ch_data.get("initial_curve"))
        all_curves.append(ch_data.get("final_curve"))
    if target_curve:
        all_curves.append(target_curve)
    y_min, y_max = compute_y_range(all_curves)

    sections = []
    for mode_name, ch_data in mode_data:
        initial_curve = ch_data.get("initial_curve")
        final_curve = ch_data.get("final_curve")
        color = _mode_color(mode_name)
        cell_series = []

        if initial_curve:
            spl_sm = smooth_octave(initial_curve["freq"], initial_curve["spl"], DEFAULT_SMOOTHING)
            cell_series.append(_comparison_series(
                "Before EQ",
                initial_curve["freq"], spl_sm,
                color="rgba(200, 200, 200, 0.6)", width=1.5,
            ))

        if final_curve:
            spl_sm = smooth_octave(final_curve["freq"], final_curve["spl"], DEFAULT_SMOOTHING)
            cell_series.append(_comparison_series(
                _mode_label(mode_name),
                final_curve["freq"], spl_sm,
                color=color, width=2,
            ))

        if target_curve and target_curve.get("freq") and target_curve.get("spl"):
            cell_series.append(_comparison_series(
                "Target",
                target_curve["freq"], target_curve["spl"],
                color="rgba(40, 40, 40, 0.9)", width=1.5, dash="dot",
            ))

        sections.append(_comparison_figure(
            f"{_mode_label(mode_name)}: {channel_name} Per-Mode Detail",
            axis("Frequency (Hz)", "log", 20, 20000),
            axis("SPL (dB)", "linear", y_min, y_max),
            series_list=cell_series,
            tab=tab,
        ))
    return sections
def create_comparison_phase_figure(
    channel_name: str,
    mode_data: list[tuple[str, dict]],
    tab=None,
) -> list | None:
    """One figure section per mode showing phase before/after."""
    has_phase = False
    for _, ch_data in mode_data:
        for key in ("initial_curve", "final_curve"):
            curve = ch_data.get(key)
            if curve and curve.get("phase"):
                has_phase = True
                break
        if has_phase:
            break
    if not has_phase:
        return None

    sections = []
    for mode_name, ch_data in mode_data:
        initial_curve = ch_data.get("initial_curve")
        final_curve = ch_data.get("final_curve")
        color = _mode_color(mode_name)
        s_list = []
        if initial_curve and initial_curve.get("phase"):
            phase_sm = wrap_phase(initial_curve["phase"])
            s_list.append(_comparison_series(
                "Before EQ", initial_curve["freq"], phase_sm,
                color="rgba(200, 200, 200, 0.6)", width=1.5,
            ))
        if final_curve and final_curve.get("phase"):
            phase_sm = wrap_phase(final_curve["phase"])
            s_list.append(_comparison_series(
                _mode_label(mode_name), final_curve["freq"], phase_sm,
                color=color, width=2.0,
            ))
        sections.append(_comparison_figure(
            f"{channel_name}: Phase Before / After EQ — "
            f"{_mode_label(mode_name)}",
            axis("Frequency (Hz)", "log", 20, 20000),
            axis("Phase (°)", "linear", None, None),
            series_list=s_list, tab=tab,
        ))
    return sections


def create_comparison_group_delay_figure(
    channel_name: str,
    mode_data: list[tuple[str, dict]],
    tab=None,
) -> list | None:
    """One figure section per mode showing group delay before/after.

    Group delay is computed from unwrapped phase: GD = -d(phase)/d(omega).
    """
    has_phase = False
    for _, ch_data in mode_data:
        if ch_data.get("pre_ir") or ch_data.get("post_ir"):
            has_phase = True
            break
        for key in ("initial_curve", "final_curve"):
            curve = ch_data.get(key)
            if curve and curve.get("phase"):
                has_phase = True
                break
        if has_phase:
            break
    if not has_phase:
        return None

    # First pass: compute smoothed GD traces and a shared y-range from
    # all GD data (2nd/98th percentiles plus a 15% margin, as before).
    per_mode: list[tuple[str, str, list | None, list | None,
                          list | None, list | None]] = []
    all_gd: list[list[float]] = []
    for mode_name, ch_data in mode_data:
        initial_curve = ch_data.get("initial_curve")
        final_curve = ch_data.get("final_curve")
        before_freq: list | None = None
        before_gd: list | None = None
        after_freq: list | None = None
        after_gd: list | None = None
        if ch_data.get("pre_ir") or (initial_curve and initial_curve.get("phase")):
            gd_freq, gd_ms = compute_group_delay_from_ir(ch_data.get("pre_ir"))
            if not gd_freq and initial_curve and initial_curve.get("phase"):
                gd_freq, gd_ms = compute_group_delay(
                    initial_curve["freq"], initial_curve["phase"]
                )
            if gd_freq:
                before_freq = list(gd_freq)
                before_gd = smooth_octave(gd_freq, gd_ms, 1.0 / 3.0)
                all_gd.append(before_gd)
        if ch_data.get("post_ir") or (final_curve and final_curve.get("phase")):
            gd_freq, gd_ms = compute_group_delay_from_ir(ch_data.get("post_ir"))
            if not gd_freq and final_curve and final_curve.get("phase"):
                gd_freq, gd_ms = compute_group_delay(
                    final_curve["freq"], final_curve["phase"]
                )
            if gd_freq:
                after_freq = list(gd_freq)
                after_gd = smooth_octave(gd_freq, gd_ms, 1.0 / 3.0)
                all_gd.append(after_gd)
        per_mode.append((mode_name, _mode_color(mode_name),
                         before_freq, before_gd, after_freq, after_gd))

    y_lo: float | None = None
    y_hi: float | None = None
    if all_gd:
        flat = [v for gd in all_gd for v in gd]
        sorted_vals = sorted(flat)
        n = len(sorted_vals)
        lo = sorted_vals[max(0, int(n * 0.02))]
        hi = sorted_vals[min(n - 1, int(n * 0.98))]
        margin = (hi - lo) * 0.15
        y_lo = lo - margin
        y_hi = hi + margin

    sections = []
    for mode_name, color, before_freq, before_gd, after_freq, after_gd in per_mode:
        s_list = []
        if before_freq:
            s_list.append(_comparison_series(
                "Before EQ", before_freq, before_gd,
                color="rgba(200, 200, 200, 0.6)", width=1.5,
            ))
        if after_freq:
            s_list.append(_comparison_series(
                _mode_label(mode_name), after_freq, after_gd,
                color=color, width=2.0,
            ))
        sections.append(_comparison_figure(
            f"{channel_name}: Group Delay Before / After EQ — "
            f"{_mode_label(mode_name)}",
            axis("Frequency (Hz)", "log", 20, 20000),
            axis("Group Delay (ms)", "linear", y_lo, y_hi),
            series_list=s_list, tab=tab,
        ))
    return sections


def create_comparison_ir_figure(
    channel_name: str,
    mode_data: list[tuple[str, dict]],
    display_ms: float = 100.0,
    tab=None,
) -> list | None:
    """One figure section per mode showing impulse response before/after."""
    has_ir = False
    for _, ch_data in mode_data:
        if ch_data.get("pre_ir") or ch_data.get("post_ir"):
            has_ir = True
            break
    if not has_ir:
        return None

    sections = []
    for mode_name, ch_data in mode_data:
        pre_ir = ch_data.get("pre_ir")
        post_ir = ch_data.get("post_ir")
        color = _mode_color(mode_name)
        s_list = []
        if pre_ir:
            s_list.append(_comparison_series(
                "Before EQ", pre_ir["time_ms"], pre_ir["amplitude"],
                color="rgba(200, 200, 200, 0.6)", width=1.0,
            ))
        if post_ir:
            s_list.append(_comparison_series(
                _mode_label(mode_name), post_ir["time_ms"],
                post_ir["amplitude"], color=color, width=1.0,
            ))
        sections.append(_comparison_figure(
            f"{channel_name}: Impulse Response Before / After EQ — "
            f"{_mode_label(mode_name)}",
            axis("Time (ms)", "linear", 0, display_ms),
            axis("Amplitude", "linear", -1.1, 1.1),
            series_list=s_list, tab=tab,
        ))
    return sections


def create_score_comparison_figure(
    mode_scores: list[tuple[str, float, float]],
    loss_types: list[str | None] | None = None,
    tab=None,
) -> dict:
    """Render saved flat-loss score values in the shared comparison chart."""
    prepared_scores = [
        {"label": _mode_label(name), "pre": pre, "post": post}
        for name, pre, post in mode_scores
    ]
    return _native_score_comparison_figure(prepared_scores, loss_types, tab)


def create_smoothed_figure(
    channel_name: str,
    initial_curve: dict | None,
    final_curve: dict | None,
    octaves: float = 1.0,
    tab=None,
) -> dict | None:
    """Per-speaker 1-octave smoothed Before/After overlay (feat-report 2b)."""
    return _native_create_smoothed_figure(
        channel_name, initial_curve, final_curve, octaves, tab
    )
