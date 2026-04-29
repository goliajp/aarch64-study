//! Tiny AArch64 instruction simulator.
//!
//! v0.1 — MOVZ, ADD (imm), LDR/STR (unsigned offset), B. 64 KiB memory.
//!        Memory-mapped UART at 0x1000.
//! v0.2 — Stage-1 MMU translation walk (4 KiB granule, 39-bit VA, levels 1-3).
//!        Demo page table identity-maps the program page (0x4000) and the UART
//!        page (0x1000); translation is a separate query method, not yet wired
//!        into the LDR/STR path (that lands when SCTLR_EL1.M is honoured).

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

#[derive(Serialize, Clone)]
pub struct CpuState {
    pub x: [u64; 31],
    pub sp: u64,
    pub pc: u64,
    pub nzcv: u8,
    pub halted: bool,
    pub last_trap: Option<String>,
    pub steps: u64,
    pub ttbr0_el1: u64,
    pub tcr_el1: u64,
    pub sctlr_el1: u64,
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
    /// Non-leaf descriptor pointing to the next-level table.
    Table { next_table: u64 },
    /// Leaf page descriptor (only at level 3 for 4 KiB granule).
    Page { pa: u64, attrs: PageAttrs },
    /// Leaf block descriptor at L1 or L2.
    Block { pa: u64, attrs: PageAttrs, span: u64 },
    /// Descriptor's valid bit was clear.
    Invalid,
    /// Memory access for the descriptor itself faulted.
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

#[wasm_bindgen]
pub struct Cpu {
    x: [u64; 31],
    sp: u64,
    pc: u64,
    nzcv: u8,
    mem: Vec<u8>,
    output_buf: Vec<u8>,
    halted: bool,
    last_trap: Option<String>,
    steps: u64,
    ttbr0_el1: u64,
    tcr_el1: u64,
    sctlr_el1: u64,
}

#[wasm_bindgen]
impl Cpu {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Cpu {
        let mut cpu = Cpu {
            x: [0; 31],
            sp: 0,
            pc: ENTRY_PC,
            nzcv: 0,
            mem: vec![0u8; MEM_SIZE],
            output_buf: Vec::new(),
            halted: false,
            last_trap: None,
            steps: 0,
            ttbr0_el1: 0,
            tcr_el1: 0,
            sctlr_el1: 0,
        };
        cpu.load_demo();
        cpu.setup_demo_pgtable();
        cpu
    }

    pub fn reset(&mut self) {
        self.x = [0; 31];
        self.sp = 0;
        self.pc = ENTRY_PC;
        self.nzcv = 0;
        for b in self.mem.iter_mut() {
            *b = 0;
        }
        self.output_buf.clear();
        self.halted = false;
        self.last_trap = None;
        self.steps = 0;
        self.ttbr0_el1 = 0;
        self.tcr_el1 = 0;
        self.sctlr_el1 = 0;
        self.load_demo();
        self.setup_demo_pgtable();
    }

    /// Execute one instruction. Returns true if the CPU is still runnable.
    pub fn step(&mut self) -> bool {
        if self.halted {
            return false;
        }
        let pc = self.pc;
        let insn = match self.fetch_u32(pc) {
            Some(v) => v,
            None => {
                self.trap(format!("fetch fault at pc={:#x}", pc));
                return false;
            }
        };
        match self.execute(insn) {
            Ok(StepResult::Continue) => true,
            Ok(StepResult::Halt) => {
                self.halted = true;
                false
            }
            Err(reason) => {
                self.trap(reason);
                false
            }
        }
    }

    /// Run up to `max` steps or until halted/trapped. Returns steps actually executed.
    pub fn run(&mut self, max: u32) -> u32 {
        let mut n = 0u32;
        while n < max && self.step() {
            n += 1;
        }
        n
    }

    pub fn state(&self) -> Result<JsValue, JsValue> {
        to_js(&CpuState {
            x: self.x,
            sp: self.sp,
            pc: self.pc,
            nzcv: self.nzcv,
            halted: self.halted,
            last_trap: self.last_trap.clone(),
            steps: self.steps,
            ttbr0_el1: self.ttbr0_el1,
            tcr_el1: self.tcr_el1,
            sctlr_el1: self.sctlr_el1,
        })
    }

    /// Walk the stage-1 page tables for `va` using the current TTBR0/TCR.
    /// Returns the walk trace plus the resolved physical address (or fault).
    pub fn translate(&self, va: u64) -> Result<JsValue, JsValue> {
        to_js(&self.do_translate(va))
    }

    pub fn l1_table_pa(&self) -> u64 {
        L1_TABLE_PA
    }

    /// Return a slice of memory as a Uint8Array. `start` and `len` are byte offsets.
    pub fn mem_slice(&self, start: u32, len: u32) -> Vec<u8> {
        let s = (start as usize).min(self.mem.len());
        let e = (s + len as usize).min(self.mem.len());
        self.mem[s..e].to_vec()
    }

    pub fn output(&self) -> String {
        String::from_utf8_lossy(&self.output_buf).into_owned()
    }

