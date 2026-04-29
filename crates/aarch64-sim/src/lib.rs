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
            Ok(v) => v,
            Err(e) => {
                self.trap(format!("{e} at pc={:#x}", pc));
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

    /// MMU-aware translation for instruction fetch and data accesses. When
    /// SCTLR_EL1.M=0 the address passes through unchanged; when M=1 we walk
    /// the stage-1 tables and surface any fault as a string error.
    fn translate_for_access(&self, va: u64) -> Result<u64, String> {
        if self.sctlr_el1 & 1 == 0 {
            return Ok(va);
        }
        let r = self.do_translate(va);
        r.pa.ok_or_else(|| r.fault.unwrap_or_else(|| "MMU fault".into()))
    }

    // --- raw physical-memory accessors. The page-table walker uses these
    //     directly so it never recurses through translate_for_access. ---

    fn read_pa_u32(&self, pa: u64) -> Result<u32, String> {
        let a = pa as usize;
        if a + 4 > self.mem.len() {
            return Err(format!("fetch fault at PA {:#x}", pa));
        }
        Ok(u32::from_le_bytes([
            self.mem[a],
            self.mem[a + 1],
            self.mem[a + 2],
            self.mem[a + 3],
        ]))
    }

    fn read_pa_u64(&self, pa: u64) -> Result<u64, String> {
        let a = pa as usize;
        if a + 8 > self.mem.len() {
            return Err(format!("load fault at PA {:#x}", pa));
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

    fn write_pa_u64(&mut self, pa: u64, val: u64) -> Result<(), String> {
        let a = pa as usize;
        if a + 8 > self.mem.len() {
            return Err(format!("store fault at PA {:#x}", pa));
        }
        self.mem[a..a + 8].copy_from_slice(&val.to_le_bytes());
        Ok(())
    }

    fn fetch_u32(&self, va: u64) -> Result<u32, String> {
        let pa = self.translate_for_access(va)?;
        self.read_pa_u32(pa)
    }

    fn load64(&self, va: u64) -> Result<u64, String> {
        let pa = self.translate_for_access(va)?;
        self.read_pa_u64(pa)
    }

    fn store64(&mut self, va: u64, val: u64) -> Result<(), String> {
        let pa = self.translate_for_access(va)?;
        // UART is identified by physical address: writing to PA 0x1000
        // emits the low byte, regardless of MMU state.
        if pa == UART_OUT {
            self.output_buf.push((val & 0xFF) as u8);
        }
        self.write_pa_u64(pa, val)
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

        // MSR Xt, sysreg / MRS Xt, sysreg :: 1101 0101 00 L op0 op1 CRn CRm op2 Rt
        // L=0 → MSR (write sysreg from Rt), L=1 → MRS (read sysreg into Rt)
        if insn & 0xFFC0_0000 == 0xD500_0000 {
            let l = (insn >> 21) & 1;
            let op0 = (insn >> 19) & 0x3;
            let op1 = (insn >> 16) & 0x7;
            let crn = (insn >> 12) & 0xF;
            let crm = (insn >> 8) & 0xF;
            let op2 = (insn >> 5) & 0x7;
            let rt = (insn & 0x1F) as usize;

            // Hint / barrier instructions occupy this same major class but with
            // Rt = 0b11111 and op0 < 2; route them to no-ops below.
            if op0 < 2 {
                // 0xD503_201F NOP, 0xD503_30xx ISB/DSB/DMB. Treat as no-op.
                if insn & 0xFFFF_F01F == 0xD503_201F || insn & 0xFFFF_F01F == 0xD503_301F {
                    self.pc = self.pc.wrapping_add(4);
                    return Ok(StepResult::Continue);
                }
                return Err(format!(
                    "unsupported system instruction {:#010x} at pc={:#x}",
                    insn, self.pc
                ));
            }

            match (op0, op1, crn, crm, op2) {
                // TTBR0_EL1 = S3_0_C2_C0_0
                (3, 0, 2, 0, 0) => {
                    if l == 0 {
                        self.ttbr0_el1 = self.read_x(rt);
                    } else {
                        let v = self.ttbr0_el1;
                        self.write_x(rt, v);
                    }
                }
                // TCR_EL1 = S3_0_C2_C0_2
                (3, 0, 2, 0, 2) => {
                    if l == 0 {
                        self.tcr_el1 = self.read_x(rt);
                    } else {
                        let v = self.tcr_el1;
                        self.write_x(rt, v);
                    }
                }
                // SCTLR_EL1 = S3_0_C1_C0_0
                (3, 0, 1, 0, 0) => {
                    if l == 0 {
                        self.sctlr_el1 = self.read_x(rt);
                    } else {
                        let v = self.sctlr_el1;
                        self.write_x(rt, v);
                    }
                }
                _ => {
                    return Err(format!(
                        "{} of unsupported sysreg S{}_{}_C{}_C{}_{} at pc={:#x}",
                        if l == 1 { "MRS" } else { "MSR" },
                        op0, op1, crn, crm, op2,
                        self.pc
                    ));
                }
            }
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
        // Demo program: enable the MMU using MSR, then write "Hello\n" through it.
        // Page tables (L1/L2/L3) are pre-populated by setup_demo_pgtable; this
        // routine only points TTBR0/TCR at them and flips SCTLR_EL1.M.
        let prog: [u32; 21] = [
            // --- bring up the MMU ---
            movz(9, L1_TABLE_PA as u32, 0),   // X9 = 0x8000 (L1 table PA)
            msr_ttbr0(9),                     // TTBR0_EL1 = X9
            movz(9, 25, 0),                   // X9 = 25  (TCR.T0SZ → 39-bit VA)
            msr_tcr(9),                       // TCR_EL1 = X9
            movz(9, 1, 0),                    // X9 = 1   (SCTLR.M = 1)
            msr_sctlr(9),                     // SCTLR_EL1 = X9 — MMU on
            isb(),                            // realistic barrier; no-op for us

            // --- the original Hello\n through the MMU ---
            movz(1, UART_OUT as u32, 0),      // MOVZ X1, #0x1000 (UART VA)
            movz(0, b'H' as u32, 0),          // MOVZ X0, #'H'
            str_imm(0, 1, 0),                 // STR  X0, [X1]   (VA→PA via MMU)
            movz(0, b'e' as u32, 0),
            str_imm(0, 1, 0),
            movz(0, b'l' as u32, 0),
            str_imm(0, 1, 0),
            str_imm(0, 1, 0),
            movz(0, b'o' as u32, 0),
            str_imm(0, 1, 0),
            movz(0, b'\n' as u32, 0),
            str_imm(0, 1, 0),
            add_imm(0, 0, 0),                 // ADD X0, X0, #0 — ADD demo
            b_self(),                         // B . — halt
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
        // Page tables themselves at PA 0x8000-0xA000 — also identity-map them
        // so a kernel could walk/edit them once MMU is on. Index = 8/9/A.
        write_u64(&mut self.mem, L3_TABLE_PA + 8 * 8, 0x8000 | page_attr);
        write_u64(&mut self.mem, L3_TABLE_PA + 9 * 8, 0x9000 | page_attr);
        write_u64(&mut self.mem, L3_TABLE_PA + 0xA * 8, 0xA000 | page_attr);

        // TTBR0/TCR/SCTLR are NOT preloaded here — the demo program brings them
        // up via MSR so the boot sequence is visible step-by-step.
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
            let descriptor = match self.read_pa_u64(entry_addr) {
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

const fn msr_sysreg(rt: u32, op0: u32, op1: u32, crn: u32, crm: u32, op2: u32) -> u32 {
    // MSR Xt, sysreg :: 1101 0101 000 1 op0 op1 CRn CRm op2 Rt
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

const fn isb() -> u32 {
    0xD503_3FDF
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

    /// Run only the demo's MMU bring-up sequence (the 7 instructions before
    /// the Hello loop) so do_translate has TTBR0/TCR populated.
    fn boot_mmu(cpu: &mut Cpu) {
        cpu.run(7);
    }

    #[test]
    fn translate_program_page() {
        let mut cpu = Cpu::new();
        boot_mmu(&mut cpu);
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
        let mut cpu = Cpu::new();
        boot_mmu(&mut cpu);
        let r = cpu.do_translate(0x1abc);
        assert_eq!(r.pa, Some(0x1abc));
    }

    #[test]
    fn translate_unmapped_va_faults() {
        let mut cpu = Cpu::new();
        boot_mmu(&mut cpu);
        let r = cpu.do_translate(0x2000);
        assert!(r.pa.is_none());
        assert!(r.fault.is_some());
    }

    #[test]
    fn mmu_enables_after_msr_sctlr() {
        let mut cpu = Cpu::new();
        assert_eq!(cpu.sctlr_el1 & 1, 0);
        boot_mmu(&mut cpu);
        assert_eq!(cpu.ttbr0_el1, L1_TABLE_PA);
        assert_eq!(cpu.tcr_el1, 25);
        assert_eq!(cpu.sctlr_el1 & 1, 1);
        // Continuing past MMU bring-up still works — fetches are now translated.
        cpu.run(1000);
        assert!(cpu.halted);
        assert!(cpu.last_trap.is_none(), "trap: {:?}", cpu.last_trap);
        assert_eq!(cpu.output(), "Hello\n");
    }
}
