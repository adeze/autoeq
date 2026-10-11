"""Validate and select frozen native per-driver acoustic replay curves."""

from __future__ import annotations

import math
from typing import Any


MAX_DRIVER_CURVE_POINTS = 100_000
_ALIGNED_AUXILIARY_FIELDS = ("noise_floor_db", "coherence")


def _finite_number(value: Any) -> bool:
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        return False
    try:
        return math.isfinite(float(value))
    except (OverflowError, ValueError):
        return False


def _numeric_vector(value: Any, *, positive: bool = False) -> bool:
    return (
        isinstance(value, list)
        and all(_finite_number(item) and (not positive or item > 0.0) for item in value)
    )


def validate_native_curve(curve: Any) -> tuple[bool, str | None]:
    """Validate a serialized native CurveData value without inventing evidence.

    The check is intentionally independent of sample rate.  It validates the
    retained measurement grid and all optional per-bin evidence fields, while
    allowing phase, coherence, and noise floor to remain absent.
    """
    if not isinstance(curve, dict):
        return False, "native curve is not an object"
    freq = curve.get("freq")
    spl = curve.get("spl")
    if not _numeric_vector(freq, positive=True) or not freq:
        return False, "native curve frequencies must be finite positive numbers"
    if len(freq) > MAX_DRIVER_CURVE_POINTS:
        return False, f"native curve exceeds {MAX_DRIVER_CURVE_POINTS} points"
    if any(right <= left for left, right in zip(freq, freq[1:])):
        return False, "native curve frequencies are not strictly increasing"
    if not _numeric_vector(spl) or len(spl) != len(freq):
        return False, "native curve SPL must be finite and match the frequency grid"

    phase = curve.get("phase")
    if phase is not None and (
        not _numeric_vector(phase) or len(phase) != len(freq)
    ):
        return False, "native curve phase must be finite and match the frequency grid"
    for field in _ALIGNED_AUXILIARY_FIELDS:
        values = curve.get(field)
        if values is not None and (
            not _numeric_vector(values) or len(values) != len(freq)
        ):
            return False, f"native curve {field} must be finite and match the frequency grid"

    norm_range = curve.get("norm_range")
    if norm_range is not None:
        if (
            not isinstance(norm_range, (list, tuple))
            or len(norm_range) != 2
            or not all(_finite_number(value) for value in norm_range)
            or norm_range[0] <= 0.0
            or norm_range[1] < norm_range[0]
        ):
            return False, "native curve norm_range is invalid"
    return True, None


def _positive_u32_rate(value: Any) -> bool:
    return type(value) is int and 0 < value <= 0xFFFFFFFF


def resolve_native_driver_curve(
    data: Any, channel_name: str, source_driver: Any
) -> tuple[bool, dict | None, str | None]:
    """Resolve one result by graph identity; never fall back in native mode.

    Returns ``(False, None, None)`` only for markerless standalone data, where
    the caller should retain its historical Python replay behavior.  Once the
    host marks a payload as native, every missing or invalid result is handled
    as an explicit unavailable state.
    """
    if getattr(data, "native_dsp_replay_present", False) is not True:
        return False, None, None

    replay = getattr(data, "native_dsp_replay", None)
    if not isinstance(replay, dict):
        return True, None, "native DSP replay transport is missing or malformed"
    rate = replay.get("sample_rate_hz")
    if not _positive_u32_rate(rate):
        return True, None, "native driver replay has no valid explicit sample rate"
    context_rate = getattr(data, "native_playback_sample_rate", None)
    if context_rate is not None:
        if not _positive_u32_rate(context_rate):
            return True, None, "native playback sample-rate context is invalid"
        if context_rate != rate:
            return True, None, (
                f"native replay rate {rate} Hz does not match report playback rate "
                f"{context_rate} Hz"
            )

    if not isinstance(channel_name, str) or not channel_name:
        return True, None, "native driver replay channel identity is invalid"
    if not isinstance(source_driver, dict):
        return True, None, "source driver identity is unavailable"
    driver_index = source_driver.get("index")
    driver_name = source_driver.get("name")
    if type(driver_index) is not int or driver_index < 0 or not isinstance(driver_name, str):
        return True, None, "source driver has no valid original index and name"

    channel_results = replay.get("drivers")
    if not isinstance(channel_results, dict):
        return True, None, "native replay has no per-driver result map"
    results = channel_results.get(channel_name)
    if not isinstance(results, list):
        return True, None, f"native replay has no driver results for {channel_name}"
    matches = [
        result
        for result in results
        if isinstance(result, dict)
        and result.get("index") == driver_index
        and type(result.get("index")) is int
        and result.get("name") == driver_name
    ]
    if len(matches) != 1:
        if matches:
            return True, None, "native replay driver identity is ambiguous"
        return True, None, (
            f"native replay has no exact result for {channel_name} driver "
            f"{driver_name!r} at original index {driver_index}"
        )

    state = matches[0].get("state")
    if not isinstance(state, dict):
        return True, None, "native driver replay state is malformed"
    status = state.get("status")
    if status == "unavailable":
        reason = state.get("reason")
        if not isinstance(reason, str) or not reason.strip():
            reason = "native replay marked this driver unavailable"
        return True, None, reason
    if status != "available":
        return True, None, "native driver replay status is missing or unknown"
    curve = state.get("curve")
    valid, reason = validate_native_curve(curve)
    if not valid:
        return True, None, reason

    initial = source_driver.get("initial_curve")
    valid, reason = validate_native_curve(initial)
    if not valid:
        return True, None, f"source measurement curve is invalid: {reason}"
    source_freq = initial["freq"]
    band = source_driver.get("measured_band_hz")
    if band is None:
        retained = list(range(len(source_freq)))
    else:
        if (
            not isinstance(band, (list, tuple))
            or len(band) != 2
            or not all(_finite_number(edge) for edge in band)
            or band[0] <= 0.0
            or band[0] >= band[1]
        ):
            return True, None, "source driver measured-band bounds are invalid"
        retained = [
            index for index, frequency in enumerate(source_freq)
            if band[0] <= frequency <= band[1]
        ]
        if len(retained) < 2:
            return True, None, "source driver measured band retains fewer than two bins"
    expected_freq = [source_freq[index] for index in retained]
    if curve["freq"] != expected_freq:
        return True, None, "native driver curve does not match the clipped measurement grid"

    source_phase = initial.get("phase")
    result_phase = curve.get("phase")
    if source_phase is None and result_phase is not None:
        return True, None, "native replay added phase to a measurement with unknown phase"
    if source_phase is not None and result_phase is None:
        return True, None, "native replay dropped measured phase evidence"
    for field in _ALIGNED_AUXILIARY_FIELDS:
        source_values = initial.get(field)
        result_values = curve.get(field)
        if source_values is None and result_values is not None:
            return True, None, f"native replay added unavailable {field} evidence"
        if source_values is not None and result_values is None:
            return True, None, f"native replay dropped measured {field} evidence"
        if source_values is not None:
            expected_values = [source_values[index] for index in retained]
            if result_values != expected_values:
                return True, None, f"native replay changed measured {field} evidence"
    source_norm = initial.get("norm_range")
    result_norm = curve.get("norm_range")
    if source_norm is None and result_norm is not None:
        return True, None, "native replay added unavailable normalization provenance"
    if source_norm is not None and list(source_norm) != list(result_norm or []):
        return True, None, "native replay changed measurement normalization provenance"

    # Return the complete value so optional phase, coherence, noise-floor, and
    # normalization provenance survive into the figures unchanged.
    return True, dict(curve), None
