"""Thin bridge for canonical serialized DSP filter responses."""

import json
import math

from .native_report import produce


def serialized_filter_response(filt: dict, frequencies: list[float], sample_rate: float) -> list[complex]:
    """Return Rust playback responses after legacy-shaped input normalization."""
    # Import here to keep dsp.py's public helper as a one-way adapter edge.
    from .dsp import _canonical_filter_type, _finite_filter_number, kautz_sections

    sample_rate = _finite_filter_number(sample_rate, "sample rate")
    if sample_rate <= 0:
        raise ValueError("EQ sample rate must be positive")
    for frequency in frequencies:
        value = _finite_filter_number(frequency, "evaluation frequency")
        if not 0 <= value <= sample_rate / 2:
            raise ValueError("EQ evaluation frequency must be between DC and Nyquist")

    if not isinstance(filt, dict):
        raise ValueError("serialized EQ filter must be an object")
    normalized = dict(filt)
    topology = filt.get("topology", "biquad")
    if topology not in ("biquad", "warped_biquad", "kautz_filter"):
        raise ValueError(f"unsupported serialized EQ topology: {topology!r}")

    if topology == "kautz_filter":
        # Keep Python's historical malformed-input messages while the canonical
        # implementation owns all response equations and section ordering.
        kautz_sections(filt, sample_rate)
    elif topology == "warped_biquad":
        filter_type = _canonical_filter_type(filt.get("filter_type", "peak"))
        center = _finite_filter_number(filt.get("freq"), "frequency")
        q = _finite_filter_number(filt.get("q"), "Q")
        gain = _finite_filter_number(filt.get("db_gain"), "dB gain")
        if not 0 < center < sample_rate / 2 or q <= 0:
            raise ValueError("warped EQ requires a sub-Nyquist positive frequency and positive Q")
        normalized.update(filter_type=filter_type, freq=center, q=q, db_gain=gain)
        if "lambda" in filt:
            normalized["lambda"] = _finite_filter_number(filt["lambda"], "lambda")
    else:
        filter_type = _canonical_filter_type(filt.get("filter_type", "peak"))
        center = float(filt.get("freq", filt.get("frequency", 1_000.0)))
        nyquist = sample_rate / 2.0
        margin = nyquist * math.sqrt(math.ulp(1.0))
        if not math.isfinite(center) or center <= 0:
            center = margin
        elif center >= nyquist:
            center = nyquist - margin

        q = float(filt.get("q", 1.0))
        if not math.isfinite(q):
            raise ValueError(f"biquad Q must be finite, got {q!r}")
        if q == 0.0:
            if filter_type == "notch":
                q = 30.0
            elif filter_type in {"bandpass", "highpass", "lowpass"}:
                q = 1.0 / math.sqrt(2.0)
            elif filter_type in {"lowshelf", "highshelf", "lowshelforf", "highshelforf"}:
                q = 1.0668676536332304
        if q <= 0.0:
            q = 1.0e-2
        gain = float(filt.get("db_gain", filt.get("gain", 0.0)))
        if not math.isfinite(gain):
            gain = 0.0
        normalized.update(filter_type=filter_type, freq=center, q=q, db_gain=gain)

    try:
        output = produce({
            "operation": "serialized_filter_response",
            "filter": normalized,
            "frequencies": frequencies,
            "sample_rate": sample_rate,
        })
        pairs = json.loads(output)
    except (RuntimeError, json.JSONDecodeError) as error:
        raise ValueError(str(error)) from error
    if not isinstance(pairs, list) or len(pairs) != len(frequencies):
        raise ValueError("Canonical serialized EQ response returned an invalid grid")
    result = []
    for pair in pairs:
        if not isinstance(pair, list) or len(pair) != 2:
            raise ValueError("Canonical serialized EQ response returned an invalid complex value")
        real, imaginary = pair
        if (isinstance(real, bool) or not isinstance(real, (int, float))
                or isinstance(imaginary, bool) or not isinstance(imaginary, (int, float))
                or not math.isfinite(real) or not math.isfinite(imaginary)):
            raise ValueError("Canonical serialized EQ response returned a non-finite value")
        result.append(complex(real, imaginary))
    return result
