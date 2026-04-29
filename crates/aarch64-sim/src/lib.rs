//! Tiny AArch64 multi-core simulator.
//!
//! v0.1 — MOVZ, ADD (imm), LDR/STR (unsigned offset), B + MMIO UART.
//! v0.2 — Stage-1 MMU translation walk (4 KiB granule, 39-bit VA).
//! v0.3 — MSR/MRS + SCTLR_EL1.M=1 routes fetch/load/store through MMU.
//! v0.4 — EL2 boot, ERET drops to EL1.
//! v0.5 — SVC + ESR_EL1 (full EL0 ↔ EL1 syscall round trip).
//! v0.6 — Two cores (P-core / E-core) sharing physical memory + UART.
//!        Per-core register file, EL state, and sysregs (incl. MPIDR_EL1).

use serde::Serialize;
use serde_wasm_bindgen::Serializer;
use wasm_bindgen::prelude::*;

/// Build a JsValue from any serde value, keeping u64/i64/u128/i128 as JS BigInt
/// rather than the lossy Number default serde-wasm-bindgen ships.
fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    let serializer = Serializer::new().serialize_large_number_types_as_bigints(true);
    value.serialize(&serializer).map_err(|e| JsValue::from_str(&e.to_string()))
}

const MEM_SIZE: usize = 0x10000;
const UART_OUT: u64 = 0x1000;
const ENTRY_PC: u64 = 0x4000;

// Demo page-table layout (4 KiB granule, T0SZ=25 → 39-bit VA, start at level 1).
const L1_TABLE_PA: u64 = 0x8000;
const L2_TABLE_PA: u64 = 0x9000;
const L3_TABLE_PA: u64 = 0xA000;

const NUM_CORES: usize = 2;

// MPIDR_EL1 values per core. Bit 31 is RES1 in ARMv8. We model two clusters:
// core 0 in cluster 0 (P-core, Aff1=0), core 1 in cluster 1 (E-core, Aff1=1).
const MPIDR_VALUES: [u64; NUM_CORES] = [0x8000_0000, 0x8000_0100];
const CORE_KIND: [&str; NUM_CORES] = ["P-core", "E-core"];

#[derive(Serialize, Clone)]
pub struct CoreState {
    pub id: u8,
    pub kind: String,
    pub mpidr: u64,
    pub x: [u64; 31],
    pub sp: u64,
    pub pc: u64,
    pub nzcv: u8,
    pub halted: bool,
    pub last_trap: Option<String>,
    pub steps: u64,
    pub current_el: u8,
    pub ttbr0_el1: u64,
    pub tcr_el1: u64,
    pub sctlr_el1: u64,
    pub vbar_el1: u64,
    pub elr_el1: u64,
    pub spsr_el1: u64,
    pub esr_el1: u64,
    pub vbar_el2: u64,
    pub elr_el2: u64,
    pub spsr_el2: u64,
    pub esr_el2: u64,
}

#[derive(Serialize, Clone)]
pub struct PageAttrs {
    pub af: bool,
    pub ap: u8,
    pub attr_idx: u8,
    pub sh: u8,
}

#[derive(Serialize, Clone)]
#[serde(tag = "kind")]
pub enum WalkOutcome {
    Table { next_table: u64 },
    Page { pa: u64, attrs: PageAttrs },
    Block { pa: u64, attrs: PageAttrs, span: u64 },
    Invalid,
    Fault { reason: String },
}

#[derive(Serialize, Clone)]
pub struct WalkStep {
    pub level: u8,
    pub table_addr: u64,
    pub index: u32,
    pub entry_addr: u64,
    pub descriptor: u64,
    pub outcome: WalkOutcome,
}

#[derive(Serialize, Clone)]
pub struct TranslationResult {
    pub va: u64,
    pub steps: Vec<WalkStep>,
    pub pa: Option<u64>,
    pub fault: Option<String>,
    pub mmu_enabled: bool,
}

// === Core: per-core register file + EL state + sysregs ===========================

struct Core {
    id: u8,
    mpidr: u64,
    x: [u64; 31],
    sp: u64,
    pc: u64,
    nzcv: u8,
    halted: bool,
    last_trap: Option<String>,
    steps: u64,
    current_el: u8,
    ttbr0_el1: u64,
    tcr_el1: u64,
    sctlr_el1: u64,
    vbar_el1: u64,
    elr_el1: u64,
    spsr_el1: u64,
    esr_el1: u64,
    vbar_el2: u64,
    elr_el2: u64,
    spsr_el2: u64,
    esr_el2: u64,
}