    pub fn entry_pc(&self) -> u64 {
        ENTRY_PC
    }

    pub fn uart_addr(&self) -> u64 {
        UART_OUT
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

enum StepResult {
    Continue,
    Halt,
}

impl Cpu {
    fn trap(&mut self, msg: String) {
        self.last_trap = Some(msg);
        self.halted = true;
    }

    fn fetch_u32(&self, addr: u64) -> Option<u32> {
        let a = addr as usize;
        if a + 4 > self.mem.len() {
            return None;
        }
        Some(u32::from_le_bytes([
            self.mem[a],
            self.mem[a + 1],
            self.mem[a + 2],
            self.mem[a + 3],
        ]))
    }

    fn load64(&self, addr: u64) -> Result<u64, String> {
        let a = addr as usize;
        if a + 8 > self.mem.len() {
            return Err(format!("load fault at {:#x}", addr));
        }
        Ok(u64::from_le_bytes([
            self.mem[a],
            self.mem[a + 1],
            self.mem[a + 2],
            self.mem[a + 3],
            self.mem[a + 4],
            self.mem[a + 5],
            self.mem[a + 6],
            self.mem[a + 7],
        ]))
    }

    fn store64(&mut self, addr: u64, val: u64) -> Result<(), String> {
        // UART hook: any 8-byte store whose base lands in [UART_OUT, UART_OUT+8)
        // emits the low byte. The store still proceeds against backing memory.
        if addr == UART_OUT {
            self.output_buf.push((val & 0xFF) as u8);
        }
        let a = addr as usize;
        if a + 8 > self.mem.len() {
            return Err(format!("store fault at {:#x}", addr));
        }
        self.mem[a..a + 8].copy_from_slice(&val.to_le_bytes());
        Ok(())
    }

    fn execute(&mut self, insn: u32) -> Result<StepResult, String> {
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

        // STR Xt, [Xn, #imm12] :: 11 111 0 01 00 imm12 Rn Rt   (offset = imm12 * 8)
        if insn & 0xFFC0_0000 == 0xF900_0000 {
            let rt = (insn & 0x1F) as usize;
            let rn = ((insn >> 5) & 0x1F) as usize;
            let imm12 = ((insn >> 10) & 0xFFF) as u64;
            let addr = self.read_x(rn).wrapping_add(imm12 * 8);
            self.store64(addr, self.read_x(rt))?;
            self.pc = self.pc.wrapping_add(4);
            return Ok(StepResult::Continue);
        }

        // LDR Xt, [Xn, #imm12] :: 11 111 0 01 01 imm12 Rn Rt
        if insn & 0xFFC0_0000 == 0xF940_0000 {
            let rt = (insn & 0x1F) as usize;
            let rn = ((insn >> 5) & 0x1F) as usize;
            let imm12 = ((insn >> 10) & 0xFFF) as u64;
            let addr = self.read_x(rn).wrapping_add(imm12 * 8);
            let val = self.load64(addr)?;
            self.write_x(rt, val);
            self.pc = self.pc.wrapping_add(4);
            return Ok(StepResult::Continue);
        }

        // B label :: 0 00101 imm26
        if insn & 0xFC00_0000 == 0x1400_0000 {
            let imm26_raw = (insn & 0x03FF_FFFF) as i32;
            // sign-extend 26-bit
            let imm26 = (imm26_raw << 6) >> 6;
            let offset = (imm26 as i64) * 4;
            let target = (self.pc as i64).wrapping_add(offset) as u64;
            // Branch-to-self halts the simulation (idiom for "done").
            if target == self.pc {
                return Ok(StepResult::Halt);
            }
            self.pc = target;
            return Ok(StepResult::Continue);
        }

        Err(format!("undefined instruction {:#010x} at pc={:#x}", insn, self.pc))
    }

    fn read_x(&self, idx: usize) -> u64 {
        // X31 in arithmetic context reads as zero (XZR); we don't model SP-context here.
        if idx == 31 { 0 } else { self.x[idx] }
    }

    fn write_x(&mut self, idx: usize, val: u64) {
        if idx < 31 {
            self.x[idx] = val;
        }
    }

    fn load_demo(&mut self) {
        // Program: write "Hello\n" to UART by repeated STR to [X1, #0] where X1 = UART_OUT.
        let prog: [u32; 14] = [
            movz(1, UART_OUT as u32, 0),      // MOVZ X1, #0x1000
            movz(0, b'H' as u32, 0),          // MOVZ X0, #'H'
            str_imm(0, 1, 0),                 // STR  X0, [X1]
            movz(0, b'e' as u32, 0),
            str_imm(0, 1, 0),
            movz(0, b'l' as u32, 0),
            str_imm(0, 1, 0),
            str_imm(0, 1, 0),                 // 'l' twice
            movz(0, b'o' as u32, 0),
            str_imm(0, 1, 0),
            movz(0, b'\n' as u32, 0),
            str_imm(0, 1, 0),
            add_imm(0, 0, 0),                 // NOP-ish (ADD X0, X0, #0) to show ADD too
            b_self(),                          // halt
        ];
        let mut off = ENTRY_PC as usize;
        for word in prog.iter() {
            self.mem[off..off + 4].copy_from_slice(&word.to_le_bytes());
            off += 4;
        }
    }

    /// Build a tiny stage-1 page table: identity-map the program page (0x4000)
    /// and the UART page (0x1000) using a 3-level walk with 4 KiB granule.
    fn setup_demo_pgtable(&mut self) {
        // L1[0] -> L2 table.   Descriptor: bits[1:0]=11 (valid table), addr in [47:12].
        write_u64(&mut self.mem, L1_TABLE_PA, L2_TABLE_PA | 0b11);
        // L2[0] -> L3 table.
        write_u64(&mut self.mem, L2_TABLE_PA, L3_TABLE_PA | 0b11);

        // L3 page descriptor format we use: bits[1:0]=11, AF=bit10, valid; output PA in [47:12].
        let page_attr = (1u64 << 10) | 0b11; // AF=1, valid+page
        // VA 0x1000 -> PA 0x1000 (UART page). Index = (0x1000 >> 12) & 0x1FF = 1.
        write_u64(&mut self.mem, L3_TABLE_PA + 1 * 8, 0x1000 | page_attr);
        // VA 0x4000 -> PA 0x4000 (program page). Index = 4.
        write_u64(&mut self.mem, L3_TABLE_PA + 4 * 8, 0x4000 | page_attr);

        // TCR_EL1.T0SZ = 25 → 39-bit VA → start level 1 with 4 KiB granule.
        self.tcr_el1 = 25;
        self.ttbr0_el1 = L1_TABLE_PA;
        // SCTLR_EL1.M still 0 — translation is a separate query, not yet applied to LDR/STR.
        self.sctlr_el1 = 0;
    }

    fn do_translate(&self, va: u64) -> TranslationResult {
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
        // Choose start level for 4 KiB granule (each level adds 9 bits).
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
            let descriptor = match self.load64(entry_addr) {
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
                    fault: Some(format!("translation fault at level {} (invalid descriptor)", level)),
                    mmu_enabled,
                };
            }

            // Level 3: only valid kind is page descriptor (typ=1).
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
                return TranslationResult { va, steps, pa: Some(pa), fault: None, mmu_enabled };
            }

            // Non-leaf level: typ=1 → table descriptor, typ=0 → block (huge page).
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
                    outcome: WalkOutcome::Block { pa, attrs, span: block_size },
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
                return TranslationResult { va, steps, pa: Some(pa), fault: None, mmu_enabled };
            }
        }
    }
}

