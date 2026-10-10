/* tslint:disable */
/* eslint-disable */

/**
 * Boot the viewer into the shell container. Idempotent: resolves
 * immediately when the container already holds the canvas.
 */
export function mount_report(container_id: string): Promise<void>;

/**
 * Entry point for web workers
 */
export function wasm_thread_entry_point(ptr: number): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly mount_report: (a: number, b: number) => any;
    readonly wasm_thread_entry_point: (a: number) => void;
    readonly wasm_bindgen_8346fa3ca1514f34___convert__closures_____invoke___js_sys_f2b43b9a95a28f9a___Function_fn_wasm_bindgen_8346fa3ca1514f34___JsValue_____wasm_bindgen_8346fa3ca1514f34___sys__Undefined___js_sys_f2b43b9a95a28f9a___Function_fn_wasm_bindgen_8346fa3ca1514f34___JsValue_____wasm_bindgen_8346fa3ca1514f34___sys__Undefined_______true_: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen_8346fa3ca1514f34___convert__closures_____invoke___wasm_bindgen_8346fa3ca1514f34___JsValue__core_e8e77cc1e90bd3b0___result__Result_____wasm_bindgen_8346fa3ca1514f34___JsError___true_: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen_8346fa3ca1514f34___convert__closures________invoke___web_sys_6f1a51b6b48ebdd3___features__gen_MessageEvent__MessageEvent______true_: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_8346fa3ca1514f34___convert__closures_____invoke___js_sys_f2b43b9a95a28f9a___Array______true_: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_8346fa3ca1514f34___convert__closures_____invoke___js_sys_f2b43b9a95a28f9a___Array______true__7: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_8346fa3ca1514f34___convert__closures_____invoke___wasm_bindgen_8346fa3ca1514f34___JsValue______true_: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_8346fa3ca1514f34___convert__closures_____invoke___wasm_bindgen_8346fa3ca1514f34___JsValue______true__6: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_8346fa3ca1514f34___convert__closures_____invoke___bool__true_: (a: number, b: number) => number;
    readonly wasm_bindgen_8346fa3ca1514f34___convert__closures_____invoke_______true_: (a: number, b: number) => void;
    readonly __wbindgen_malloc_command_export: (a: number, b: number) => number;
    readonly __wbindgen_realloc_command_export: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store_command_export: (a: number) => void;
    readonly __externref_table_alloc_command_export: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free_command_export: (a: number, b: number, c: number) => void;
    readonly __wbindgen_destroy_closure_command_export: (a: number, b: number) => void;
    readonly __externref_table_dealloc_command_export: (a: number) => void;
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