enum StepResult {
    Continue,
    Halt,
}

impl Core {
    fn new(id: u8, mpidr: u64) -> Self {
        Core {
            id,
            mpidr,
            x: [0; 31],
            sp: 0,
            pc: ENTRY_PC,
            nzcv: 0,
            halted: false,
            last_trap: None,
            steps: 0,
            current_el: 2,
            ttbr0_el1: 0,
            tcr_el1: 0,
            sctlr_el1: 0,
            vbar_el1: 0,
            elr_el1: 0,
            spsr_el1: 0,
            esr_el1: 0,
            vbar_el2: 0,
            elr_el2: 0,
            spsr_el2: 0,
            esr_el2: 0,
        }
    }

    fn reset(&mut self) {
        let saved_id = self.id;
        let saved_mpidr = self.mpidr;
        *self = Core::new(saved_id, saved_mpidr);
    }

    fn snapshot(&self) -> CoreState {
        CoreState {
            id: self.id,
            kind: CORE_KIND[self.id as usize].to_string(),
            mpidr: self.mpidr,
            x: self.x,
            sp: self.sp,
            pc: self.pc,
            nzcv: self.nzcv,
            halted: self.halted,
            last_trap: self.last_trap.clone(),
            steps: self.steps,
            current_el: self.current_el,
            ttbr0_el1: self.ttbr0_el1,
            tcr_el1: self.tcr_el1,
            sctlr_el1: self.sctlr_el1,
            vbar_el1: self.vbar_el1,
            elr_el1: self.elr_el1,
            spsr_el1: self.spsr_el1,
            esr_el1: self.esr_el1,
            vbar_el2: self.vbar_el2,
            elr_el2: self.elr_el2,
            spsr_el2: self.spsr_el2,
            esr_el2: self.esr_el2,
        }
    }

    fn read_x(&self, idx: usize) -> u64 {
        if idx == 31 { 0 } else { self.x[idx] }
    }

    fn write_x(&mut self, idx: usize, val: u64) {
        if idx < 31 {
            self.x[idx] = val;
        }
    }

    fn trap(&mut self, msg: String) {
        self.last_trap = Some(msg);
        self.halted = true;
    }

    fn read_sysreg(&self, sr: (u32, u32, u32, u32, u32)) -> Result<u64, String> {
        Ok(match sr {
            (3, 0, 2, 0, 0) => self.ttbr0_el1,
            (3, 0, 2, 0, 2) => self.tcr_el1,
            (3, 0, 1, 0, 0) => self.sctlr_el1,
            (3, 0, 12, 0, 0) => self.vbar_el1,
            (3, 0, 4, 0, 0) => self.spsr_el1,
            (3, 0, 4, 0, 1) => self.elr_el1,
            (3, 0, 5, 2, 0) => self.esr_el1,
            (3, 4, 12, 0, 0) => self.vbar_el2,
            (3, 4, 4, 0, 0) => self.spsr_el2,
            (3, 4, 4, 0, 1) => self.elr_el2,
            (3, 4, 5, 2, 0) => self.esr_el2,
            // CurrentEL[3:2] = current_el.
            (3, 0, 4, 2, 2) => (self.current_el as u64) << 2,
            // MPIDR_EL1 — read-only, identifies this core.
            (3, 0, 0, 0, 5) => self.mpidr,
            _ => return Err(unsupported_sysreg("MRS", sr, self.pc)),
        })
    }

    fn write_sysreg(&mut self, sr: (u32, u32, u32, u32, u32), val: u64) -> Result<(), String> {
        match sr {
            (3, 0, 2, 0, 0) => self.ttbr0_el1 = val,
            (3, 0, 2, 0, 2) => self.tcr_el1 = val,
            (3, 0, 1, 0, 0) => self.sctlr_el1 = val,
            (3, 0, 12, 0, 0) => self.vbar_el1 = val,
            (3, 0, 4, 0, 0) => self.spsr_el1 = val,
            (3, 0, 4, 0, 1) => self.elr_el1 = val,
            (3, 0, 5, 2, 0) => self.esr_el1 = val,
            (3, 4, 12, 0, 0) => self.vbar_el2 = val,
            (3, 4, 4, 0, 0) => self.spsr_el2 = val,
            (3, 4, 4, 0, 1) => self.elr_el2 = val,
            (3, 4, 5, 2, 0) => self.esr_el2 = val,
            (3, 0, 4, 2, 2) => return Err("MSR to CurrentEL (read-only)".into()),
            (3, 0, 0, 0, 5) => return Err("MSR to MPIDR_EL1 (read-only)".into()),
            _ => return Err(unsupported_sysreg("MSR", sr, self.pc)),
        }
        Ok(())
    }

