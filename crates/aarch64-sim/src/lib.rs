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

// AIC timer: fires an IRQ on every core every TIMER_PERIOD system steps.
// Picked small so context switches show up quickly in interactive Step mode.
const TIMER_PERIOD: u64 = 30;

// AIC MMIO layout (loosely modelled on Apple's per-core AIC view): software
// reads from one MMIO base and the controller routes the call by which core
// issued it. We expose two registers:
//   AIC_BASE + 0x00  → ACK (read-only): returns the lowest pending IRQ id for
//                      the calling core and clears that bit. 0xFFFF_FFFF when
//                      nothing pending.
//   AIC_BASE + 0x10  → IPI_SET (write-only): target core id; raises IRQ_IPI on
//                      that core.
const AIC_BASE: u64 = 0x2000;
const AIC_END: u64 = 0x2100;
const AIC_REG_ACK: u64 = 0x00;
const AIC_REG_IPI_SET: u64 = 0x10;

const IRQ_TIMER: u32 = 0;
const IRQ_IPI: u32 = 1;
const IRQ_NONE: u32 = 0xFFFF_FFFF;

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
    pub daif: u8,
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

#[derive(Serialize, Clone)]
pub struct AicState {
    /// Per-core pending bitmap. Bit 0 = IRQ_TIMER, bit 1 = IRQ_IPI.
    pub pending: Vec<u32>,
    pub total_acks: u64,
}

// === Aic: tiny Apple-style interrupt controller ===============================

struct Aic {
    pending: [u32; NUM_CORES],
    total_acks: u64,
}

impl Aic {
    fn new() -> Self {
        Aic {
            pending: [0; NUM_CORES],
            total_acks: 0,
        }
    }

    fn reset(&mut self) {
        for p in self.pending.iter_mut() {
            *p = 0;
        }
        self.total_acks = 0;
    }

    fn set_irq(&mut self, core: usize, irq_id: u32) {
        if core < NUM_CORES && irq_id < 32 {
            self.pending[core] |= 1u32 << irq_id;
        }
    }

    fn has_pending(&self, core: usize) -> bool {
        core < NUM_CORES && self.pending[core] != 0
    }

    /// MMIO ACK read by `core`: lowest pending IRQ id, clears it. Returns
    /// IRQ_NONE if nothing pending.
    fn read_ack(&mut self, core: usize) -> u32 {
        if core >= NUM_CORES || self.pending[core] == 0 {
            return IRQ_NONE;
        }
        let irq = self.pending[core].trailing_zeros();
        self.pending[core] &= !(1u32 << irq);
        self.total_acks = self.total_acks.saturating_add(1);
        irq
    }

    fn mmio_read(&mut self, core: usize, offset: u64) -> u64 {
        match offset {
            AIC_REG_ACK => self.read_ack(core) as u64,
            _ => 0,
        }
    }

    fn mmio_write(&mut self, _core: usize, offset: u64, val: u64) {
        match offset {
            AIC_REG_IPI_SET => {
                let target = val as usize;
                self.set_irq(target, IRQ_IPI);
            }
            _ => {}
        }
    }

    fn snapshot(&self) -> AicState {
        AicState {
            pending: self.pending.to_vec(),
            total_acks: self.total_acks,
        }
    }
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
    /// Low 4 bits of PSTATE.DAIF — D, A, I, F (bit 3 → bit 0). 1 = masked.
    /// On reset (EL2) we mask everything; SVC/IRQ entries also re-mask.
    daif: u8,
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
            daif: 0xF, // boot at EL2 with all interrupts masked
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
            daif: self.daif,
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

    /// Pack the current EL+SP context and DAIF into an SPSR-shaped value, suitable
    /// for storing in SPSR_EL<x> on exception entry.
    fn build_spsr(&self) -> u64 {
        let m_low: u64 = match self.current_el {
            0 => 0b0000,           // EL0t (no SP_ELx)
            1 => 0b0101,           // EL1h (uses SP_EL1)
            2 => 0b1001,           // EL2h
            _ => 0,
        };
        m_low | ((self.daif as u64) << 6)
    }

