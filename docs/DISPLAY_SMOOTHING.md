# Cross-language display smoothing

`autoeq_core::display_smoothing` and Swift's `FrequencyDisplaySmoother` implement
`log-cell-rectangular-cubic-v1`. This is presentation only: raw evidence, objectives,
filter coefficients and qualification are unchanged. Existing optimizer transforms
in `curve_transforms` retain their own semantics.

Modes use the same wire names: `raw`, `octaveThird`, `octaveSixth`, `octaveTwelfth`,
`octaveTwentyFourth`, `octaveFortyEighth`, `psychoacoustic`. Rust parses these with
`Mode::from_str` and returns typed errors; Swift uses the existing Codable enum.
Unknown modes must not silently become raw.

Each sample owns its cell between adjacent log2-frequency midpoints. Rectangular
windows integrate cell overlap, truncate at the supplied band edges and renormalize.
Fractional modes average dB with full widths 1/3, 1/6, 1/12, 1/24 and 1/48 octave.
Psychoacoustic mode computes `20/3 * log10(mean(10^(3*dB/20)))`, using 1/3 octave
through 100 Hz, narrowing linearly in log10 frequency to 1/6 octave at 1 kHz and
remaining there. This is not REW Gaussian/IIR or proprietary-product parity.

Nonempty finite paired samples and positive strictly increasing frequencies are
required, including raw mode. Singleton inputs are unchanged. Log-frequency cells
that collapse at floating-point precision are rejected. Double precision, a local
reference level for the cubic mean and scaling for dB means protect dynamic range.

For solver response envelopes, smooth each population member first, then calculate
displayed frequency-wise extrema and quantiles. Smoothing extrema alone does not
describe the extrema of the smoothed population. Population min/max is not a
confidence interval, parameter constraint or FIR-tap interval.

## Parity fixture and lightweight gate

Public synthetic reference: `crates/autoeq-core/tests/fixtures/display_smoothing_v1.tsv`.
Its byte-identical Swift counterpart is
`SwiftTests/OpenRoomCalibrationDSPTests/Fixtures/display_smoothing_v1.tsv` in
`open-room-calibration`. The 42 irregular samples span 20 Hz–20 kHz, include dense
clusters and the bandwidth transition, and exercise all seven modes. Expected values
were exported from the production Swift implementation. Absolute tolerance: `1e-10`
dB. Independent flat-response and analytical two-point cubic-mean tests supplement
the cross-language fixture. SHA-256:
`a997d8cf80cc76df87fdde074b1925a105cd3f5b7e52ce8e6c204084e69e74ee`.

From the umbrella checkout, without compiling the AutoEQ dependency graph:

```sh
rustc --edition=2024 --test autoeq/crates/autoeq-core/src/display_smoothing.rs \
  -D warnings -o /tmp/display-smoothing-tests
/tmp/display-smoothing-tests
```

The module uses only `std` and supports `#[path]` inclusion in bounded telemetry
trials. Swift's focused package test is `swift test --filter FrequencyDisplaySmootherTests`.
A source-only verification can compile the production smoothing file with test-only
plot/error shims to avoid building the application graph. Such a check establishes
kernel parity, not full application integration. Bump the algorithm identifier and
replace both fixtures together whenever numerical semantics change.