    fn translate_for_access(&self, mem: &[u8], va: u64) -> Result<u64, String> {
        if self.sctlr_el1 & 1 == 0 {
            return Ok(va);
        }
        let r = self.do_translate(mem, va);
        r.pa.ok_or_else(|| r.fault.unwrap_or_else(|| "MMU fault".into()))
    }

    fn fetch_u32(&self, mem: &[u8], va: u64) -> Result<u32, String> {
        let pa = self.translate_for_access(mem, va)?;
        read_pa_u32(mem, pa)
    }

    fn load64(&self, mem: &[u8], va: u64) -> Result<u64, String> {
        let pa = self.translate_for_access(mem, va)?;
        read_pa_u64(mem, pa)
    }

    fn store64(
        &self,
        mem: &mut [u8],
        out: &mut Vec<u8>,
        va: u64,
        val: u64,
    ) -> Result<(), String> {
        let pa = self.translate_for_access(mem, va)?;
        if pa == UART_OUT {
            out.push((val & 0xFF) as u8);
        }
        write_pa_u64(mem, pa, val)
    }

    fn do_translate(&self, mem: &[u8], va: u64) -> TranslationResult {
        let mmu_enabled = self.sctlr_el1 & 1 != 0;
        let t0sz = self.tcr_el1 & 0x3F;
        if t0sz == 0 || self.ttbr0_el1 == 0 {
            return TranslationResult {
                va,
                steps: Vec::new(),
                pa: None,
                fault: Some("MMU not configured (TTBR0_EL1 or TCR_EL1.T0SZ unset)".into()),
                mmu_enabled,
            };
        }

        let va_bits = 64 - t0sz;
        let start_level: u8 = if va_bits >= 40 {
            0
        } else if va_bits >= 31 {
            1
        } else if va_bits >= 22 {
            2
        } else {
            3
        };

        let mut steps: Vec<WalkStep> = Vec::new();
        let mut table_addr = self.ttbr0_el1 & 0x0000_FFFF_FFFF_F000;
        let mut level = start_level;

        loop {
            let shift = 12 + 9 * (3 - level as u32);
            let index = ((va >> shift) & 0x1FF) as u32;
            let entry_addr = table_addr + (index as u64) * 8;
            let descriptor = match read_pa_u64(mem, entry_addr) {
                Ok(d) => d,
                Err(e) => {
                    steps.push(WalkStep {
                        level,
                        table_addr,
                        index,
                        entry_addr,
                        descriptor: 0,
                        outcome: WalkOutcome::Fault { reason: e.clone() },
                    });
                    return TranslationResult {
                        va,
                        steps,
                        pa: None,
                        fault: Some(e),
                        mmu_enabled,
                    };
                }
            };

            let valid = descriptor & 1 != 0;
            let typ = (descriptor >> 1) & 1;
            if !valid {
                steps.push(WalkStep {
                    level,
                    table_addr,
                    index,
                    entry_addr,
                    descriptor,
                    outcome: WalkOutcome::Invalid,
                });
                return TranslationResult {
                    va,
                    steps,
                    pa: None,
                    fault: Some(format!(
                        "translation fault at level {} (invalid descriptor)",
                        level
                    )),
                    mmu_enabled,
                };
            }

            if level == 3 {
                let pa_base = descriptor & 0x0000_FFFF_FFFF_F000;
                let pa = pa_base | (va & 0xFFF);
                let attrs = decode_attrs(descriptor);
                let af = attrs.af;
                steps.push(WalkStep {
                    level: 3,
                    table_addr,
                    index,
                    entry_addr,
                    descriptor,
                    outcome: WalkOutcome::Page { pa, attrs },
                });
                if !af {
                    return TranslationResult {
                        va,
                        steps,
                        pa: None,
                        fault: Some("access flag fault".into()),
                        mmu_enabled,
                    };
                }
                return TranslationResult {
                    va,
                    steps,
                    pa: Some(pa),
                    fault: None,
                    mmu_enabled,
                };
            }

            if typ == 1 {
                let next = descriptor & 0x0000_FFFF_FFFF_F000;
                steps.push(WalkStep {
                    level,
                    table_addr,
                    index,
                    entry_addr,
                    descriptor,
                    outcome: WalkOutcome::Table { next_table: next },
                });
                table_addr = next;
                level += 1;
            } else {
                let block_size = 1u64 << shift;
                let pa_base = descriptor & !(block_size - 1) & 0x0000_FFFF_FFFF_FFFF;
                let pa = pa_base | (va & (block_size - 1));
                let attrs = decode_attrs(descriptor);
                let af = attrs.af;
                steps.push(WalkStep {
                    level,
                    table_addr,
                    index,
                    entry_addr,
                    descriptor,
                    outcome: WalkOutcome::Block {
                        pa,
                        attrs,
                        span: block_size,
                    },
                });
                if !af {
                    return TranslationResult {
                        va,
                        steps,
                        pa: None,
                        fault: Some("access flag fault".into()),
                        mmu_enabled,
                    };
                }
                return TranslationResult {
                    va,
                    steps,
                    pa: Some(pa),
                    fault: None,
                    mmu_enabled,
                };
            }
        }
    }

