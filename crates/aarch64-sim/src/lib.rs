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
// Sized so the per-core kernel boot (~35 inst) completes before the first
// tick, with room for several task iterations between ticks.
const TIMER_PERIOD: u64 = 80;

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

// Block-device MMIO. Loosely virtio-blk-shaped but with a fixed-size 64-byte
// sector and a synchronous "write CMD → transfer happens before the STR
// returns". 8 sectors × 64 bytes = 512 bytes total disk image.
const BLK_BASE: u64 = 0x3000;
const BLK_END: u64 = 0x3100;
const BLK_REG_SECTOR: u64 = 0x00;
const BLK_REG_BUF_ADDR: u64 = 0x08;
const BLK_REG_CMD: u64 = 0x10;
const BLK_REG_STATUS: u64 = 0x18;
const SECTOR_SIZE: u64 = 64;
const NUM_SECTORS: u64 = 8;
const DISK_SIZE: u64 = SECTOR_SIZE * NUM_SECTORS;
const BLK_CMD_READ: u64 = 0;
const BLK_CMD_WRITE: u64 = 1;
const BLK_STATUS_IDLE: u64 = 0;
const BLK_STATUS_OK: u64 = 1;
const BLK_STATUS_FAULT: u64 = 2;

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

#[derive(Serialize, Clone)]
pub struct BlockState {
    pub sector: u64,
    pub buf_addr: u64,
    pub last_command: u64,
    pub status: u64,
    pub total_reads: u64,
    pub total_writes: u64,
    pub disk: Vec<u8>,
}

// === Block: a tiny synchronous "virtio-blk"-shaped device ====================

struct Block {
    disk: Vec<u8>,
    sector: u64,
    buf_addr: u64,
    last_command: u64,
    status: u64,
    total_reads: u64,
    total_writes: u64,
}

impl Block {
    fn new() -> Self {
        let mut disk = vec![0u8; DISK_SIZE as usize];
        // Pre-populate sectors with text so a read produces visible content.
        let inscribe = |b: &mut [u8], sector: u64, text: &str| {
            let off = (sector * SECTOR_SIZE) as usize;
            let bytes = text.as_bytes();
            let n = bytes.len().min(SECTOR_SIZE as usize);
            b[off..off + n].copy_from_slice(&bytes[..n]);
        };
        inscribe(&mut disk, 0, "OSstudy disk image — sector 0\n");
        inscribe(&mut disk, 1, "Sector 1: kernels run on top of devices\n");
        inscribe(&mut disk, 2, "Sector 2: virtio is the lingua franca\n");
        inscribe(&mut disk, 3, "Sector 3: this is a 64-byte sector\n");
        Self {
            disk,
            sector: 0,
            buf_addr: 0,
            last_command: 0,
            status: BLK_STATUS_IDLE,
            total_reads: 0,
            total_writes: 0,
        }
    }

    fn reset(&mut self) {
        let fresh = Block::new();
        *self = fresh;
    }

    fn mmio_read(&self, offset: u64) -> u64 {
        match offset {
            BLK_REG_SECTOR => self.sector,
            BLK_REG_BUF_ADDR => self.buf_addr,
            BLK_REG_CMD => self.last_command,
            BLK_REG_STATUS => self.status,
            _ => 0,
        }
    }

    fn mmio_write(&mut self, mem: &mut [u8], offset: u64, val: u64) {
        match offset {
            BLK_REG_SECTOR => self.sector = val,
            BLK_REG_BUF_ADDR => self.buf_addr = val,
            BLK_REG_CMD => {
                self.last_command = val;
                let sec = self.sector as usize * SECTOR_SIZE as usize;
                let buf = self.buf_addr as usize;
                let len = SECTOR_SIZE as usize;
                let in_disk = sec + len <= self.disk.len();
                let in_mem = buf + len <= mem.len();
                if !in_disk || !in_mem {
                    self.status = BLK_STATUS_FAULT;
                    return;
                }
                match val {
                    BLK_CMD_READ => {
                        mem[buf..buf + len].copy_from_slice(&self.disk[sec..sec + len]);
                        self.status = BLK_STATUS_OK;
                        self.total_reads = self.total_reads.saturating_add(1);
                    }
                    BLK_CMD_WRITE => {
                        self.disk[sec..sec + len].copy_from_slice(&mem[buf..buf + len]);
                        self.status = BLK_STATUS_OK;
                        self.total_writes = self.total_writes.saturating_add(1);
                    }
                    _ => {
                        self.status = BLK_STATUS_FAULT;
                    }
                }
            }
            _ => {}
        }
    }

