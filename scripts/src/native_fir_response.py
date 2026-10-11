"""Thin bridge for canonical frozen FIR responses."""

import json
import math

from .native_report import produce


def fir_response(frequencies: list[float], sample_rate: float, *,
                 taps: list[float] | None = None,
                 resource_key: str | None = None,
                 graph_sha256: str | None = None,
                 resource_sha256: str | None = None) -> list[complex]:
    """Return the canonical FIR response from frozen taps or a host resource key.

    The host form contains no coefficient data: the parent callback resolves
    ``resource_key`` against its active immutable, manifest-verified snapshot.
    Standalone report generation may provide taps that were captured and
    validated by its loader before any source file changed.
    """
    if isinstance(sample_rate, bool) or not isinstance(sample_rate, (int, float)):
        raise ValueError("FIR sample rate must be numeric")
    sample_rate = float(sample_rate)
    if not math.isfinite(sample_rate) or sample_rate <= 0:
        raise ValueError("FIR sample rate must be finite and positive")
    if not isinstance(frequencies, list) or len(frequencies) > 100_000:
        raise ValueError("FIR response frequencies must be a bounded list")
    for frequency in frequencies:
        if (isinstance(frequency, bool) or not isinstance(frequency, (int, float))
                or not math.isfinite(frequency) or not 0 <= frequency <= sample_rate / 2):
            raise ValueError("FIR response frequencies must be finite and between DC and Nyquist")

    request = {
        "operation": "fir_response",
        "frequencies": frequencies,
        "sample_rate": sample_rate,
    }
    if resource_key is not None:
        if not isinstance(resource_key, str) or not resource_key:
            raise ValueError("Native FIR resource key must be a non-empty graph ir_file")
        for name, digest in (("graph", graph_sha256), ("resource", resource_sha256)):
            if (not isinstance(digest, str) or len(digest) != 64
                    or any(character not in "0123456789abcdef" for character in digest)):
                raise ValueError(f"Native FIR {name} SHA-256 is invalid")
        request.update(
            resource_key=resource_key,
            graph_sha256=graph_sha256,
            resource_sha256=resource_sha256,
        )
    else:
        if graph_sha256 is not None or resource_sha256 is not None:
            raise ValueError("Native FIR identity requires a resource key")
        if (not isinstance(taps, list) or not taps
                or any(isinstance(tap, bool) or not isinstance(tap, (int, float))
                       or not math.isfinite(tap) for tap in taps)):
            raise ValueError("Standalone FIR replay requires finite captured taps")
        request["taps"] = taps

    try:
        pairs = json.loads(produce(request))
    except (RuntimeError, json.JSONDecodeError) as error:
        raise ValueError(f"Canonical FIR response unavailable: {error}") from error
    if not isinstance(pairs, list) or len(pairs) != len(frequencies):
        raise ValueError("Canonical FIR response returned an invalid frequency grid")
    response = []
    for pair in pairs:
        if (not isinstance(pair, list) or len(pair) != 2
                or any(isinstance(value, bool) or not isinstance(value, (int, float))
                       or not math.isfinite(value) for value in pair)):
            raise ValueError("Canonical FIR response returned an invalid complex value")
        response.append(complex(pair[0], pair[1]))
    return response