    fn step(&mut self, mem: &mut [u8], out: &mut Vec<u8>) -> bool {
        if self.halted {
            return false;
        }
        let pc = self.pc;
        let insn = match self.fetch_u32(mem, pc) {
            Ok(v) => v,
            Err(e) => {
                self.trap(format!("{e} at pc={:#x}", pc));
                return false;
            }
        };
        match self.execute(insn, mem, out) {
            Ok(StepResult::Continue) => true,
            Ok(StepResult::Halt) => {
                self.halted = true;
                false
            }
            Err(e) => {
                self.trap(e);
                false
            }
        }
    }

    fn execute(
        &mut self,
        insn: u32,
        mem: &mut [u8],
        out: &mut Vec<u8>,
    ) -> Result<StepResult, String> {
        self.steps = self.steps.saturating_add(1);

        // MOVZ Xd, #imm16{, LSL #hw*16} :: 1 10 100101 hw imm16 Rd
        if insn & 0xFF80_0000 == 0xD280_0000 {
            let rd = (insn & 0x1F) as usize;
            let hw = ((insn >> 21) & 0x3) as u32;
            let imm = ((insn >> 5) & 0xFFFF) as u64;
            self.write_x(rd, imm << (hw * 16));
            self.pc = self.pc.wrapping_add(4);
            return Ok(StepResult::Continue);
        }

        // ADD Xd, Xn, #imm12{, LSL #12} :: 1 00 10001 sh imm12 Rn Rd
        if insn & 0xFF80_0000 == 0x9100_0000 {
            let rd = (insn & 0x1F) as usize;
            let rn = ((insn >> 5) & 0x1F) as usize;
            let imm12 = ((insn >> 10) & 0xFFF) as u64;
            let sh = ((insn >> 22) & 0x1) as u32;
            let imm = if sh == 1 { imm12 << 12 } else { imm12 };
            let val = self.read_x(rn).wrapping_add(imm);
            self.write_x(rd, val);
            self.pc = self.pc.wrapping_add(4);
            return Ok(StepResult::Continue);
        }

        // STR Xt, [Xn, #imm12]
        if insn & 0xFFC0_0000 == 0xF900_0000 {
            let rt = (insn & 0x1F) as usize;
            let rn = ((insn >> 5) & 0x1F) as usize;
            let imm12 = ((insn >> 10) & 0xFFF) as u64;
            let addr = self.read_x(rn).wrapping_add(imm12 * 8);
            let val = self.read_x(rt);
            self.store64(mem, out, addr, val)?;
            self.pc = self.pc.wrapping_add(4);
            return Ok(StepResult::Continue);
        }

        // LDR Xt, [Xn, #imm12]
        if insn & 0xFFC0_0000 == 0xF940_0000 {
            let rt = (insn & 0x1F) as usize;
            let rn = ((insn >> 5) & 0x1F) as usize;
            let imm12 = ((insn >> 10) & 0xFFF) as u64;
            let addr = self.read_x(rn).wrapping_add(imm12 * 8);
            let val = self.load64(mem, addr)?;
            self.write_x(rt, val);
            self.pc = self.pc.wrapping_add(4);
            return Ok(StepResult::Continue);
        }

        // MSR/MRS sysreg
        if insn & 0xFFC0_0000 == 0xD500_0000 {
            let l = (insn >> 21) & 1;
            let op0 = (insn >> 19) & 0x3;
            let op1 = (insn >> 16) & 0x7;
            let crn = (insn >> 12) & 0xF;
            let crm = (insn >> 8) & 0xF;
            let op2 = (insn >> 5) & 0x7;
            let rt = (insn & 0x1F) as usize;

            if op0 < 2 {
                // Hint / barrier — no-op.
                if insn & 0xFFFF_F01F == 0xD503_201F || insn & 0xFFFF_F01F == 0xD503_301F {
                    self.pc = self.pc.wrapping_add(4);
                    return Ok(StepResult::Continue);
                }
                return Err(format!(
                    "unsupported system instruction {:#010x} at pc={:#x}",
                    insn, self.pc
                ));
            }

            let sr = (op0, op1, crn, crm, op2);
            if l == 0 {
                let val = self.read_x(rt);
                self.write_sysreg(sr, val)?;
            } else {
                let val = self.read_sysreg(sr)?;
                self.write_x(rt, val);
            }
            self.pc = self.pc.wrapping_add(4);
            return Ok(StepResult::Continue);
        }

        // SVC #imm16 — sync exception from EL0 to EL1.
        if insn & 0xFFE0_001F == 0xD400_0001 {
            let imm16 = ((insn >> 5) & 0xFFFF) as u16;
            if self.current_el != 0 {
                return Err(format!(
                    "SVC from EL{} not modeled at pc={:#x}",
                    self.current_el, self.pc
                ));
            }
            self.elr_el1 = self.pc.wrapping_add(4);
            self.spsr_el1 = 0;
            self.esr_el1 = (0x15u64 << 26) | (1 << 25) | (imm16 as u64);
            self.current_el = 1;
            self.pc = self.vbar_el1.wrapping_add(0x400);
            return Ok(StepResult::Continue);
        }

        // ERET
        if insn == 0xD69F_03E0 {
            let (elr, spsr) = match self.current_el {
                2 => (self.elr_el2, self.spsr_el2),
                1 => (self.elr_el1, self.spsr_el1),
                _ => {
                    return Err(format!(
                        "ERET from EL{} (no exception state) at pc={:#x}",
                        self.current_el, self.pc
                    ));
                }
            };
            let new_el = ((spsr >> 2) & 0x3) as u8;
            if new_el > self.current_el {
                return Err(format!(
                    "ERET would raise EL{} → EL{} (illegal) at pc={:#x}",
                    self.current_el, new_el, self.pc
                ));
            }
            self.current_el = new_el;
            self.pc = elr;
            return Ok(StepResult::Continue);
        }

        // B label
        if insn & 0xFC00_0000 == 0x1400_0000 {
            let imm26_raw = (insn & 0x03FF_FFFF) as i32;
            let imm26 = (imm26_raw << 6) >> 6;
            let offset = (imm26 as i64) * 4;
            let target = (self.pc as i64).wrapping_add(offset) as u64;
            if target == self.pc {
                return Ok(StepResult::Halt);
            }
            self.pc = target;
            return Ok(StepResult::Continue);
        }

        Err(format!("undefined instruction {:#010x} at pc={:#x}", insn, self.pc))
    }
}