    fn snapshot(&self) -> BlockState {
        BlockState {
            sector: self.sector,
            buf_addr: self.buf_addr,
            last_command: self.last_command,
            status: self.status,
            total_reads: self.total_reads,
            total_writes: self.total_writes,
            disk: self.disk.clone(),
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

    fn load64(&self, mem: &[u8], aic: &mut Aic, block: &Block, va: u64) -> Result<u64, String> {
        let pa = self.translate_for_access(mem, va)?;
        if (AIC_BASE..AIC_END).contains(&pa) {
            return Ok(aic.mmio_read(self.id as usize, pa - AIC_BASE));
        }
        if (BLK_BASE..BLK_END).contains(&pa) {
            return Ok(block.mmio_read(pa - BLK_BASE));
        }
        read_pa_u64(mem, pa)
    }

    fn store64(
        &self,
        mem: &mut [u8],
        out: &mut Vec<u8>,
        aic: &mut Aic,
        block: &mut Block,
        va: u64,
        val: u64,
    ) -> Result<(), String> {
        let pa = self.translate_for_access(mem, va)?;
        if (AIC_BASE..AIC_END).contains(&pa) {
            aic.mmio_write(self.id as usize, pa - AIC_BASE, val);
            return Ok(());
        }
        if (BLK_BASE..BLK_END).contains(&pa) {
            block.mmio_write(mem, pa - BLK_BASE, val);
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

    fn step(
        &mut self,
        mem: &mut [u8],
        out: &mut Vec<u8>,
        aic: &mut Aic,
        block: &mut Block,
    ) -> bool {
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
        match self.execute(insn, mem, out, aic, block) {
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
        block: &mut Block,
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

        // SUB Xd, Xn, #imm12{, LSL #12} :: 1 10 10001 sh imm12 Rn Rd
        if insn & 0xFF80_0000 == 0xD100_0000 {
            let rd = (insn & 0x1F) as usize;
            let rn = ((insn >> 5) & 0x1F) as usize;
            let imm12 = ((insn >> 10) & 0xFFF) as u64;
            let sh = ((insn >> 22) & 0x1) as u32;
            let imm = if sh == 1 { imm12 << 12 } else { imm12 };
            let val = self.read_x(rn).wrapping_sub(imm);
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
            self.store64(mem, out, aic, block, addr, val)?;
            self.pc = self.pc.wrapping_add(4);
            return Ok(StepResult::Continue);
        }

        // LDR Xt, [Xn, #imm12]
        if insn & 0xFFC0_0000 == 0xF940_0000 {
            let rt = (insn & 0x1F) as usize;
            let rn = ((insn >> 5) & 0x1F) as usize;
            let imm12 = ((insn >> 10) & 0xFFF) as u64;
            let addr = self.read_x(rn).wrapping_add(imm12 * 8);
            let val = self.load64(mem, aic, block, addr)?;
            self.write_x(rt, val);
            self.pc = self.pc.wrapping_add(4);
            return Ok(StepResult::Continue);
        }

        // LDRB Wt, [Xn, #imm12]  (byte load, zero-extend) :: 0011 1001 01 imm12 Rn Rt
        if insn & 0xFFC0_0000 == 0x3940_0000 {
            let rt = (insn & 0x1F) as usize;
            let rn = ((insn >> 5) & 0x1F) as usize;
            let imm12 = ((insn >> 10) & 0xFFF) as u64;
            let va = self.read_x(rn).wrapping_add(imm12);
            let pa = self.translate_for_access(mem, va)?;
            // AIC/Block ranges fall back to byte access via mmio_read of low byte.
            let byte = if (AIC_BASE..AIC_END).contains(&pa) {
                (aic.mmio_read(self.id as usize, pa - AIC_BASE) & 0xFF) as u8
            } else if (BLK_BASE..BLK_END).contains(&pa) {
                (block.mmio_read(pa - BLK_BASE) & 0xFF) as u8
            } else {
                let a = pa as usize;
                if a >= mem.len() {
                    return Err(format!("byte fetch fault at PA {:#x}", pa));
                }
                mem[a]
            };
            self.write_x(rt, byte as u64);
            self.pc = self.pc.wrapping_add(4);
            return Ok(StepResult::Continue);
        }

        // CBZ / CBNZ Xt, label :: sf 011010 op imm19 Rt
        if insn & 0xFE00_0000 == 0xB400_0000 {
            let op = (insn >> 24) & 1; // 0 = CBZ, 1 = CBNZ
            let imm19_raw = ((insn >> 5) & 0x7_FFFF) as u32;
            let imm19 = ((imm19_raw as i32) << 13) >> 13; // sign-extend 19-bit
            let offset = (imm19 as i64) * 4;
            let rt = (insn & 0x1F) as usize;
            let val = self.read_x(rt);
            let take = if op == 0 { val == 0 } else { val != 0 };
            if take {
                let target = (self.pc as i64).wrapping_add(offset) as u64;
                if target == self.pc {
                    return Ok(StepResult::Halt);
                }
                self.pc = target;
            } else {
                self.pc = self.pc.wrapping_add(4);
            }
            return Ok(StepResult::Continue);
        }

        // LDP/STP (signed offset, 64-bit)
        // 1 0 1 0 1 0 0 1 0 L imm7 Rt2 Rn Rt1
        //   STP: 0xA9000000  |  LDP: 0xA9400000
        if (insn & 0xFFC0_0000 == 0xA900_0000) || (insn & 0xFFC0_0000 == 0xA940_0000) {
            let l = (insn >> 22) & 1;
            // imm7 is signed, scaled by 8.
            let imm7_raw = ((insn >> 15) & 0x7F) as u32;
            let imm7 = ((imm7_raw as i32) << 25) >> 25; // sign-extend 7-bit
            let offset_bytes = (imm7 as i64) * 8;
            let rt2 = ((insn >> 10) & 0x1F) as usize;
            let rn = ((insn >> 5) & 0x1F) as usize;
            let rt1 = (insn & 0x1F) as usize;
            let base = self.read_x(rn);
            let addr = (base as i64).wrapping_add(offset_bytes) as u64;
            if l == 0 {
                let v1 = self.read_x(rt1);
                let v2 = self.read_x(rt2);
                self.store64(mem, out, aic, block, addr, v1)?;
                self.store64(mem, out, aic, block, addr.wrapping_add(8), v2)?;
            } else {
                let v1 = self.load64(mem, aic, block, addr)?;
                let v2 = self.load64(mem, aic, block, addr.wrapping_add(8))?;
                self.write_x(rt1, v1);
                self.write_x(rt2, v2);
            }
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
    block: Block,
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
            block: Block::new(),
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
        self.block.reset();
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
            } else if self.cores[i].step(
                &mut self.mem,
                &mut self.output_buf,
                &mut self.aic,
                &mut self.block,
            ) {
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
            self.cores[i].step(
                &mut self.mem,
                &mut self.output_buf,
                &mut self.aic,
                &mut self.block,
            )
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

    pub fn block_state(&self) -> Result<JsValue, JsValue> {
        to_js(&self.block.snapshot())
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
    //   PA 0x4000  kernel boot
    //   PA 0x4800  sync handler (stub)
    //   PA 0x4880  IRQ handler = scheduler with X0-X3 save/restore
    //   PA 0x4D00  task A — counter that prints 'A'
    //   PA 0x4E00  task B — disk printer (walks 0x6000 byte by byte)
    //   PA 0x4F00  core 0's slot region (entry + save_ptr + 2 save areas)
    //   PA 0x5000  core 1's slot region (same layout)
    //
    // Each core derives "my slot region" from MPIDR_EL1: bit 8 of MPIDR
    // distinguishes our two clusters (P-core 0x80000000, E-core 0x80000100),
    // so mpidr_offset = mpidr - 0x80000000 ∈ {0, 0x100} maps directly:
    //   slot_base = 0x4F00 + mpidr_offset → 0x4F00 / 0x5000
    //   initial_task = TASK_A_ENTRY + mpidr_offset → 0x4D00 / 0x4E00
    // So core 0 boots into task A and core 1 boots into task B; they run
    // concurrently with independent state and the UART output truly
    // interleaves instead of duplicating.
    //
    // Within a slot region:
    //   +0x00  current task entry (8 bytes)
    //   +0x08  current task save-area pointer (8 bytes)
    //   +0x10  save area 0 (32 bytes — X0..X3)
    //   +0x30  save area 1 (32 bytes — X0..X3)
    // The scheduler swaps between save areas via the (2*slot_base + 0x40 -
    // current_save_ptr) trick — no per-core constants baked into the handler.
    const SPSR_EL1H_DAIF: u32 = 0x3C5;
    const VBAR: u32 = 0x4400;
    const SYNC_HANDLER_PA: u64 = 0x4800;
    const IRQ_HANDLER_PA: u64 = 0x4880;
    const TASK_A_ENTRY: u32 = 0x4D00;
    const TASK_B_ENTRY: u32 = 0x4E00;
    const TASK_SUM: u32 = TASK_A_ENTRY + TASK_B_ENTRY; // 0x9B00
    const SLOT_BASE_BASE: u32 = 0x4F00; // base for core 0
    const MPIDR_BASE_HI: u32 = 0x8000; // moved to upper half via LSL #16
    const DISK_BUF_PA: u32 = 0x6000;
    const EL1_ENTRY: u32 = ENTRY_PC as u32 + 5 * 4;

    let kernel: [u32; 35] = [
        // --- EL2 prologue → ERET to EL1 ---
        movz(9, EL1_ENTRY, 0),
        msr_elr_el2(9),
        movz(9, SPSR_EL1H_DAIF, 0),
        msr_spsr_el2(9),
        eret(),
        // --- EL1: MMU bring-up ---
        movz(9, L1_TABLE_PA as u32, 0),
        msr_ttbr0(9),
        movz(9, 25, 0),
        msr_tcr(9),
        movz(9, 1, 0),
        msr_sctlr(9),
        isb(),
        // --- EL1: synchronous disk read sector 0 → DISK_BUF_PA ---
        movz(9, BLK_BASE as u32, 0),
        movz(10, 0, 0),
        str_imm(10, 9, 0), // SECTOR = 0
        movz(10, DISK_BUF_PA, 0),
        str_imm(10, 9, 1), // BUF_ADDR = 0x6000
        movz(10, BLK_CMD_READ as u32, 0),
        str_imm(10, 9, 2), // CMD = READ
        // --- EL1: install vector base ---
        movz(9, VBAR, 0),
        msr_vbar_el1(9),
        // --- EL1: derive per-core slot region from MPIDR_EL1 ---
        // X9 = mpidr_offset ∈ {0, 0x100}
        mrs_mpidr(9),
        movz(10, MPIDR_BASE_HI, 1), // X10 = 0x80000000 (LSL #16)
        sub_reg(9, 9, 10),
        // X14 = my slot base
        movz(10, SLOT_BASE_BASE, 0),
        add_reg(14, 10, 9),
        // X11 = my initial task entry
        movz(10, TASK_A_ENTRY, 0),
        add_reg(11, 10, 9),
        // X12 = my initial save area = slot_base + 0x10
        add_imm(12, 14, 0x10),
        // Initialise my slot: [slot_base+0]=entry, [slot_base+8]=save_ptr
        str_imm(11, 14, 0),
        str_imm(12, 14, 1),
        // --- EL1: ERET into my initial task at EL0t with DAIF=0 ---
        msr_elr_el1(11),
        movz(10, 0, 0),
        msr_spsr_el1(10),
        eret(),
    ];
    // Sync handler at VBAR+0x400 — stub for any stray SVC.
    let sync_handler: [u32; 5] = [
        movz(1, UART_OUT as u32, 0),
        movz(0, b'K' as u32, 0),
        str_imm(0, 1, 0),
        add_imm(0, 0, 0),
        eret(),
    ];
    // IRQ handler at VBAR+0x480 — per-core scheduler with X0..X3 save/restore.
    // Re-derives MY slot region from MPIDR each entry, so the same code runs
    // on both cores without per-core constants.
    //   x14 = my slot base (0x4F00 / 0x5000)
    //   x12 = current save-area ptr  (slot_base + 0x10 or +0x30)
    //   x15 = "the other" save-area ptr = (2*slot_base + 0x40) - x12
    let scheduler: [u32; 22] = [
        // ACK AIC to clear pending
        movz(9, AIC_BASE as u32, 0),       // 0
        ldr_imm(10, 9, 0),                  // 1: X10 = irq id (discarded)
        // X14 = my slot base
        mrs_mpidr(14),                      // 2: X14 = mpidr
        movz(9, MPIDR_BASE_HI, 1),          // 3: X9 = 0x80000000
        sub_reg(14, 14, 9),                 // 4: X14 = mpidr_offset
        movz(9, SLOT_BASE_BASE, 0),         // 5: X9 = 0x4F00
        add_reg(14, 9, 14),                 // 6: X14 = slot_base
        // Load current entry + save-area ptr from my slot
        ldr_imm(11, 14, 0),                 // 7: X11 = current entry
        ldr_imm(12, 14, 1),                 // 8: X12 = current save_ptr
        // Save outgoing X0..X3 to current save area
        stp_imm(0, 1, 12, 0),               // 9
        stp_imm(2, 3, 12, 2),               // 10: imm7=2 → +16
        // Compute "other" entry = TASK_SUM - X11
        movz(13, TASK_SUM, 0),              // 11
        sub_reg(13, 13, 11),                // 12: X13 = other entry
        // Compute "other" save area = (2*slot_base + 0x40) - X12
        add_reg(15, 14, 14),                // 13: X15 = 2*slot_base
        add_imm(15, 15, 0x40),              // 14: X15 = 2*slot_base + 0x40
        sub_reg(15, 15, 12),                // 15: X15 = other save_ptr
        // Commit new state to my slot
        str_imm(13, 14, 0),                 // 16
        str_imm(15, 14, 1),                 // 17
        // Restore incoming X0..X3 from new save area
        ldp_imm(0, 1, 15, 0),               // 18
        ldp_imm(2, 3, 15, 2),               // 19
        // Switch
        msr_elr_el1(13),                    // 20
        eret(),                             // 21
    ];
    // Task A — counts in X3, prints 'A'. X3 persists across context switches.
    let task_a: [u32; 5] = [
        movz(1, UART_OUT as u32, 0),
        add_imm(3, 3, 1),                   // X3 = X3 + 1 (counter)
        movz(0, b'A' as u32, 0),
        str_imm(0, 1, 0),
        b_offset(-3),                       // back to ADD (skip the MOVZ)
    ];
    // Task B — "disk printer". X3 holds the next byte offset into the disk
    // buffer at PA 0x6000 (preserved across context switches via the save
    // area). Each iteration: re-init X1/X4, compute X4 = base + X3, load byte;
    // if zero → reset X3 and re-enter; else → emit, X3++, re-enter.
    let task_b: [u32; 10] = [
        movz(1, UART_OUT as u32, 0),    // 0: X1 = UART
        movz(4, 0x6000, 0),             // 1: X4 = disk buffer base
        add_reg(4, 4, 3),               // 2: X4 += X3
        ldrb_imm(0, 4, 0),              // 3: W0 = byte at X4
        cbz(0, 4),                      // 4: if zero, branch to inst 8 (restart)
        str_imm(0, 1, 0),               // 5: STR X0, [X1] — emit byte to UART
        add_imm(3, 3, 1),               // 6: X3 += 1
        b_offset(-7),                   // 7: → inst 0 (loop)
        movz(3, 0, 0),                  // 8: restart — X3 = 0
        b_offset(-8),                   // 9: → inst 1 (skip MOVZ X1 since fall-through)
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
    // VA 0x3000 → PA 0x3000 (block-device MMIO)
    write_u64(mem, L3_TABLE_PA + 3 * 8, 0x3000 | page_attr);
    // VA 0x4000 → PA 0x4000 (program page)
    write_u64(mem, L3_TABLE_PA + 4 * 8, 0x4000 | page_attr);
    // VA 0x5000 → PA 0x5000 (core 1's per-core scheduler slot region)
    write_u64(mem, L3_TABLE_PA + 5 * 8, 0x5000 | page_attr);
    // VA 0x6000 → PA 0x6000 (disk buffer page)
    write_u64(mem, L3_TABLE_PA + 6 * 8, 0x6000 | page_attr);
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

const fn sub_imm(rd: u32, rn: u32, imm12: u32) -> u32 {
    0xD100_0000 | ((imm12 & 0xFFF) << 10) | ((rn & 0x1F) << 5) | (rd & 0x1F)
}

const fn add_reg(rd: u32, rn: u32, rm: u32) -> u32 {
    0x8B00_0000 | ((rm & 0x1F) << 16) | ((rn & 0x1F) << 5) | (rd & 0x1F)
}

const fn sub_reg(rd: u32, rn: u32, rm: u32) -> u32 {
    0xCB00_0000 | ((rm & 0x1F) << 16) | ((rn & 0x1F) << 5) | (rd & 0x1F)
}

#[allow(dead_code)]
const fn ldrb_imm(rt: u32, rn: u32, imm12: u32) -> u32 {
    0x3940_0000 | ((imm12 & 0xFFF) << 10) | ((rn & 0x1F) << 5) | (rt & 0x1F)
}

const fn cbz(rt: u32, words: i32) -> u32 {
    let imm19 = (words as u32) & 0x7_FFFF;
    0xB400_0000 | (imm19 << 5) | (rt & 0x1F)
}

#[allow(dead_code)]
const fn cbnz(rt: u32, words: i32) -> u32 {
    let imm19 = (words as u32) & 0x7_FFFF;
    0xB500_0000 | (imm19 << 5) | (rt & 0x1F)
}

const fn str_imm(rt: u32, rn: u32, imm12: u32) -> u32 {
    0xF900_0000 | ((imm12 & 0xFFF) << 10) | ((rn & 0x1F) << 5) | (rt & 0x1F)
}

#[allow(dead_code)]
const fn ldr_imm(rt: u32, rn: u32, imm12: u32) -> u32 {
    0xF940_0000 | ((imm12 & 0xFFF) << 10) | ((rn & 0x1F) << 5) | (rt & 0x1F)
}

/// STP Xt1, Xt2, [Xn, #imm7*8] (signed-offset).
const fn stp_imm(rt1: u32, rt2: u32, rn: u32, imm7: i32) -> u32 {
    let imm = (imm7 as u32) & 0x7F;
    0xA900_0000 | (imm << 15) | ((rt2 & 0x1F) << 10) | ((rn & 0x1F) << 5) | (rt1 & 0x1F)
}

/// LDP Xt1, Xt2, [Xn, #imm7*8] (signed-offset).
const fn ldp_imm(rt1: u32, rt2: u32, rn: u32, imm7: i32) -> u32 {
    let imm = (imm7 as u32) & 0x7F;
    0xA940_0000 | (imm << 15) | ((rt2 & 0x1F) << 10) | ((rn & 0x1F) << 5) | (rt1 & 0x1F)
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

/// MRS Xt, sysreg :: MSR with L=1 (bit 21 set).
const fn mrs_sysreg(rt: u32, op0: u32, op1: u32, crn: u32, crm: u32, op2: u32) -> u32 {
    msr_sysreg(rt, op0, op1, crn, crm, op2) | (1 << 21)
}

const fn mrs_mpidr(rt: u32) -> u32 {
    mrs_sysreg(rt, 3, 0, 0, 0, 5)
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
    fn cores_split_to_different_tasks_before_first_tick() {
        // 35-inst kernel, timer at step 80. After 60 steps both cores have
        // ERETed: core 0 into task A (prints 'A'), core 1 into task B
        // (disk printer — first byte is 'O' from "OSstudy…").
        let mut cpu = Cpu::new();
        cpu.run(60);
        let out = cpu.output();
        assert!(!out.is_empty(), "no output yet: {:?}", out);
        // We should see BOTH 'A' (from core 0) and 'O' (from core 1).
        assert!(out.contains('A'), "no A: {:?}", out);
        assert!(out.contains('O'), "no O (disk content): {:?}", out);
        assert!(!cpu.cores[0].halted);
        assert!(!cpu.cores[1].halted);
    }

    #[test]
    fn scheduler_swaps_each_core_to_other_task() {
        let mut cpu = Cpu::new();
        // Run long enough for ≥ 2 timer ticks. After tick 1: core 0 → B,
        // core 1 → A. After tick 2: core 0 → A again, core 1 → B again.
        cpu.run(200);
        assert!(cpu.timer_ticks >= 2);
        let out = cpu.output();
        assert!(out.contains('A'), "no A: {:?}", out);
        assert!(out.contains('O'), "no disk content (O): {:?}", out);
    }

    #[test]
    fn each_core_uses_its_own_slot_region() {
        let mut cpu = Cpu::new();
        cpu.run(400);
        let read_u64 = |mem: &[u8], pa: usize| -> u64 {
            u64::from_le_bytes(mem[pa..pa + 8].try_into().unwrap())
        };
        // Core 0's slot is at 0x4F00, core 1's at 0x5000. Both should hold
        // valid task entry pointers (either A_entry=0x4D00 or B_entry=0x4E00).
        let core0_entry = read_u64(&cpu.mem, 0x4F00);
        let core1_entry = read_u64(&cpu.mem, 0x5000);
        assert!(core0_entry == 0x4D00 || core0_entry == 0x4E00, "{:#x}", core0_entry);
        assert!(core1_entry == 0x4D00 || core1_entry == 0x4E00, "{:#x}", core1_entry);
        // After enough ticks they should have swapped at least once, but at
        // any sample point they should be on DIFFERENT tasks (since they
        // started on different ones and swap in lockstep with the timer).
        assert_ne!(core0_entry, core1_entry, "cores ended up on same task");
        // Save-area pointers point inside their own slot region.
        let core0_save = read_u64(&cpu.mem, 0x4F08);
        let core1_save = read_u64(&cpu.mem, 0x5008);
        assert!((0x4F00..0x5000).contains(&core0_save), "core0 save_ptr escaped: {:#x}", core0_save);
        assert!((0x5000..0x6000).contains(&core1_save), "core1 save_ptr escaped: {:#x}", core1_save);
    }

    #[test]
    fn kernel_disk_read_populates_buffer() {
        let mut cpu = Cpu::new();
        // 35-inst kernel; run 50 to ensure both cores finished disk read.
        cpu.run(50);
        let buf = &cpu.mem[0x6000..0x6010];
        assert_eq!(&buf[..7], b"OSstudy");
        let snap = cpu.block.snapshot();
        assert!(snap.total_reads >= 2, "total_reads = {}", snap.total_reads);
        assert_eq!(snap.status, BLK_STATUS_OK);
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
        cpu.run(5);
        assert_eq!(cpu.cores[0].current_el, 1);
        assert_eq!(cpu.cores[0].daif, 0xF);
        // Remaining kernel boot is 30 more instructions (35 total per core)
        // before ERETing into EL0 with SPSR_EL1=0.
        cpu.run(30);
        assert_eq!(cpu.cores[0].current_el, 0);
        assert_eq!(cpu.cores[0].daif, 0);
    }

    #[test]
    fn ldrb_cbz_sub_imm_loop() {
        // Tiny program: count down X0 from 5 to 0 using SUB imm + CBZ.
        let mut cpu = Cpu::new();
        let prog = [
            movz(0, 5, 0),                // X0 = 5
            sub_imm(0, 0, 1),             // X0 -= 1
            cbnz(0, -1),                  // if X0 != 0, branch back -1 word
            b_self(),                     // halt
        ];
        write_words(&mut cpu.mem, ENTRY_PC, &prog);
        cpu.cores[0].sctlr_el1 = 0;
        cpu.cores[1].sctlr_el1 = 0;
        cpu.run(50);
        assert_eq!(cpu.cores[0].x[0], 0);
        // LDRB at PA 0x1000 (in real memory or UART range — no UART side effect
        // for a load): make sure byte zero-extends into a u64.
        cpu.mem[0x100] = 0xAB;
        let prog2 = [
            movz(1, 0x100, 0),
            ldrb_imm(2, 1, 0),
            b_self(),
        ];
        cpu.cores[0].pc = ENTRY_PC;
        cpu.cores[0].halted = false;
        cpu.cores[0].steps = 0;
        write_words(&mut cpu.mem, ENTRY_PC, &prog2);
        cpu.run(20);
        assert_eq!(cpu.cores[0].x[2], 0xAB);
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
