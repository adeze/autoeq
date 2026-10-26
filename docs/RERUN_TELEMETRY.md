# Direct DE telemetry

Build the optional CLI adapter with `--features rerun-telemetry`. This forwards
the root feature to `autoeq-cli`, where Rerun SDK `0.40.0` (compatible requirement) has default features
disabled and only `sdk` enabled. No viewer, GPU renderer, chat or app-shell code
is added. Without that feature, telemetry flags return an explicit error.

```sh
cargo build --locked --features rerun-telemetry --bin autoeq
target/debug/autoeq --curve measurement.csv --loss speaker-flat \
  --algo autoeq:de --peq-model pk --num-filters 2 --seed 91827 \
  --rerun-recording study.rrd --rerun-port 9876
```

The port is optional, always on `127.0.0.1`. Start a compatible viewer separately.
Use the normal input, objective and optimization options; the example does not
authorize receiver writes or represent a production calibration. Exact checkpoint
saving/resume remains available through `--checkpoint-exact` / `--resume-exact`.
Recording alone does not require a checkpoint file. Warm-start checkpoint flags,
other optimizers, non-`pk` layouts, multi-driver/subwoofer/multi-curve objectives,
missing seeds and local refinement are rejected rather than partially traced.

The existing safe-generation-barrier callback is composed with checkpoint saving.
Every callback is recorded synchronously to the local SDK stream, including the
initial barrier and same-generation terminal finalization; there is no polling
or sampling. `generation` and unique `callback_index` remain separate timelines.
Resumed studies begin at their actual resumed generation and do not invent earlier
history. Outcome metadata records callback count, first/last generation, expected
generation barriers and terminal observation. Immutable solver/build/configuration
identities, seed, measurement identity and serialized checkpoint SHA-256 retain
provenance. Each invocation uses a fresh viewer recording ID, shared by its file
and network streams, independently of the stable scientific run identity.

Frequency response uses the existing `ObjectiveContext.peq_spl` on the full
canonical objective grid. The prepared measurement is `target - deviation`;
predicted correction is that measurement plus the candidate PEQ magnitude.
Measurement/target are static; candidate filter and predicted response change
at each callback. Final post-polish response is separately named. These are
magnitude predictions, not new acoustic measurements, impulse/phase simulations
or validation of an acoustic model. No extra smoothing or interpolation is used.

The `frequency_response` display is a common 12×6 plotting frame: log-frequency
scaled horizontally, fixed per-run dB domain vertically, with physical Hz/dB
ticks and higher level upward on the Y-down Spatial2D canvas. The domain comes
from prepared measurement/target plus 20 dB margin and never rescales per frame;
out-of-domain curves are not clipped. Display vertices are float32; separate
float64 tensors preserve every original grid/response value. Levels are prepared
and normalized; absolute calibrated SPL is not asserted. Loss is loss-specific,
not labeled dB. This is a chart projection, not physical room geometry.

The local file is primary. Each callback flushes that file with a 5 s bound,
providing backpressure and surfacing recording failures instead of dropping
updates. The optional viewer stream uses SDK queues; no explicit network flush
occurs in the objective callback. SDK queue backpressure can increase runtime.
Final viewer flush has a 2 s bound. Viewer-only delivery failure warns and is
recorded in local metadata; it does not suppress valid optimization exports.
Local recording failure remains fatal. Transport delivery still needs viewer review;
enqueueing is not proof. SDK teardown/filesystem I/O can add latency.

Existing outputs are refused. Recordings stage beside the destination and publish
without overwriting, including publication races. Optimization or local-recording
failure retains available partial data with outcome metadata and returns an
error; abrupt process termination cannot guarantee finalization. Successful
observer execution and solver convergence are distinct facts. Telemetry does not
modify objective functions, bounds, RNG seeds, selected parameters or filters;
offline numerical parity remains a separate required check.