// === Cpu: the system shell — N cores + shared memory + UART buffer ============

#[wasm_bindgen]
pub struct Cpu {
    cores: Vec<Core>,
    mem: Vec<u8>,
    output_buf: Vec<u8>,
}

#[wasm_bindgen]
impl Cpu {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Cpu {
        let cores = (0..NUM_CORES as u8)
            .map(|i| Core::new(i, MPIDR_VALUES[i as usize]))
            .collect();
        let mut sys = Cpu {
            cores,
            mem: vec![0u8; MEM_SIZE],
            output_buf: Vec::new(),
        };
        load_demo(&mut sys.mem);
        setup_demo_pgtable(&mut sys.mem);
        sys
    }

    pub fn reset(&mut self) {
        for c in self.cores.iter_mut() {
            c.reset();
        }
        for b in self.mem.iter_mut() {
            *b = 0;
        }
        self.output_buf.clear();
        load_demo(&mut self.mem);
        setup_demo_pgtable(&mut self.mem);
    }

    /// Step every core once (deterministic order, core 0 first). Returns true
    /// if any core was runnable (i.e., made forward progress).
    pub fn step(&mut self) -> bool {
        let mut any = false;
        for core in self.cores.iter_mut() {
            if core.step(&mut self.mem, &mut self.output_buf) {
                any = true;
            }
        }
        any
    }

    /// Step a single core. Useful for "advance only this core" UI controls.
    pub fn step_core(&mut self, idx: u32) -> bool {
        if let Some(core) = self.cores.get_mut(idx as usize) {
            core.step(&mut self.mem, &mut self.output_buf)
        } else {
            false
        }
    }

