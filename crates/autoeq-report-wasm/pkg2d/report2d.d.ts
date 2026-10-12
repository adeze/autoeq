/* tslint:disable */
/* eslint-disable */

/**
 * Builds the shared Crossover Configuration HTML section in WebAssembly.
 *
 * `data_json` is a serialized RoomEQ result document. The returned string is
 * the same escaped HTML fragment used by native report generation.
 *
 * # Errors
 *
 * Returns a JavaScript error when `data_json` is not valid JSON.
 */
export function crossover_config_html(data_json: string): string;

/**
 * Last internal error captured by [`toggle_series`] (empty when none).
 */
export function last_error(): string;

/**
 * Legend hit rectangles for a rendered canvas as JSON
 * (`[{"series":i,"x":..,"y":..,"w":..,"h":..}]`, `"[]"` when unknown).
 */
export function legend_json(canvas_id: string): string;

/**
 * Dispatches one canonical report producer request in WebAssembly.
 *
 * Section results are serialized JSON; the assembly operation returns HTML.
 *
 * # Errors
 * Returns an error for invalid JSON, unsupported operations, or invalid request fields.
 */
export function produce_json(request_json: string): string;

/**
 * Render one payload section into a canvas.
 *
 * Returns 0 on success, 1 when the canvas is missing, 2 on bad JSON.
 */
export function render_section(canvas_id: string, section_json: string, w: number, h: number, dpr: number): number;

/**
 * Payload schema discriminator (lets the shell refuse stale documents).
 */
export function schema_version(): string;

/**
 * Map a rubber-band pixel rectangle to data domains for box zoom.
 *
 * `x0/y0/x1/y1` are canvas CSS px (origin top-left). Returns
 * `{"x":[lo,hi],"y":[lo,hi]}` in data units, or `""` when the canvas is
 * unknown, never rendered a figure, or the selection is degenerate.
 */
export function selection_domain_json(canvas_id: string, x0: number, y0: number, x1: number, y1: number): string;

/**
 * Builds the canonical report fragments as one JSON object in WebAssembly.
 *
 * The input is a serialized RoomEQ result document. HTML fragments are
 * strings; signal-flow sections are ordinary report section objects.
 *
 * # Errors
 *
 * Returns a JavaScript error when `data_json` is not valid JSON.
 */
export function shared_sections_json(data_json: string): string;

/**
 * Flip one series' visibility and re-render. Returns 1 when re-rendered,
 * 0 when the canvas or series is unknown, -99 on an internal panic
 * (see [`last_error`]).
 */
export function toggle_series(canvas_id: string, idx: number): number;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly crossover_config_html: (a: number, b: number) => [number, number, number, number];
    readonly last_error: () => [number, number];
    readonly legend_json: (a: number, b: number) => [number, number];
    readonly produce_json: (a: number, b: number) => [number, number, number, number];
    readonly render_section: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => number;
    readonly schema_version: () => [number, number];
    readonly selection_domain_json: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number];
    readonly shared_sections_json: (a: number, b: number) => [number, number, number, number];
    readonly toggle_series: (a: number, b: number, c: number) => number;
    readonly __wbindgen_exn_store_command_export: (a: number) => void;
    readonly __externref_table_alloc_command_export: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc_command_export: (a: number, b: number) => number;
    readonly __wbindgen_realloc_command_export: (a: number, b: number, c: number, d: number) => number;
    readonly __externref_table_dealloc_command_export: (a: number) => void;
    readonly __wbindgen_free_command_export: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