    /// Take an asynchronous IRQ exception into EL1, vectoring to VBAR_EL1+0x480.
    /// Caller is responsible for verifying DAIF.I is clear and irq_pending is set.
    fn take_irq(&mut self) {
        self.elr_el1 = self.pc; // resume at the not-yet-fetched instruction
        self.spsr_el1 = self.build_spsr();
        self.esr_el1 = 0; // IRQ has no syndrome
        self.current_el = 1;
        self.daif = 0xF; // exception entry masks everything
        self.pc = self.vbar_el1.wrapping_add(0x480);
    }

    fn irq_masked(&self) -> bool {
        self.daif & 0b0010 != 0 // I bit (bit 1 of low nibble: D=8, A=4, I=2, F=1)
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

    fn load64(&self, mem: &[u8], aic: &mut Aic, va: u64) -> Result<u64, String> {
        let pa = self.translate_for_access(mem, va)?;
        if (AIC_BASE..AIC_END).contains(&pa) {
            return Ok(aic.mmio_read(self.id as usize, pa - AIC_BASE));
        }
        read_pa_u64(mem, pa)
    }

    fn store64(
        &self,
        mem: &mut [u8],
        out: &mut Vec<u8>,
        aic: &mut Aic,
        va: u64,
        val: u64,
    ) -> Result<(), String> {
        let pa = self.translate_for_access(mem, va)?;
        if (AIC_BASE..AIC_END).contains(&pa) {
            aic.mmio_write(self.id as usize, pa - AIC_BASE, val);
            return Ok(());
        }
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

    fn step(&mut self, mem: &mut [u8], out: &mut Vec<u8>, aic: &mut Aic) -> bool {
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
        match self.execute(insn, mem, out, aic) {
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
        aic: &mut Aic,
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

        // ADD Xd, Xn, Xm  (shifted register, LSL #0) :: 1 0 0 01011 00 0 Rm 000000 Rn Rd
        if insn & 0xFF20_FC00 == 0x8B00_0000 {
            let rd = (insn & 0x1F) as usize;
            let rn = ((insn >> 5) & 0x1F) as usize;
            let rm = ((insn >> 16) & 0x1F) as usize;
            let val = self.read_x(rn).wrapping_add(self.read_x(rm));
            self.write_x(rd, val);
            self.pc = self.pc.wrapping_add(4);
            return Ok(StepResult::Continue);
        }

        // SUB Xd, Xn, Xm  (shifted register, LSL #0) :: 1 1 0 01011 00 0 Rm 000000 Rn Rd
        if insn & 0xFF20_FC00 == 0xCB00_0000 {
            let rd = (insn & 0x1F) as usize;
            let rn = ((insn >> 5) & 0x1F) as usize;
            let rm = ((insn >> 16) & 0x1F) as usize;
            let val = self.read_x(rn).wrapping_sub(self.read_x(rm));
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
            self.store64(mem, out, aic, addr, val)?;
            self.pc = self.pc.wrapping_add(4);
            return Ok(StepResult::Continue);
        }

        // LDR Xt, [Xn, #imm12]
        if insn & 0xFFC0_0000 == 0xF940_0000 {
            let rt = (insn & 0x1F) as usize;
            let rn = ((insn >> 5) & 0x1F) as usize;
            let imm12 = ((insn >> 10) & 0xFFF) as u64;
            let addr = self.read_x(rn).wrapping_add(imm12 * 8);
            let val = self.load64(mem, aic, addr)?;
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
                // MSR <pstatefield>, #imm — same major class with op0=00, op1=011,
                // CRn=0100. Distinguished from DAIFSet/DAIFClr by op2.
                //   op2=110 → MSR DAIFSet, #imm4
                //   op2=111 → MSR DAIFClr, #imm4
                // The imm4 (D=8/A=4/I=2/F=1) maps directly onto our internal
                // 4-bit daif representation.
                if op0 == 0 && op1 == 3 && crn == 4 && (op2 == 6 || op2 == 7) {
                    let imm4 = (crm & 0xF) as u8;
                    if op2 == 6 {
                        self.daif |= imm4; // DAIFSet
                    } else {
                        self.daif &= !imm4; // DAIFClr
                    }
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
            self.spsr_el1 = self.build_spsr();
            self.esr_el1 = (0x15u64 << 26) | (1 << 25) | (imm16 as u64);
            self.current_el = 1;
            self.daif = 0xF; // exception entry masks DAIF
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
            let new_daif = ((spsr >> 6) & 0xF) as u8;
            self.current_el = new_el;
            self.daif = new_daif;
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
    aic: Aic,
    /// Number of Cpu::step() calls since reset.
    system_steps: u64,
    /// system_steps value at which the next timer IRQ fires.
    timer_next: u64,
    /// Total number of timer ticks observed; mostly for the UI.
    timer_ticks: u64,
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
            aic: Aic::new(),
            system_steps: 0,
            timer_next: TIMER_PERIOD,
            timer_ticks: 0,
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
        self.aic.reset();
        self.system_steps = 0;
        self.timer_next = TIMER_PERIOD;
        self.timer_ticks = 0;
        load_demo(&mut self.mem);
        setup_demo_pgtable(&mut self.mem);
    }

    /// Step every core once. On the way in: bump system_steps; if the timer
    /// is due, broadcast IRQ_TIMER to all cores via AIC. Each core then either
    /// takes a pending IRQ (when DAIF.I is clear) or executes one instruction.
    pub fn step(&mut self) -> bool {
        self.system_steps = self.system_steps.saturating_add(1);
        if self.system_steps >= self.timer_next {
            for i in 0..NUM_CORES {
                self.aic.set_irq(i, IRQ_TIMER);
            }
            self.timer_next = self.system_steps + TIMER_PERIOD;
            self.timer_ticks = self.timer_ticks.saturating_add(1);
        }

        let mut any = false;
        for i in 0..self.cores.len() {
            if self.cores[i].halted {
                continue;
            }
            if self.aic.has_pending(i) && !self.cores[i].irq_masked() {
                self.cores[i].take_irq();
                any = true;
            } else if self.cores[i].step(&mut self.mem, &mut self.output_buf, &mut self.aic) {
                any = true;
            }
        }
        any
    }

    /// Step a single core. Honours pending IRQs on that core (set either by
    /// the system timer in `step()` or by another core via IPI MMIO).
    pub fn step_core(&mut self, idx: u32) -> bool {
        let i = idx as usize;
        if i >= self.cores.len() {
            return false;
        }
        if self.cores[i].halted {
            return false;
        }
        if self.aic.has_pending(i) && !self.cores[i].irq_masked() {
            self.cores[i].take_irq();
            true
        } else {
            self.cores[i].step(&mut self.mem, &mut self.output_buf, &mut self.aic)
        }
    }

    pub fn run(&mut self, max: u32) -> u32 {
        let mut n = 0u32;
        while n < max && self.step() {
            n += 1;
        }
        n
    }

    pub fn system_steps(&self) -> u64 {
        self.system_steps
    }

    pub fn timer_period(&self) -> u64 {
        TIMER_PERIOD
    }

    /// System steps until the next timer IRQ fires (0 if it's due now).
    pub fn timer_remaining(&self) -> u64 {
        self.timer_next.saturating_sub(self.system_steps)
    }

    pub fn timer_ticks(&self) -> u64 {
        self.timer_ticks
    }

    pub fn aic_state(&self) -> Result<JsValue, JsValue> {
        to_js(&self.aic.snapshot())
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
    // Memory regions, both cores execute the same code:
    //   PA 0x4000  kernel boot — EL2 → EL1, MMU bring-up, init scheduler slot,
    //              drop to EL0 at task A.
    //   PA 0x4800  sync handler at VBAR_EL1+0x400 — leftover SVC handler,
    //              currently unused by the new tasks.
    //   PA 0x4880  IRQ handler at VBAR_EL1+0x480 — the scheduler. ACKs the AIC,
    //              flips the global "current task" slot, and ERETs into the
    //              other task.
    //   PA 0x4D00  task A (EL0) — spins printing 'A' to UART.
    //   PA 0x4E00  task B (EL0) — spins printing 'B' to UART.
    //   PA 0x4F00  current-task slot (8 bytes) — holds the currently running
    //              task entry. Initialised to TASK_A_ENTRY by kernel boot.
    //
    // Both cores boot with the same code and write the same value to the slot,
    // so they start out running task A in lockstep. Each timer tick (broadcast
    // by the AIC to all cores) makes the scheduler swap them to the other task.
    const SPSR_EL1H_DAIF: u32 = 0x3C5;
    const VBAR: u32 = 0x4400;
    const SYNC_HANDLER_PA: u64 = 0x4800;
    const IRQ_HANDLER_PA: u64 = 0x4880;
    const TASK_A_ENTRY: u32 = 0x4D00;
    const TASK_B_ENTRY: u32 = 0x4E00;
    const TASK_SUM: u32 = TASK_A_ENTRY + TASK_B_ENTRY; // 0x9B00 — fits in 16 bits
    const TASK_SLOT_PA: u32 = 0x4F00;
    const EL1_ENTRY: u32 = ENTRY_PC as u32 + 5 * 4;

    let kernel: [u32; 21] = [
        // EL2 prologue → ERET to EL1
        movz(9, EL1_ENTRY, 0),
        msr_elr_el2(9),
        movz(9, SPSR_EL1H_DAIF, 0),
        msr_spsr_el2(9),
        eret(),
        // EL1: MMU bring-up
        movz(9, L1_TABLE_PA as u32, 0),
        msr_ttbr0(9),
        movz(9, 25, 0),
        msr_tcr(9),
        movz(9, 1, 0),
        msr_sctlr(9),
        isb(),
        // EL1: install vector base
        movz(9, VBAR, 0),
        msr_vbar_el1(9),
        // EL1: initialise the scheduler slot to task A and ERET into it
        movz(9, TASK_A_ENTRY, 0),
        movz(10, TASK_SLOT_PA, 0),
        str_imm(9, 10, 0),
        msr_elr_el1(9),
        movz(10, 0, 0),
        msr_spsr_el1(10),
        eret(),
    ];
    // Sync handler (VBAR+0x400) — kept as a stub so a stray SVC traps to a
    // visible 'K' rather than going somewhere undefined.
    let sync_handler: [u32; 5] = [
        movz(1, UART_OUT as u32, 0),
        movz(0, b'K' as u32, 0),
        str_imm(0, 1, 0),
        add_imm(0, 0, 0),
        eret(),
    ];
    // IRQ handler (VBAR+0x480) — the scheduler. Reads AIC ACK to clear the
    // pending IRQ, computes the "other" task entry as (A+B)-current, swaps
    // the slot, and ERETs into the new task.
    let scheduler: [u32; 9] = [
        movz(9, AIC_BASE as u32, 0),     // X9 = AIC base
        ldr_imm(10, 9, 0),                // X10 = ACK (clears pending)
        movz(9, TASK_SLOT_PA, 0),         // X9 = slot addr
        ldr_imm(11, 9, 0),                // X11 = current task entry
        movz(12, TASK_SUM, 0),            // X12 = A_entry + B_entry
        sub_reg(12, 12, 11),              // X12 = the other entry
        str_imm(12, 9, 0),                // *slot = other
        msr_elr_el1(12),                  // ELR_EL1 = other (next task)
        eret(),                           // → EL0 at the other task
    ];
    let task_a: [u32; 5] = [
        movz(1, UART_OUT as u32, 0),
        movz(0, b'A' as u32, 0),
        str_imm(0, 1, 0),
        add_imm(0, 0, 0),
        b_offset(-1),
    ];
    let task_b: [u32; 5] = [
        movz(1, UART_OUT as u32, 0),
        movz(0, b'B' as u32, 0),
        str_imm(0, 1, 0),
        add_imm(0, 0, 0),
        b_offset(-1),
    ];

    write_words(mem, ENTRY_PC, &kernel);
    write_words(mem, SYNC_HANDLER_PA, &sync_handler);
    write_words(mem, IRQ_HANDLER_PA, &scheduler);
    write_words(mem, TASK_A_ENTRY as u64, &task_a);
    write_words(mem, TASK_B_ENTRY as u64, &task_b);
}

fn setup_demo_pgtable(mem: &mut [u8]) {
    write_u64(mem, L1_TABLE_PA, L2_TABLE_PA | 0b11);
    write_u64(mem, L2_TABLE_PA, L3_TABLE_PA | 0b11);
    let page_attr = (1u64 << 10) | 0b11;
    // VA 0x1000 → PA 0x1000 (UART)
    write_u64(mem, L3_TABLE_PA + 8, 0x1000 | page_attr);
    // VA 0x2000 → PA 0x2000 (AIC MMIO)
    write_u64(mem, L3_TABLE_PA + 2 * 8, 0x2000 | page_attr);
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

const fn add_reg(rd: u32, rn: u32, rm: u32) -> u32 {
    0x8B00_0000 | ((rm & 0x1F) << 16) | ((rn & 0x1F) << 5) | (rd & 0x1F)
}

const fn sub_reg(rd: u32, rn: u32, rm: u32) -> u32 {
    0xCB00_0000 | ((rm & 0x1F) << 16) | ((rn & 0x1F) << 5) | (rd & 0x1F)
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

/// Encode `B label` with a signed instruction-word offset (-1 = previous insn).
const fn b_offset(words: i32) -> u32 {
    let imm26 = (words as u32) & 0x03FF_FFFF;
    0x1400_0000 | imm26
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
    fn both_cores_run_task_a_before_first_tick() {
        // Kernel boot is 21 instructions; after that both cores ERET into
        // task A and start writing 'A's. We need to stop before the timer
        // fires (every 30 system steps) to keep the output free of B chars.
        let mut cpu = Cpu::new();
        cpu.run(28);
        let out = cpu.output();
        // Output should be all 'A' characters now.
        assert!(!out.is_empty(), "no output yet");
        assert!(out.chars().all(|c| c == 'A'), "got non-A chars: {:?}", out);
        assert!(!cpu.cores[0].halted);
        assert!(!cpu.cores[1].halted);
    }

    #[test]
    fn scheduler_swaps_to_task_b_on_first_tick() {
        let mut cpu = Cpu::new();
        // Run long enough for at least one timer tick + scheduler handler
        // execution + a few task-B iterations.
        cpu.run(80);
        assert!(cpu.timer_ticks >= 1);
        let out = cpu.output();
        assert!(out.contains('A'), "no A: {:?}", out);
        assert!(out.contains('B'), "no B: {:?}", out);
    }

    #[test]
    fn aic_acks_clear_pending_bits() {
        let mut cpu = Cpu::new();
        cpu.run(200);
        // Many ticks; software has been ACKing each one. After ACK the bit
        // clears, so pending should NOT have unbounded accumulation. At any
        // sample point either the bit is clear or it was just raised.
        let aic_state = cpu.aic.snapshot();
        // total_acks should be ≥ timer_ticks * 2 - some_in_flight (both cores
        // ACK each tick). Approx check: at least timer_ticks acks happened.
        assert!(
            aic_state.total_acks >= cpu.timer_ticks,
            "total_acks {} < timer_ticks {}",
            aic_state.total_acks,
            cpu.timer_ticks
        );
    }

    #[test]
    fn daif_restored_through_eret() {
        let mut cpu = Cpu::new();
        // After 5 steps the EL2 prologue ERETs into EL1 with SPSR_EL2=0x3C5
        // (M[3:0]=EL1h, DAIF all set) — kernel runs with IRQs masked.
        cpu.run(5);
        assert_eq!(cpu.cores[0].current_el, 1);
        assert_eq!(cpu.cores[0].daif, 0xF);
        // Remaining kernel boot is 16 more instructions (21 total per core).
        cpu.run(16);
        assert_eq!(cpu.cores[0].current_el, 0);
        assert_eq!(cpu.cores[0].daif, 0);
    }

    #[test]
    fn daifclr_clears_i_bit() {
        // Verify the MSR DAIFClr immediate path — start at EL2 with daif=0xF,
        // run a hand-built program that does DAIFClr #2 (clear I).
        let mut cpu = Cpu::new();
        // Encode MSR DAIFClr #2: 0xD503_42FF.
        let prog = [0xD503_42FFu32, b_self()];
        write_words(&mut cpu.mem, ENTRY_PC, &prog);
        cpu.cores[0].sctlr_el1 = 0; // identity-map by bypassing MMU
        cpu.cores[0].daif = 0xF;
        cpu.run(5);
        // DAIFClr #2 = clear bit 1 (I) → daif = 0xD.
        assert_eq!(cpu.cores[0].daif, 0xD);
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