    pub fn run(&mut self, max: u32) -> u32 {
        let mut n = 0u32;
        while n < max && self.step() {
            n += 1;
        }
        n
    }

    /// Returns an array of CoreState (one per core) as a JS Array.
    pub fn state(&self) -> Result<JsValue, JsValue> {
        let states: Vec<CoreState> = self.cores.iter().map(|c| c.snapshot()).collect();
        to_js(&states)
    }

    pub fn mem_slice(&self, start: u32, len: u32) -> Vec<u8> {
        let s = (start as usize).min(self.mem.len());
        let e = (s + len as usize).min(self.mem.len());
        self.mem[s..e].to_vec()
    }

    pub fn output(&self) -> String {
        String::from_utf8_lossy(&self.output_buf).into_owned()
    }

    /// Walk page tables for `va` using the sysregs of `core_idx`.
    pub fn translate(&self, va: u64, core_idx: u32) -> Result<JsValue, JsValue> {
        let core = self
            .cores
            .get(core_idx as usize)
            .ok_or_else(|| JsValue::from_str("invalid core index"))?;
        to_js(&core.do_translate(&self.mem, va))
    }

    pub fn entry_pc(&self) -> u64 {
        ENTRY_PC
    }

    pub fn uart_addr(&self) -> u64 {
        UART_OUT
    }

    pub fn l1_table_pa(&self) -> u64 {
        L1_TABLE_PA
    }

    pub fn num_cores(&self) -> u32 {
        self.cores.len() as u32
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

// === Shared-memory helpers (free functions) ===================================

fn read_pa_u32(mem: &[u8], pa: u64) -> Result<u32, String> {
    let a = pa as usize;
    if a + 4 > mem.len() {
        return Err(format!("fetch fault at PA {:#x}", pa));
    }
    Ok(u32::from_le_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]]))
}

fn read_pa_u64(mem: &[u8], pa: u64) -> Result<u64, String> {
    let a = pa as usize;
    if a + 8 > mem.len() {
        return Err(format!("load fault at PA {:#x}", pa));
    }
    Ok(u64::from_le_bytes([
        mem[a],
        mem[a + 1],
        mem[a + 2],
        mem[a + 3],
        mem[a + 4],
        mem[a + 5],
        mem[a + 6],
        mem[a + 7],
    ]))
}

fn write_pa_u64(mem: &mut [u8], pa: u64, val: u64) -> Result<(), String> {
    let a = pa as usize;
    if a + 8 > mem.len() {
        return Err(format!("store fault at PA {:#x}", pa));
    }
    mem[a..a + 8].copy_from_slice(&val.to_le_bytes());
    Ok(())
}

fn write_u64(mem: &mut [u8], addr: u64, val: u64) {
    let a = addr as usize;
    mem[a..a + 8].copy_from_slice(&val.to_le_bytes());
}

fn write_words(mem: &mut [u8], base: u64, words: &[u32]) {
    let mut off = base as usize;
    for w in words.iter() {
        mem[off..off + 4].copy_from_slice(&w.to_le_bytes());
        off += 4;
    }
}

fn decode_attrs(desc: u64) -> PageAttrs {
    PageAttrs {
        af: (desc >> 10) & 1 != 0,
        ap: ((desc >> 6) & 0x3) as u8,
        attr_idx: ((desc >> 2) & 0x7) as u8,
        sh: ((desc >> 8) & 0x3) as u8,
    }
}

fn unsupported_sysreg(op: &str, sr: (u32, u32, u32, u32, u32), pc: u64) -> String {
    format!(
        "{op} of unsupported sysreg S{}_{}_C{}_C{}_{} at pc={:#x}",
        sr.0, sr.1, sr.2, sr.3, sr.4, pc
    )
}

// === Demo program =============================================================

