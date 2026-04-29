/* tslint:disable */
/* eslint-disable */

export class Cpu {
    free(): void;
    [Symbol.dispose](): void;
    entry_pc(): bigint;
    l1_table_pa(): bigint;
    mem_slice(start: number, len: number): Uint8Array;
    constructor();
    num_cores(): number;
    output(): string;
    reset(): void;
    run(max: number): number;
    /**
     * Returns an array of CoreState (one per core) as a JS Array.
     */
    state(): any;
    /**
     * Step every core once (deterministic order, core 0 first). Returns true
     * if any core was runnable (i.e., made forward progress).
     */
    step(): boolean;
    /**
     * Step a single core. Useful for "advance only this core" UI controls.
     */
    step_core(idx: number): boolean;
    /**
     * Walk page tables for `va` using the sysregs of `core_idx`.
     */
    translate(va: bigint, core_idx: number): any;
    uart_addr(): bigint;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_cpu_free: (a: number, b: number) => void;
    readonly cpu_mem_slice: (a: number, b: number, c: number) => [number, number];
    readonly cpu_new: () => number;
    readonly cpu_num_cores: (a: number) => number;
    readonly cpu_output: (a: number) => [number, number];
    readonly cpu_reset: (a: number) => void;
    readonly cpu_run: (a: number, b: number) => number;
    readonly cpu_state: (a: number) => [number, number, number];
    readonly cpu_step: (a: number) => number;
    readonly cpu_step_core: (a: number, b: number) => number;
    readonly cpu_translate: (a: number, b: bigint, c: number) => [number, number, number];
    readonly cpu_entry_pc: (a: number) => bigint;
    readonly cpu_l1_table_pa: (a: number) => bigint;
    readonly cpu_uart_addr: (a: number) => bigint;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
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
