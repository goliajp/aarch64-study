/* tslint:disable */
/* eslint-disable */

export class Cpu {
    free(): void;
    [Symbol.dispose](): void;
    entry_pc(): bigint;
    l1_table_pa(): bigint;
    /**
     * Return a slice of memory as a Uint8Array. `start` and `len` are byte offsets.
     */
    mem_slice(start: number, len: number): Uint8Array;
    constructor();
    output(): string;
    reset(): void;
    /**
     * Run up to `max` steps or until halted/trapped. Returns steps actually executed.
     */
    run(max: number): number;
    state(): any;
    /**
     * Execute one instruction. Returns true if the CPU is still runnable.
     */
    step(): boolean;
    /**
     * Walk the stage-1 page tables for `va` using the current TTBR0/TCR.
     * Returns the walk trace plus the resolved physical address (or fault).
     */
    translate(va: bigint): any;
    uart_addr(): bigint;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_cpu_free: (a: number, b: number) => void;
    readonly cpu_mem_slice: (a: number, b: number, c: number) => [number, number];
    readonly cpu_new: () => number;
    readonly cpu_output: (a: number) => [number, number];
    readonly cpu_reset: (a: number) => void;
    readonly cpu_run: (a: number, b: number) => number;
    readonly cpu_state: (a: number) => [number, number, number];
    readonly cpu_step: (a: number) => number;
    readonly cpu_translate: (a: number, b: bigint) => [number, number, number];
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