fn load_demo(mem: &mut [u8]) {
    // Three regions, both cores execute the same code:
    //   PA 0x4000  kernel boot — EL2 → EL1, MMU bring-up, drop to EL0
    //   PA 0x4800  sync handler at VBAR_EL1+0x400 — writes 'K', ERET
    //   PA 0x4C00  user code at EL0 — writes 'U', SVC, '\n', halt
    //
    // With two cores, output becomes interleaved: typically "UUKK\n\n".
    const SPSR_EL1H_DAIF: u32 = 0x3C5;
    const VBAR: u32 = 0x4400;
    const HANDLER_PA: u64 = 0x4800;
    const USER_PA: u64 = 0x4C00;
    const EL1_ENTRY: u32 = ENTRY_PC as u32 + 5 * 4;

    let kernel: [u32; 19] = [
        // EL2: arrange ERET to EL1
        movz(9, EL1_ENTRY, 0),
        msr_elr_el2(9),
        movz(9, SPSR_EL1H_DAIF, 0),
        msr_spsr_el2(9),
        eret(),
        // EL1: bring up MMU
        movz(9, L1_TABLE_PA as u32, 0),
        msr_ttbr0(9),
        movz(9, 25, 0),
        msr_tcr(9),
        movz(9, 1, 0),
        msr_sctlr(9),
        isb(),
        // EL1: install vector base, ERET into EL0
        movz(9, VBAR, 0),
        msr_vbar_el1(9),
        movz(9, USER_PA as u32, 0),
        msr_elr_el1(9),
        movz(9, 0, 0),
        msr_spsr_el1(9),
        eret(),
    ];
    let handler: [u32; 5] = [
        movz(1, UART_OUT as u32, 0),
        movz(0, b'K' as u32, 0),
        str_imm(0, 1, 0),
        add_imm(0, 0, 0),
        eret(),
    ];
    let user: [u32; 8] = [
        movz(1, UART_OUT as u32, 0),
        movz(0, b'U' as u32, 0),
        str_imm(0, 1, 0),
        svc_imm(0),
        movz(0, b'\n' as u32, 0),
        str_imm(0, 1, 0),
        add_imm(0, 0, 0),
        b_self(),
    ];
    write_words(mem, ENTRY_PC, &kernel);
    write_words(mem, HANDLER_PA, &handler);
    write_words(mem, USER_PA, &user);
}

fn setup_demo_pgtable(mem: &mut [u8]) {
    write_u64(mem, L1_TABLE_PA, L2_TABLE_PA | 0b11);
    write_u64(mem, L2_TABLE_PA, L3_TABLE_PA | 0b11);
    let page_attr = (1u64 << 10) | 0b11;
    // VA 0x1000 → PA 0x1000 (UART)
    write_u64(mem, L3_TABLE_PA + 8, 0x1000 | page_attr);
    // VA 0x4000 → PA 0x4000 (program page)
    write_u64(mem, L3_TABLE_PA + 4 * 8, 0x4000 | page_attr);
    // page tables themselves
    write_u64(mem, L3_TABLE_PA + 8 * 8, 0x8000 | page_attr);
    write_u64(mem, L3_TABLE_PA + 9 * 8, 0x9000 | page_attr);
    write_u64(mem, L3_TABLE_PA + 0xA * 8, 0xA000 | page_attr);
}

// === Instruction encoders =====================================================

const fn movz(rd: u32, imm16: u32, hw: u32) -> u32 {
    0xD280_0000 | ((hw & 0x3) << 21) | ((imm16 & 0xFFFF) << 5) | (rd & 0x1F)
}

const fn add_imm(rd: u32, rn: u32, imm12: u32) -> u32 {
    0x9100_0000 | ((imm12 & 0xFFF) << 10) | ((rn & 0x1F) << 5) | (rd & 0x1F)
}

const fn str_imm(rt: u32, rn: u32, imm12: u32) -> u32 {
    0xF900_0000 | ((imm12 & 0xFFF) << 10) | ((rn & 0x1F) << 5) | (rt & 0x1F)
}

#[allow(dead_code)]
const fn ldr_imm(rt: u32, rn: u32, imm12: u32) -> u32 {
    0xF940_0000 | ((imm12 & 0xFFF) << 10) | ((rn & 0x1F) << 5) | (rt & 0x1F)
}

const fn b_self() -> u32 {
    0x1400_0000
}

const fn msr_sysreg(rt: u32, op0: u32, op1: u32, crn: u32, crm: u32, op2: u32) -> u32 {
    0xD500_0000
        | ((op0 & 0x3) << 19)
        | ((op1 & 0x7) << 16)
        | ((crn & 0xF) << 12)
        | ((crm & 0xF) << 8)
        | ((op2 & 0x7) << 5)
        | (rt & 0x1F)
}

