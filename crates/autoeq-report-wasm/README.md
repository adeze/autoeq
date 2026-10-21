# autoeq-report-wasm

## Generated browser assets

Native report assembly embeds four files from `dist/`; build real assets before
compiling native consumers such as SOTF's report host:

```sh
just report-dist
just report-dist-gpui
```

These recipes require the stable and nightly `wasm32-unknown-unknown` targets
respectively, and a `wasm-bindgen` CLI matching the lockfile (currently 0.2.129).
`WANDBIN` may point to that executable. Generated bundles remain ignored files.
The wasm32 report library requires explicit `shell_assets` for assembly; it
cannot embed its own not-yet-built binary. Native asset validation is unchanged.

## Report library

Shared AutoEQ report payload types and the platform-independent renderer core.
The crate owns the versioned JSON schema in `src/schema.rs`, HTML assembly,
Canvas 2D drawing, and grid rendering. `autoeq-plot` produces payloads using
these types. The renderer uses `d3rs` for scales, axes, colors, and Sankey
layout, with Canvas as the fallback when WebGPU is unavailable.

This crate is an `rlib` and does not own the WebAssembly export bundle. The
separate `autoeq-report-wasm-shell` crate provides the `cdylib` exports, while
`autoeq-report-gpui` provides the optional explorer viewer.

The payload contract and examples are documented in
[`SCHEMA.md`](SCHEMA.md). Run its library tests with:

```bash
cargo test -p autoeq-report-wasm --locked --lib
```
