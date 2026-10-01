"""Generate synthetic comparison vectors using the existing Python DSP contracts.

Run with the explicitly selected py-open-room-calibration uv project; no saved
private measurements or receiver candidates are consumed. Output goes to stdout.
"""

import hashlib
import inspect
import json
from dataclasses import asdict
from itertools import pairwise

import numpy as np
from open_room_calibration.dsp.contracts import ChannelCalibrationTarget
from open_room_calibration.dsp.excess_phase_aligner import HighFrequencyPhaseAligner


def main() -> None:
    frequencies = np.asarray([120, 240, 480, 960, 1920, 3840, 7680, 15360], dtype=float)
    target = ChannelCalibrationTarget(sub_shelf_gain_db=0, hf_damping_slope_db_per_octave=0)
    target_db = target.evaluate(frequencies).astype(float)
    np.testing.assert_allclose(target_db, -0.4 * np.log2(frequencies / 1000), atol=2e-7, rtol=0)
    # Explicit comparison operation: octave-segment trapezoidal mean, f32
    # accumulation like Rust's mean_response_in_range. Not Python preset anchoring.
    levels = target_db.astype(np.float32)
    total = np.float32(0)
    for left, right in pairwise(levels):
        total += (left + right) / np.float32(2)
    offset = 80.0 - float(total / np.float32(len(levels) - 1))
    cases = []
    for count, rate in [(128, 48000), (512, 6000)]:
        axis = np.asarray([20, 60, 120, 300, 1000, rate / 2], dtype=float)
        gains = np.asarray([0, -2, -6, -3, -1, 0], dtype=float)
        taps = HighFrequencyPhaseAligner.synthesize_minimum_phase_fir(axis, gains, count, rate).astype(float)
        bins = np.asarray([0, 20, 60, 120, 300, 1000, rate / 2], dtype=float)
        for sign in ([1, -1] if count == 128 else [1]):
            signed = taps * sign
            response = np.exp(-2j * np.pi * bins[:, None] * np.arange(count) / rate) @ signed
            assert np.isfinite(response).all() and np.isfinite(signed).all()
            cases.append({"tap_count": count, "sample_rate_hz": rate, "sign": sign,
                          "requested_frequencies_hz": axis.tolist(), "requested_gain_db": gains.tolist(),
                          "taps": signed.tolist(), "frequencies_hz": bins.tolist(),
                          "response_re": response.real.tolist(), "response_im": response.imag.tolist(),
                          "response_db": (20 * np.log10(np.abs(response))).tolist()})
    functions = [ChannelCalibrationTarget.evaluate, HighFrequencyPhaseAligner.synthesize_minimum_phase_fir,
                 HighFrequencyPhaseAligner.normalize_gain]
    sources = {fn.__qualname__: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for fn in functions}
    result = {"schema_version": 1, "evidence_status": "synthetic_comparison_only",
              "numpy_version": np.__version__, "shared_target_parameters": asdict(target),
              "default_target_parameters": asdict(ChannelCalibrationTarget()),
              "python_source_sha256": sources,
              "frequencies_hz": frequencies.tolist(), "target_db": target_db.tolist(),
              "default_python_target_db": ChannelCalibrationTarget().evaluate(frequencies).astype(float).tolist(),
              "normalization_offset_db": offset, "normalized_target_db": (target_db + offset).tolist(),
              "fir_cases": cases}
    print(json.dumps(result, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