const fn msr_ttbr0(rt: u32) -> u32 {
    msr_sysreg(rt, 3, 0, 2, 0, 0)
}
const fn msr_tcr(rt: u32) -> u32 {
    msr_sysreg(rt, 3, 0, 2, 0, 2)
}
const fn msr_sctlr(rt: u32) -> u32 {
    msr_sysreg(rt, 3, 0, 1, 0, 0)
}
const fn msr_elr_el2(rt: u32) -> u32 {
    msr_sysreg(rt, 3, 4, 4, 0, 1)
}
const fn msr_spsr_el2(rt: u32) -> u32 {
    msr_sysreg(rt, 3, 4, 4, 0, 0)
}
const fn msr_vbar_el1(rt: u32) -> u32 {
    msr_sysreg(rt, 3, 0, 12, 0, 0)
}
const fn msr_elr_el1(rt: u32) -> u32 {
    msr_sysreg(rt, 3, 0, 4, 0, 1)
}
const fn msr_spsr_el1(rt: u32) -> u32 {
    msr_sysreg(rt, 3, 0, 4, 0, 0)
}

const fn isb() -> u32 {
    0xD503_3FDF
}

const fn eret() -> u32 {
    0xD69F_03E0
}

const fn svc_imm(imm16: u32) -> u32 {
    0xD400_0001 | ((imm16 & 0xFFFF) << 5)
}

// === Tests ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boots_two_cores_at_el2() {
        let cpu = Cpu::new();
        assert_eq!(cpu.cores.len(), 2);
        assert_eq!(cpu.cores[0].current_el, 2);
        assert_eq!(cpu.cores[1].current_el, 2);
        assert_eq!(cpu.cores[0].pc, ENTRY_PC);
        assert_eq!(cpu.cores[1].pc, ENTRY_PC);
        assert_eq!(cpu.cores[0].mpidr, 0x8000_0000);
        assert_eq!(cpu.cores[1].mpidr, 0x8000_0100);
    }

    #[test]
    fn both_cores_complete_round_trip() {
        let mut cpu = Cpu::new();
        cpu.run(2000);
        assert!(cpu.cores[0].halted, "core 0 didn't halt");
        assert!(cpu.cores[1].halted, "core 1 didn't halt");
        assert!(cpu.cores[0].last_trap.is_none(), "core 0 trap: {:?}", cpu.cores[0].last_trap);
        assert!(cpu.cores[1].last_trap.is_none(), "core 1 trap: {:?}", cpu.cores[1].last_trap);
        // Both cores wrote U, K, \n. Order is core-0-first per step():
        //   step n:  core0-U, core1-U → "UU"
        //   step n+1: SVC for both, handler runs both, K K → "UUKK"
        //   final:    \n \n → "UUKK\n\n"
        assert_eq!(cpu.output(), "UUKK\n\n");
        assert_eq!(cpu.cores[0].current_el, 0);
        assert_eq!(cpu.cores[1].current_el, 0);
    }

    #[test]
    fn mpidr_is_per_core() {
        let mut cpu = Cpu::new();
        // Run until both cores enter EL1. After 5 steps the EL2 prologue is done.
        cpu.run(5);
        // Core 0 reads MPIDR_EL1 (S3_0_C0_C0_5) — using the read_sysreg path directly.
        let v0 = cpu.cores[0].read_sysreg((3, 0, 0, 0, 5)).unwrap();
        let v1 = cpu.cores[1].read_sysreg((3, 0, 0, 0, 5)).unwrap();
        assert_eq!(v0, 0x8000_0000);
        assert_eq!(v1, 0x8000_0100);
        // Writing MPIDR is forbidden.
        assert!(cpu.cores[0].write_sysreg((3, 0, 0, 0, 5), 0).is_err());
    }

    #[test]
    fn translate_after_boot() {
        let mut cpu = Cpu::new();
        // Run kernel boot prefix on both cores: 5 EL2 + 7 MMU = 12 instructions per core.
        cpu.run(12);
        // Both cores share the same page tables (same TTBR0 PA).
        let r0 = cpu.cores[0].do_translate(&cpu.mem, 0x4000);
        let r1 = cpu.cores[1].do_translate(&cpu.mem, 0x4000);
        assert_eq!(r0.pa, Some(0x4000));
        assert_eq!(r1.pa, Some(0x4000));
    }

    #[test]
    fn movz_then_add() {
        let mut cpu = Cpu::new();
        // Overwrite memory with our own tiny program that runs at EL2 (no MMU).
        let prog = [movz(0, 5, 0), add_imm(0, 0, 7), b_self()];
        write_words(&mut cpu.mem, ENTRY_PC, &prog);
        // Disable MMU on core 0 so this program can run identity-mapped.
        cpu.cores[0].sctlr_el1 = 0;
        cpu.run(20);
        // Core 0 should have computed X0 = 12 then halted on B .
        assert_eq!(cpu.cores[0].x[0], 12);
    }
}