fn write_u64(mem: &mut [u8], addr: u64, val: u64) {
    let a = addr as usize;
    mem[a..a + 8].copy_from_slice(&val.to_le_bytes());
}

fn decode_attrs(desc: u64) -> PageAttrs {
    PageAttrs {
        af: (desc >> 10) & 1 != 0,
        ap: ((desc >> 6) & 0x3) as u8,
        attr_idx: ((desc >> 2) & 0x7) as u8,
        sh: ((desc >> 8) & 0x3) as u8,
    }
}

// --- instruction encoders (host-side helpers for the demo program) ---

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
    // B . (offset 0) — interpreted by the executor as halt.
    0x1400_0000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_writes_hello() {
        let mut cpu = Cpu::new();
        cpu.run(1000);
        assert!(cpu.halted);
        assert_eq!(cpu.output(), "Hello\n");
    }

    #[test]
    fn movz_then_add() {
        let mut cpu = Cpu::new();
        // overwrite memory with our own tiny program
        let prog = [movz(0, 5, 0), add_imm(0, 0, 7), b_self()];
        let mut off = ENTRY_PC as usize;
        for w in prog.iter() {
            cpu.mem[off..off + 4].copy_from_slice(&w.to_le_bytes());
            off += 4;
        }
        cpu.run(100);
        assert_eq!(cpu.x[0], 12);
    }

    #[test]
    fn translate_program_page() {
        let cpu = Cpu::new();
        let r = cpu.do_translate(0x4000);
        assert!(r.fault.is_none(), "fault: {:?}", r.fault);
        assert_eq!(r.pa, Some(0x4000));
        assert_eq!(r.steps.len(), 3);
        assert!(matches!(r.steps[0].outcome, WalkOutcome::Table { .. }));
        assert!(matches!(r.steps[1].outcome, WalkOutcome::Table { .. }));
        assert!(matches!(r.steps[2].outcome, WalkOutcome::Page { pa: 0x4000, .. }));
    }

    #[test]
    fn translate_uart_page_with_offset() {
        let cpu = Cpu::new();
        let r = cpu.do_translate(0x1abc);
        assert_eq!(r.pa, Some(0x1abc));
    }

    #[test]
    fn translate_unmapped_va_faults() {
        let cpu = Cpu::new();
        let r = cpu.do_translate(0x2000); // not in our 2-page identity map
        assert!(r.pa.is_none());
        assert!(r.fault.is_some());
    }
}
