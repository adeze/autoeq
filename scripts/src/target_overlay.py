"""Target-curve loading and display alignment for RoomEQ reports."""

from __future__ import annotations

from pathlib import Path

from .native_target_overlay import (
    build_target_overlay_curves as _build_native_target_overlay_curves,
    target_overlay_helper as _target_overlay_helper,
    target_shape_for_channel as _native_target_shape_for_channel,
)
from .native_target_csv import parse_target_csv as _parse_native_target_csv


#: Midband (Hz) over which a design-target level is matched to measured data.
#: Reports anchor every channel tab's target to the per-channel mean of the
#: measured L/R pair over this band so one global calibration offset applies.
TARGET_LEVEL_MATCH_BAND_HZ = (100.0, 10_000.0)
# Target files are small response tables; bounding reads also bounds the JSON
# request copied into native/WASM report transport.
MAX_TARGET_CSV_BYTES = 8 * 1024 * 1024


def _configured_target_path(data: dict, json_path: Path | None) -> Path | None:
    effective_config = (data.get("metadata") or {}).get("effective_config") or {}
    configured = effective_config.get("target_curve")
    if isinstance(configured, dict):
        configured = configured.get("path") or configured.get("file")
    if not isinstance(configured, str) or not configured.strip():
        return None

    path = Path(configured)
    if path.is_absolute():
        return path

    candidates = []
    if json_path is not None:
        candidates.append(Path(json_path).parent / path)
    candidates.append(path)
    return next((candidate for candidate in candidates if candidate.exists()), candidates[0])


def load_target_shape(data: dict, json_path: Path | None = None) -> dict | None:
    """Load the file-backed target stored in ``metadata.effective_config``."""
    path = _configured_target_path(data, json_path)
    if path is None:
        return None

    try:
        with path.open("rb") as handle:
            csv_bytes = handle.read(MAX_TARGET_CSV_BYTES + 1)
        if len(csv_bytes) > MAX_TARGET_CSV_BYTES:
            raise ValueError("target CSV exceeds the 8 MiB size limit")
        return _parse_native_target_csv(csv_bytes.decode("utf-8-sig"))
    except (OSError, UnicodeError, ValueError) as error:
        print(f"Warning: Could not load target curve '{path}': {error}")
        return None


def _interpolate_log_space(target: dict, frequencies: list[float]) -> list[float]:
    return _target_overlay_helper(
        "interpolate_log_space", target=target, frequencies=frequencies
    )


def band_mean_spl(
    curve: dict,
    min_freq: float,
    max_freq: float,
) -> float | None:
    """Arithmetic mean of finite SPL samples inside ``[min_freq, max_freq]``."""
    return _target_overlay_helper(
        "band_mean_spl", curve=curve, min_hz=min_freq, max_hz=max_freq
    )


def shift_target_to_reference_band_mean(
    target: dict,
    reference: dict,
    min_freq: float = TARGET_LEVEL_MATCH_BAND_HZ[0],
    max_freq: float = TARGET_LEVEL_MATCH_BAND_HZ[1],
) -> dict | None:
    """Interpolate ``target`` onto ``reference`` and match band means.

    Unlike :func:`align_target_to_curve` (per-channel optimizer-band fit with
    a relative-to-peak stopband guard), this applies the report-level rule:
    the target mean over ``[min_freq, max_freq]`` equals the reference mean
    over the same band. Returns ``None`` when either band is empty.
    """
    return _target_overlay_helper(
        "shift_target_to_reference_band_mean",
        target=target,
        reference=reference,
        min_hz=min_freq,
        max_hz=max_freq,
    )


def global_target_offset_for_pair(
    reference_l: dict | None,
    reference_r: dict | None,
    shape: dict | None,
    min_freq: float = TARGET_LEVEL_MATCH_BAND_HZ[0],
    max_freq: float = TARGET_LEVEL_MATCH_BAND_HZ[1],
) -> float | None:
    """Single offset anchoring a target shape to measured per-channel level.

    The anchor is the mean of the L and R reference band means over
    ``[min_freq, max_freq]``: with no L/R pair (or an empty band) there is
    no anchor (``None``) and targets render as serialized. The one offset
    applies to every channel tab, preserving designed inter-channel target
    differences. Unlike an L+R *sum* anchor, per-channel overlays land at
    the level of the per-channel data instead of 3-6 dB above it.
    """
    return _target_overlay_helper(
        "global_target_offset_for_pair",
        reference_l=reference_l,
        reference_r=reference_r,
        shape=shape,
        min_hz=min_freq,
        max_hz=max_freq,
    )


def align_target_to_curve(
    target: dict,
    reference: dict,
    min_freq: float = 20.0,
    max_freq: float = 20_000.0,
) -> dict | None:
    """Interpolate a relative target and align its level to a displayed curve.

    The relative-to-peak guard excludes crossover stopbands from level alignment.
    This is especially important for the LFE route, whose post-DSP response is
    intentionally low-passed.
    """
    return _target_overlay_helper(
        "align_target_to_curve",
        target=target,
        reference=reference,
        min_hz=min_freq,
        max_hz=max_freq,
    )


def _target_shape_for_channel(
    data: dict,
    channel_name: str,
    target: dict,
    reference: dict,
) -> dict:
    """Apply the existing DSP crossover response through the shared producer."""
    return _native_target_shape_for_channel(data, channel_name, target, reference)


def _target_alignment_band_for_channel(
    data: dict,
    channel_name: str,
    min_freq: float,
    max_freq: float,
) -> tuple[float, float]:
    """Match the routed optimizer's main-only target reference band."""
    return tuple(_target_overlay_helper(
        "target_alignment_band_for_channel",
        data=data,
        channel=channel_name,
        min_hz=min_freq,
        max_hz=max_freq,
    ))


def _global_anchor_offset(
    data: dict, reference_curves: dict[str, dict]
) -> float | None:
    """Mean-based offset for absolute targets; None without an L/R anchor."""
    return _target_overlay_helper(
        "global_target_offset", data=data, reference_curves=reference_curves
    )


def build_target_overlay_curves(data, reference_curves, json_path=None):
    """Build overlays through the shared Rust producer using resolved target data."""
    return _build_native_target_overlay_curves(
        data, reference_curves, load_target_shape(data, json_path)
    )
