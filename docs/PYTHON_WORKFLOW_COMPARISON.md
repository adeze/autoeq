# Bounded Python workflow comparison

The fixture is synthetic and consumes no private measurements, candidates, or receiver state. It compares explicit numerical operations, not full calibration engines.

## Reproduce

From AutoEQ, with an explicit path to the existing Python checkout:

```sh
uv run --project /absolute/path/to/py-open-room-calibration --frozen python scripts/generate_python_workflow_vectors.py > /tmp/python_workflow_vectors.json
cmp tests/data/python_workflow_vectors.json /tmp/python_workflow_vectors.json
cargo test -p roomeq-engine --test python_workflow_vectors
```

The generator calls `ChannelCalibrationTarget.evaluate` and `HighFrequencyPhaseAligner.synthesize_minimum_phase_fir`. It records SHA-256 hashes of those function sources and the normalization function. Checked-in vectors let Rust tests run without a Python runtime. Source changes require explicit regeneration and review; hashes identify source text, not complete dependency or environment provenance.

## Established scope and gaps

| Operation | Comparison contract | Limit |
|---|---|---|
| Target evaluation | Pure -0.4 dB/octave tilt, 1000 Hz anchor, frequencies at or above 120 Hz; shelf and HF damping disabled | Python's default shelf exponent, knee-gated tilt and HF damping differ from Rust's procedural target. Defaults are not equivalent. A sampled target can represent the explicit Python curve. |
| Sampled-target normalization | Explicit 80 dB measurement and octave-segment mean offset on an identical grid | This is a documented comparison operation. It is not the Python preset pipeline's anchoring rule. Rust's means accumulate float32 values; tolerance is 0.00002 dB. |
| Realized FIR evaluation | Identical Python-generated float32 taps, promoted without alteration to float64; 128 taps at 48 kHz and 512 taps at 6 kHz; positive and inverted satellite signs; DC through Nyquist | Complex response tolerance is 1e-11; dB tolerance is 1e-9. This checks evaluation of serialized coefficients, not equality of Rust and Python FIR synthesis. |
| Optimization | Rust already has multi-seat optimization, source-bound saved-IR refinement, training/held-out acceptance and final FIR binding | No end-to-end optimizer comparison is established. Python's saved-preset adapter returns a summary with incomplete processing/reference provenance and no transferable FIR/manifest. |
| Temporal evidence | Rust reports direct/early/late correction energy and separately searches saved-IR temporal FIR candidates | Python's new direct/cumulative-early/full spectral upper-band veto is a different operation. Reporting metrics are not that veto. |
| Exports | Rust packages existing external DSP formats; sampled targets and coefficients retain their existing paths | No ADY/OCA or receiver-contract parity follows. Use the canonical Swift workflow for governed candidate generation and receiver actions. |

No numerical defaults changed. Future comparisons must bind qualified inputs, channel identity, frequency grids, processing/stimulus references, sample rates, tap counts, polarity, timing and realized response. A matched model output does not establish measured acoustic benefit.
