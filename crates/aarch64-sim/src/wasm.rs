//! `#[wasm_bindgen]` glue. Active only when the `wasm` feature is enabled.
//!
//! This module is intentionally thin: it forwards every call to the pure-Rust
//! API on `Cpu` and serialises returned values via `serde-wasm-bindgen` so JS
//! sees plain objects with `BigInt` for `u64` fields.
//!
//! JS API surface preserved: `cpu.step()`, `cpu.state()`, `cpu.aic_state()`,
//! `cpu.block_state()`, `cpu.translate()`, etc. — same names as the pure-Rust
//! API, exposed via `js_name`.

use serde::Serialize;
use serde_wasm_bindgen::Serializer;
use wasm_bindgen::prelude::*;

use crate::Cpu;

/// Build a `JsValue` from any serde value, keeping `u64`/`i64`/`u128`/`i128`
/// as JS `BigInt` rather than the lossy `Number` default serde-wasm-bindgen
/// ships with.
fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    let serializer = Serializer::new().serialize_large_number_types_as_bigints(true);
    value
        .serialize(&serializer)
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

#[wasm_bindgen]
impl Cpu {
    #[wasm_bindgen(constructor)]
    pub fn new_wasm() -> Cpu {
        Cpu::new()
    }

    #[wasm_bindgen(js_name = "reset")]
    pub fn reset_wasm(&mut self) {
        self.reset();
    }

    #[wasm_bindgen(js_name = "step")]
    pub fn step_wasm(&mut self) -> bool {
        self.step()
    }

    #[wasm_bindgen(js_name = "step_core")]
    pub fn step_core_wasm(&mut self, idx: u32) -> bool {
        self.step_core(idx)
    }

    #[wasm_bindgen(js_name = "run")]
    pub fn run_wasm(&mut self, max: u32) -> u32 {
        self.run(max)
    }

    #[wasm_bindgen(js_name = "system_steps")]
    pub fn system_steps_wasm(&self) -> u64 {
        self.system_steps()
    }

    #[wasm_bindgen(js_name = "timer_period")]
    pub fn timer_period_wasm(&self) -> u64 {
        self.timer_period()
    }

    #[wasm_bindgen(js_name = "timer_remaining")]
    pub fn timer_remaining_wasm(&self) -> u64 {
        self.timer_remaining()
    }

    #[wasm_bindgen(js_name = "timer_ticks")]
    pub fn timer_ticks_wasm(&self) -> u64 {
        self.timer_ticks()
    }

    #[wasm_bindgen(js_name = "aic_state")]
    pub fn aic_state_wasm(&self) -> Result<JsValue, JsValue> {
        to_js(&self.aic_state())
    }

    #[wasm_bindgen(js_name = "block_state")]
    pub fn block_state_wasm(&self) -> Result<JsValue, JsValue> {
        to_js(&self.block_state())
    }

    #[wasm_bindgen(js_name = "set_disk_text")]
    pub fn set_disk_text_wasm(&mut self, text: &str) {
        self.set_disk_text(text);
    }

    #[wasm_bindgen(js_name = "disk_text")]
    pub fn disk_text_wasm(&self) -> String {
        self.disk_text()
    }

    #[wasm_bindgen(js_name = "state")]
    pub fn state_wasm(&self) -> Result<JsValue, JsValue> {
        to_js(&self.state())
    }

    #[wasm_bindgen(js_name = "mem_slice")]
    pub fn mem_slice_wasm(&self, start: u32, len: u32) -> Vec<u8> {
        self.mem_slice(start, len)
    }

    #[wasm_bindgen(js_name = "output")]
    pub fn output_wasm(&self) -> String {
        self.output()
    }

    #[wasm_bindgen(js_name = "translate")]
    pub fn translate_wasm(&self, va: u64, core_idx: u32) -> Result<JsValue, JsValue> {
        let r = self
            .translate(va, core_idx)
            .ok_or_else(|| JsValue::from_str("invalid core index"))?;
        to_js(&r)
    }

    #[wasm_bindgen(js_name = "entry_pc")]
    pub fn entry_pc_wasm(&self) -> u64 {
        self.entry_pc()
    }

    #[wasm_bindgen(js_name = "uart_addr")]
    pub fn uart_addr_wasm(&self) -> u64 {
        self.uart_addr()
    }

    #[wasm_bindgen(js_name = "l1_table_pa")]
    pub fn l1_table_pa_wasm(&self) -> u64 {
        self.l1_table_pa()
    }

    #[wasm_bindgen(js_name = "num_cores")]
    pub fn num_cores_wasm(&self) -> u32 {
        self.num_cores()
    }

    #[wasm_bindgen(js_name = "atomic_counter")]
    pub fn atomic_counter_wasm(&self) -> u64 {
        self.atomic_counter()
    }
}

/// Disassemble a single 32-bit AArch64 instruction. Mirrors the in-crate
/// decoder; useful for the UI's instruction listing.
#[wasm_bindgen(js_name = "disassemble")]
pub fn disassemble_wasm(insn: u32, pc: u64) -> String {
    crate::disassemble(insn, pc)
}
