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

#[cfg(feature = "wasm")]
use wasm_bindgen::prelude::wasm_bindgen;

#[cfg(feature = "wasm")]
mod wasm;

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
    pub wfi_halted: bool,
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
    /// Cumulative number of IPIs the AIC has dispatched (every successful
    /// MMIO write to AIC_REG_IPI_SET counts once).
    pub total_ipis: u64,
    /// Index of the most recent IPI's target core, or None if no IPI yet.
    /// Cleared back to None at reset.
    pub last_ipi_target: Option<u32>,
}

// === Aic: tiny Apple-style interrupt controller ===============================

struct Aic {
    pending: [u32; NUM_CORES],
    total_acks: u64,
    total_ipis: u64,
    last_ipi_target: Option<u32>,
}

impl Aic {
    fn new() -> Self {
        Aic {
            pending: [0; NUM_CORES],
            total_acks: 0,
            total_ipis: 0,
            last_ipi_target: None,
        }
    }

    fn reset(&mut self) {
        for p in self.pending.iter_mut() {
            *p = 0;
        }
        self.total_acks = 0;
        self.total_ipis = 0;
        self.last_ipi_target = None;
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
                let target = val as u32;
                if (target as usize) < NUM_CORES {
                    self.set_irq(target as usize, IRQ_IPI);
                    self.total_ipis = self.total_ipis.saturating_add(1);
                    self.last_ipi_target = Some(target);
                }
            }
            _ => {}
        }
    }

    fn snapshot(&self) -> AicState {
        AicState {
            pending: self.pending.to_vec(),
            total_acks: self.total_acks,
            total_ipis: self.total_ipis,
            last_ipi_target: self.last_ipi_target,
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
        inscribe(&mut disk, 0, "AArch64 disk image - sector 0\n");
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
        // Preserve the disk image (the user's textarea edits live there) and
        // only clear ephemeral controller state.
        self.sector = 0;
        self.buf_addr = 0;
        self.last_command = 0;
        self.status = BLK_STATUS_IDLE;
        self.total_reads = 0;
        self.total_writes = 0;
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
    /// Set by WFI: the core stops fetching until an unmasked IRQ wakes it.
    /// Cleared when take_irq runs.
    wfi_halted: bool,
    /// Address reserved by the most-recent LDXR. STXR succeeds only while
    /// this matches; cleared by CLREX, by IRQ entry, and by a store from
    /// any other core to the same address.
    exclusive_monitor: Option<u64>,
    /// PA of the last store this core performed in this step. Cpu::step
    /// reads it and invalidates the matching monitor on the other cores.
    last_store_pa: Option<u64>,
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
            wfi_halted: false,
            exclusive_monitor: None,
            last_store_pa: None,
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
            wfi_halted: self.wfi_halted,
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
        self.exclusive_monitor = None; // exception entry clears the local monitor
        self.wfi_halted = false;
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
        let pa = r.pa.ok_or_else(|| r.fault.clone().unwrap_or_else(|| "MMU fault".into()))?;

        // AP-bit enforcement: at EL0, the leaf descriptor's AP[0] must be set
        // (user-accessible). AP=00 / AP=10 are kernel-only and fault for EL0.
        // Higher ELs are unrestricted in this toy.
        if self.current_el == 0 {
            let ap = match r.steps.last().map(|s| &s.outcome) {
                Some(WalkOutcome::Page { attrs, .. }) | Some(WalkOutcome::Block { attrs, .. }) => {
                    attrs.ap
                }
                _ => return Ok(pa),
            };
            if ap & 1 == 0 {
                return Err(format!(
                    "permission fault at VA {:#x} (AP={:#04b}, EL0)",
                    va, ap
                ));
            }
        }
        Ok(pa)
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
        &mut self,
        mem: &mut [u8],
        out: &mut Vec<u8>,
        aic: &mut Aic,
        block: &mut Block,
        va: u64,
        val: u64,
    ) -> Result<(), String> {
        let pa = self.translate_for_access(mem, va)?;
        self.last_store_pa = Some(pa);
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

    /// Public translation entry: walk + AP-bit enforcement based on this core's
    /// current_el. translate_for_access (instruction path) and the JS-callable
    /// translate() both go through here.
    fn do_translate(&self, mem: &[u8], va: u64) -> TranslationResult {
        let mut r = self.do_translate_walk(mem, va);
        if r.pa.is_some() && self.current_el == 0 {
            let ap = match r.steps.last().map(|s| &s.outcome) {
                Some(WalkOutcome::Page { attrs, .. })
                | Some(WalkOutcome::Block { attrs, .. }) => attrs.ap,
                _ => return r,
            };
            if ap & 1 == 0 {
                r.fault = Some(format!(
                    "permission fault at VA {:#x} (AP={:#04b}, EL0)",
                    va, ap
                ));
                r.pa = None;
            }
        }
        r
    }

    fn do_translate_walk(&self, mem: &[u8], va: u64) -> TranslationResult {
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

        // LDXR Xt, [Xn] :: 1100_1000_0101_1111_0111_1100_Rn_Rt   (size=11, L=1, excl)
        if insn & 0xFFFF_FC00 == 0xC85F_7C00 {
            let rt = (insn & 0x1F) as usize;
            let rn = ((insn >> 5) & 0x1F) as usize;
            let va = self.read_x(rn);
            let pa = self.translate_for_access(mem, va)?;
            let val = read_pa_u64(mem, pa)?;
            self.write_x(rt, val);
            self.exclusive_monitor = Some(pa);
            self.pc = self.pc.wrapping_add(4);
            return Ok(StepResult::Continue);
        }

        // STXR Ws, Xt, [Xn] :: 1100_1000_000_Rs_0_11111_Rn_Rt   (size=11, L=0, excl)
        if insn & 0xFFE0_FC00 == 0xC800_7C00 {
            let rt = (insn & 0x1F) as usize;
            let rn = ((insn >> 5) & 0x1F) as usize;
            let rs = ((insn >> 16) & 0x1F) as usize;
            let va = self.read_x(rn);
            let pa = self.translate_for_access(mem, va)?;
            let succeeded = self.exclusive_monitor == Some(pa);
            self.exclusive_monitor = None;
            if succeeded {
                let val = self.read_x(rt);
                self.last_store_pa = Some(pa);
                write_pa_u64(mem, pa, val)?;
                self.write_x(rs, 0);
            } else {
                self.write_x(rs, 1);
            }
            self.pc = self.pc.wrapping_add(4);
            return Ok(StepResult::Continue);
        }

        // CLREX :: 0xD503_3F5F (CRm=1111, the only encoding we emit)
        if insn & 0xFFFF_F0FF == 0xD503_305F {
            self.exclusive_monitor = None;
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
                // WFI :: 0xD503_207F — set wfi_halted, advance PC.
                if insn == 0xD503_207F {
                    self.wfi_halted = true;
                    self.pc = self.pc.wrapping_add(4);
                    return Ok(StepResult::Continue);
                }
                // Other hints (NOP/YIELD/WFE/SEV…) and barriers — no-op.
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
            self.exclusive_monitor = None; // any exception entry clears the monitor
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

#[cfg_attr(feature = "wasm", wasm_bindgen)]
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

impl Cpu {
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

        let mut any_runnable = false;
        for i in 0..self.cores.len() {
            if self.cores[i].halted {
                continue;
            }
            // A WFI'd core counts as "runnable" — the system still needs to
            // tick because a future timer IRQ can wake it. Only fully halted
            // cores stop the run() loop.
            any_runnable = true;
            if self.aic.has_pending(i) && !self.cores[i].irq_masked() {
                self.cores[i].take_irq();
            } else if self.cores[i].wfi_halted {
                // sleeping — no fetch this step
            } else {
                self.cores[i].step(
                    &mut self.mem,
                    &mut self.output_buf,
                    &mut self.aic,
                    &mut self.block,
                );
            }
            self.invalidate_remote_monitors(i);
        }
        any_runnable
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
        let progressed = if self.aic.has_pending(i) && !self.cores[i].irq_masked() {
            self.cores[i].take_irq();
            true
        } else if self.cores[i].wfi_halted {
            false
        } else {
            self.cores[i].step(
                &mut self.mem,
                &mut self.output_buf,
                &mut self.aic,
                &mut self.block,
            )
        };
        self.invalidate_remote_monitors(i);
        progressed
    }

    /// Implements the cross-core half of the exclusive monitor: a store from
    /// core `i` clears every *other* core's reservation if it matches the
    /// stored address. This is what makes LDXR/STXR a real concurrency
    /// primitive — without it two cores could both think their CAS won.
    fn invalidate_remote_monitors(&mut self, i: usize) {
        let Some(pa) = self.cores[i].last_store_pa.take() else {
            return;
        };
        for (j, c) in self.cores.iter_mut().enumerate() {
            if j != i && c.exclusive_monitor == Some(pa) {
                c.exclusive_monitor = None;
            }
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

    pub fn aic_state(&self) -> AicState {
        self.aic.snapshot()
    }

    pub fn block_state(&self) -> BlockState {
        self.block.snapshot()
    }

    /// Replace disk sector 0 with the given UTF-8 text (padded with zeros to
    /// SECTOR_SIZE bytes). Also patches the live disk-buffer page at PA 0x6000
    /// so task B's printer reflects the change without a reset.
    pub fn set_disk_text(&mut self, text: &str) {
        let bytes = text.as_bytes();
        let n = bytes.len().min(SECTOR_SIZE as usize);
        for i in 0..(SECTOR_SIZE as usize) {
            let b = if i < n { bytes[i] } else { 0 };
            self.block.disk[i] = b;
            self.mem[0x6000 + i] = b;
        }
    }

    pub fn disk_text(&self) -> String {
        let mut end = SECTOR_SIZE as usize;
        for i in 0..(SECTOR_SIZE as usize) {
            if self.block.disk[i] == 0 {
                end = i;
                break;
            }
        }
        String::from_utf8_lossy(&self.block.disk[..end]).into_owned()
    }

    /// Returns one CoreState per core.
    pub fn state(&self) -> Vec<CoreState> {
        self.cores.iter().map(|c| c.snapshot()).collect()
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
    /// Returns `None` if `core_idx` is out of range.
    pub fn translate(&self, va: u64, core_idx: u32) -> Option<TranslationResult> {
        self.cores
            .get(core_idx as usize)
            .map(|c| c.do_translate(&self.mem, va))
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

    /// Reads the shared u64 atomic counter at PA 0x6FF8 — task A's LDXR/STXR
    /// loop bumps it once per scheduling round.
    pub fn atomic_counter(&self) -> u64 {
        read_pa_u64(&self.mem, ATOMIC_COUNTER_PA_PUB).unwrap_or(0)
    }
}

const ATOMIC_COUNTER_PA_PUB: u64 = 0x6FF8;

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

// === Disassembler ============================================================
// Mirrors the decoder in Core::execute, producing ARM-style mnemonics. Used
// by the JS UI to render a Disassembly panel; not used by execution itself.

pub fn disassemble(insn: u32, pc: u64) -> String {
    // MOVZ Xd, #imm16 {, LSL #hw*16}
    if insn & 0xFF80_0000 == 0xD280_0000 {
        let rd = insn & 0x1F;
        let hw = (insn >> 21) & 0x3;
        let imm = (insn >> 5) & 0xFFFF;
        return if hw == 0 {
            format!("movz x{rd}, #{imm:#x}")
        } else {
            format!("movz x{rd}, #{imm:#x}, lsl #{}", hw * 16)
        };
    }
    // ADD Xd, Xn, #imm12 {, LSL #12}
    if insn & 0xFF80_0000 == 0x9100_0000 {
        let rd = insn & 0x1F;
        let rn = (insn >> 5) & 0x1F;
        let imm = (insn >> 10) & 0xFFF;
        let sh = (insn >> 22) & 1;
        return if sh == 1 {
            format!("add x{rd}, x{rn}, #{imm:#x}, lsl #12")
        } else {
            format!("add x{rd}, x{rn}, #{imm:#x}")
        };
    }
    // SUB Xd, Xn, #imm12
    if insn & 0xFF80_0000 == 0xD100_0000 {
        let rd = insn & 0x1F;
        let rn = (insn >> 5) & 0x1F;
        let imm = (insn >> 10) & 0xFFF;
        return format!("sub x{rd}, x{rn}, #{imm:#x}");
    }
    // ADD Xd, Xn, Xm
    if insn & 0xFF20_FC00 == 0x8B00_0000 {
        let rd = insn & 0x1F;
        let rn = (insn >> 5) & 0x1F;
        let rm = (insn >> 16) & 0x1F;
        return format!("add x{rd}, x{rn}, x{rm}");
    }
    // SUB Xd, Xn, Xm
    if insn & 0xFF20_FC00 == 0xCB00_0000 {
        let rd = insn & 0x1F;
        let rn = (insn >> 5) & 0x1F;
        let rm = (insn >> 16) & 0x1F;
        return format!("sub x{rd}, x{rn}, x{rm}");
    }
    // STR Xt, [Xn, #imm12*8]
    if insn & 0xFFC0_0000 == 0xF900_0000 {
        let rt = insn & 0x1F;
        let rn = (insn >> 5) & 0x1F;
        let imm = (insn >> 10) & 0xFFF;
        let off = imm * 8;
        return if off == 0 {
            format!("str x{rt}, [x{rn}]")
        } else {
            format!("str x{rt}, [x{rn}, #{off}]")
        };
    }
    // LDR Xt, [Xn, #imm12*8]
    if insn & 0xFFC0_0000 == 0xF940_0000 {
        let rt = insn & 0x1F;
        let rn = (insn >> 5) & 0x1F;
        let imm = (insn >> 10) & 0xFFF;
        let off = imm * 8;
        return if off == 0 {
            format!("ldr x{rt}, [x{rn}]")
        } else {
            format!("ldr x{rt}, [x{rn}, #{off}]")
        };
    }
    // LDRB Wt, [Xn, #imm12]
    if insn & 0xFFC0_0000 == 0x3940_0000 {
        let rt = insn & 0x1F;
        let rn = (insn >> 5) & 0x1F;
        let imm = (insn >> 10) & 0xFFF;
        return if imm == 0 {
            format!("ldrb w{rt}, [x{rn}]")
        } else {
            format!("ldrb w{rt}, [x{rn}, #{imm}]")
        };
    }
    // LDP/STP signed offset
    if insn & 0xFFC0_0000 == 0xA900_0000 || insn & 0xFFC0_0000 == 0xA940_0000 {
        let l = (insn >> 22) & 1;
        let imm7_raw = (insn >> 15) & 0x7F;
        let imm7 = ((imm7_raw as i32) << 25) >> 25;
        let off = imm7 * 8;
        let rt2 = (insn >> 10) & 0x1F;
        let rn = (insn >> 5) & 0x1F;
        let rt1 = insn & 0x1F;
        let mnem = if l == 0 { "stp" } else { "ldp" };
        return if off == 0 {
            format!("{mnem} x{rt1}, x{rt2}, [x{rn}]")
        } else {
            format!("{mnem} x{rt1}, x{rt2}, [x{rn}, #{off}]")
        };
    }
    // LDXR Xt, [Xn]
    if insn & 0xFFFF_FC00 == 0xC85F_7C00 {
        let rt = insn & 0x1F;
        let rn = (insn >> 5) & 0x1F;
        return format!("ldxr x{rt}, [x{rn}]");
    }
    // STXR Ws, Xt, [Xn]
    if insn & 0xFFE0_FC00 == 0xC800_7C00 {
        let rt = insn & 0x1F;
        let rn = (insn >> 5) & 0x1F;
        let rs = (insn >> 16) & 0x1F;
        return format!("stxr w{rs}, x{rt}, [x{rn}]");
    }
    // CLREX (any CRm — canonical CRm=1111)
    if insn & 0xFFFF_F0FF == 0xD503_305F {
        return "clrex".into();
    }
    // SVC #imm16
    if insn & 0xFFE0_001F == 0xD400_0001 {
        let imm = (insn >> 5) & 0xFFFF;
        return format!("svc #{imm:#x}");
    }
    // ERET
    if insn == 0xD69F_03E0 {
        return "eret".into();
    }
    // WFI
    if insn == 0xD503_207F {
        return "wfi".into();
    }
    // Hint class
    if insn & 0xFFFF_F01F == 0xD503_201F {
        let hint = (insn >> 5) & 0x7F;
        return match hint {
            0 => "nop".into(),
            1 => "yield".into(),
            2 => "wfe".into(),
            3 => "wfi".into(),
            n => format!("hint #{n}"),
        };
    }
    // Barrier class
    if insn & 0xFFFF_F01F == 0xD503_301F {
        let crm = (insn >> 8) & 0xF;
        let op2 = (insn >> 5) & 0x7;
        let mnem = match op2 {
            4 => Some("dsb"),
            5 => Some("dmb"),
            6 => Some("isb"),
            _ => None,
        };
        let domain = match crm {
            0xF => Some("sy"),
            0xE => Some("st"),
            0xD => Some("ld"),
            0xB => Some("ish"),
            _ => None,
        };
        if let (Some(m), Some(d)) = (mnem, domain) {
            return format!("{m} {d}");
        }
    }
    // MSR / MRS sysreg or DAIFSet/Clr
    if insn & 0xFFC0_0000 == 0xD500_0000 {
        let l = (insn >> 21) & 1;
        let op0 = (insn >> 19) & 0x3;
        let op1 = (insn >> 16) & 0x7;
        let crn = (insn >> 12) & 0xF;
        let crm = (insn >> 8) & 0xF;
        let op2 = (insn >> 5) & 0x7;
        let rt = insn & 0x1F;
        if op0 == 0 && op1 == 3 && crn == 4 && (op2 == 6 || op2 == 7) {
            let pf = if op2 == 6 { "DAIFSet" } else { "DAIFClr" };
            return format!("msr {pf}, #{crm:#x}");
        }
        if op0 >= 2 {
            let sr = sysreg_name(op0, op1, crn, crm, op2);
            return if l == 0 {
                format!("msr {sr}, x{rt}")
            } else {
                format!("mrs x{rt}, {sr}")
            };
        }
    }
    // CBZ / CBNZ Xt, label
    if insn & 0xFE00_0000 == 0xB400_0000 {
        let op = (insn >> 24) & 1;
        let imm19_raw = (insn >> 5) & 0x7_FFFF;
        let imm19 = ((imm19_raw as i32) << 13) >> 13;
        let target = (pc as i64).wrapping_add((imm19 as i64) * 4) as u64;
        let rt = insn & 0x1F;
        let mnem = if op == 0 { "cbz" } else { "cbnz" };
        return format!("{mnem} x{rt}, {target:#x}");
    }
    // B label
    if insn & 0xFC00_0000 == 0x1400_0000 {
        let imm26_raw = (insn & 0x03FF_FFFF) as i32;
        let imm26 = (imm26_raw << 6) >> 6;
        let target = (pc as i64).wrapping_add((imm26 as i64) * 4) as u64;
        return format!("b {target:#x}");
    }
    // Unknown
    format!(".word {insn:#010x}")
}

fn sysreg_name(op0: u32, op1: u32, crn: u32, crm: u32, op2: u32) -> String {
    match (op0, op1, crn, crm, op2) {
        (3, 0, 0, 0, 5) => "mpidr_el1".into(),
        (3, 0, 1, 0, 0) => "sctlr_el1".into(),
        (3, 0, 2, 0, 0) => "ttbr0_el1".into(),
        (3, 0, 2, 0, 2) => "tcr_el1".into(),
        (3, 0, 4, 0, 0) => "spsr_el1".into(),
        (3, 0, 4, 0, 1) => "elr_el1".into(),
        (3, 0, 4, 2, 2) => "currentel".into(),
        (3, 0, 5, 2, 0) => "esr_el1".into(),
        (3, 0, 12, 0, 0) => "vbar_el1".into(),
        (3, 4, 4, 0, 0) => "spsr_el2".into(),
        (3, 4, 4, 0, 1) => "elr_el2".into(),
        (3, 4, 5, 2, 0) => "esr_el2".into(),
        (3, 4, 12, 0, 0) => "vbar_el2".into(),
        _ => format!("s{op0}_{op1}_c{crn}_c{crm}_{op2}"),
    }
}

// === Demo program =============================================================

fn load_demo(mem: &mut [u8]) {
    // Memory regions, both cores execute the same code:
    //   PA 0x4000  kernel boot
    //   PA 0x4800  sync handler (stub)
    //   PA 0x4880  IRQ handler = scheduler with X0-X3 save/restore
    //   PA 0x4D00  task A — silent WFI sleeper (pinned to core 0)
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
    // Atomic counter sits in the same user-mapped page as the disk buffer
    // (0x6000–0x6FFF), well past the 64-byte sector. Task A bumps it via
    // LDXR/STXR; the UI reads it from RAM.
    const ATOMIC_COUNTER_PA: u32 = 0x6FF8;
    // Sync handler dispatches IPIs to core 1 via AIC_REG_IPI_SET (offset
    // 0x10 = imm12 #2 in an 8-byte-scaled STR).
    const IPI_TARGET: u32 = 1;
    const IPI_OFFSET_WORDS: u32 = (AIC_REG_IPI_SET / 8) as u32;
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
    // Sync handler at VBAR+0x400 — kernel-side IPI service. Task A calls
    // SVC #0 to ask the kernel to ping core 1 via the AIC, then ERETs.
    // Real systems route IPIs through a privileged path because the AIC
    // page is AP=00 (kernel-only); user code must trap to ask.
    let sync_handler: [u32; 4] = [
        movz(9, AIC_BASE as u32, 0),     // 0: X9 = AIC_BASE
        movz(10, IPI_TARGET as u32, 0),  // 1: X10 = peer core id (1)
        str_imm(10, 9, IPI_OFFSET_WORDS), // 2: STR X10, [X9, #0x10]
        eret(),                          // 3: back to task A
    ];
    // IRQ handler at VBAR+0x480 — minimal: ack the AIC and ERET back to the
    // *same* preempted task. Tasks are pinned per core (core 0 = task A,
    // core 1 = task B), so no context-switch is needed and the UART output
    // stays monotonic (only one core ever writes it).
    let scheduler: [u32; 3] = [
        movz(9, AIC_BASE as u32, 0), // 0: X9 = AIC_BASE
        ldr_imm(10, 9, 0),           // 1: X10 = irq id (read clears pending)
        eret(),                      // 2: return to preempted task
    ];
    // Task A — atomic counter + IPI generator. Each scheduling round it
    // (1) atomically bumps the u64 at PA 0x6FF8 via an LDXR/STXR pair,
    // (2) issues SVC #0 to ask the kernel to ping core 1 via the AIC, and
    // (3) WFIs until the next IRQ. The MOVZ that loads the counter
    // address runs once at first entry; subsequent resumes loop back to
    // the LDXR. Does not touch UART, so task B's output stays clean.
    let task_a: [u32; 8] = [
        movz(4, ATOMIC_COUNTER_PA, 0), // 0: X4 = &counter (PA 0x6FF8)
        ldxr(5, 4),                    // 1: X5 = *X4, set local monitor
        add_imm(5, 5, 1),              // 2: X5 += 1
        stxr(6, 5, 4),                 // 3: try CAS, W6 = 0/1 (ok/retry)
        cbnz(6, -2),                   // 4: if W6 != 0 → branch back to LDXR
        svc_imm(0),                    // 5: kernel: please ping core 1
        wfi(),                         // 6: sleep until next IRQ
        b_offset(-6),                  // 7: on resume → back to LDXR
    ];
    // Task B — "disk printer". Walks the 64-byte disk buffer at PA 0x6000
    // once, emitting each byte to the UART. On hitting the null terminator
    // it parks itself in WFI forever so the disk content is printed exactly
    // once. The trailing `b -1` keeps us pinned at the WFI on every IRQ
    // resume (otherwise PC advances past WFI into UDF).
    let task_b: [u32; 10] = [
        movz(1, UART_OUT as u32, 0), // 0: X1 = UART
        movz(4, 0x6000, 0),          // 1: X4 = disk buffer base
        add_reg(4, 4, 3),            // 2: X4 += X3
        ldrb_imm(0, 4, 0),           // 3: W0 = byte at X4
        cbz(0, 4),                   // 4: if zero, branch to inst 8 (wfi)
        str_imm(0, 1, 0),            // 5: STR X0, [X1] — emit byte to UART
        add_imm(3, 3, 1),            // 6: X3 += 1
        b_offset(-6),                // 7: → inst 1 (re-init X4 = 0x6000 + X3)
        wfi(),                       // 8: parked — printed once, sleep
        b_offset(-1),                // 9: on IRQ resume, jump back to wfi
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
    // AF=1, valid+page (b11). AP at bits [7:6]:
    //   AP=00 → kernel R/W, EL0 no access.
    //   AP=01 → kernel R/W, EL0 R/W.
    let kern = (1u64 << 10) | 0b11; // AP=00, kernel-only
    let user = (1u64 << 10) | (0b01 << 6) | 0b11; // AP=01, user-accessible
    // User-accessible pages — tasks fetch / load / store from these.
    write_u64(mem, L3_TABLE_PA + 8, 0x1000 | user); // UART
    write_u64(mem, L3_TABLE_PA + 4 * 8, 0x4000 | user); // program/code
    write_u64(mem, L3_TABLE_PA + 6 * 8, 0x6000 | user); // disk buffer (task B)
    // Kernel-only pages — only EL1 (kernel handlers) can touch these.
    write_u64(mem, L3_TABLE_PA + 2 * 8, 0x2000 | kern); // AIC MMIO
    write_u64(mem, L3_TABLE_PA + 3 * 8, 0x3000 | kern); // Block MMIO
    write_u64(mem, L3_TABLE_PA + 5 * 8, 0x5000 | kern); // core 1 slot region
    write_u64(mem, L3_TABLE_PA + 8 * 8, 0x8000 | kern); // L1 table page
    write_u64(mem, L3_TABLE_PA + 9 * 8, 0x9000 | kern); // L2 table page
    write_u64(mem, L3_TABLE_PA + 0xA * 8, 0xA000 | kern); // L3 table page
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

const fn wfi() -> u32 {
    0xD503_207F
}

/// LDXR Xt, [Xn] — load-exclusive 64-bit. Sets the local monitor to PA(Xn).
const fn ldxr(rt: u32, rn: u32) -> u32 {
    0xC85F_7C00 | ((rn & 0x1F) << 5) | (rt & 0x1F)
}

/// STXR Ws, Xt, [Xn] — store-exclusive 64-bit. Writes Ws=0 on success, 1 on
/// monitor mismatch. Always clears the local monitor.
const fn stxr(rs: u32, rt: u32, rn: u32) -> u32 {
    0xC800_7C00 | ((rs & 0x1F) << 16) | ((rn & 0x1F) << 5) | (rt & 0x1F)
}

/// CLREX — clear the local monitor unconditionally.
#[allow(dead_code)]
const fn clrex() -> u32 {
    0xD503_3F5F
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
    fn disassemble_covers_supported_instructions() {
        assert_eq!(disassemble(movz(9, 0x4014, 0), 0x4000), "movz x9, #0x4014");
        assert_eq!(
            disassemble(movz(9, 0x8000, 1), 0x4000),
            "movz x9, #0x8000, lsl #16"
        );
        assert_eq!(disassemble(add_imm(0, 0, 0x1), 0x4000), "add x0, x0, #0x1");
        assert_eq!(disassemble(sub_imm(3, 3, 1), 0x4000), "sub x3, x3, #0x1");
        assert_eq!(disassemble(add_reg(14, 9, 14), 0x4000), "add x14, x9, x14");
        assert_eq!(disassemble(sub_reg(13, 13, 11), 0x4000), "sub x13, x13, x11");
        assert_eq!(disassemble(str_imm(0, 1, 0), 0x4000), "str x0, [x1]");
        assert_eq!(disassemble(str_imm(0, 1, 1), 0x4000), "str x0, [x1, #8]");
        assert_eq!(disassemble(ldr_imm(11, 14, 0), 0x4000), "ldr x11, [x14]");
        assert_eq!(disassemble(ldrb_imm(0, 4, 0), 0x4000), "ldrb w0, [x4]");
        assert_eq!(disassemble(stp_imm(0, 1, 12, 0), 0x4000), "stp x0, x1, [x12]");
        assert_eq!(disassemble(svc_imm(0), 0x4000), "svc #0x0");
        assert_eq!(disassemble(eret(), 0x4000), "eret");
        assert_eq!(disassemble(wfi(), 0x4000), "wfi");
        assert_eq!(disassemble(isb(), 0x4000), "isb sy");
        assert_eq!(disassemble(ldxr(5, 4), 0x4000), "ldxr x5, [x4]");
        assert_eq!(disassemble(stxr(6, 5, 4), 0x4000), "stxr w6, x5, [x4]");
        assert_eq!(disassemble(clrex(), 0x4000), "clrex");
        assert_eq!(disassemble(msr_ttbr0(9), 0x4000), "msr ttbr0_el1, x9");
        assert_eq!(disassemble(mrs_mpidr(14), 0x4000), "mrs x14, mpidr_el1");
        assert_eq!(disassemble(0xD503_42FFu32, 0x4000), "msr DAIFClr, #0x2");
        // Branch with PC-relative target.
        assert_eq!(disassemble(b_offset(-1), 0x4020), "b 0x401c");
        // Unknown instruction.
        assert_eq!(disassemble(0xDEAD_BEEF, 0), ".word 0xdeadbeef");
    }

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
        // ERETed: core 0 into task A (silent WFI), core 1 into task B (the
        // disk printer that walks "AArch64 disk image - sector 0\n").
        let mut cpu = Cpu::new();
        cpu.run(60);
        let out = cpu.output();
        assert!(!out.is_empty(), "no output yet: {:?}", out);
        // Only task B prints; output begins with the disk content prefix.
        assert!(out.starts_with('A'), "expected disk-content prefix, got: {:?}", out);
        assert!(!cpu.cores[0].halted);
        assert!(!cpu.cores[1].halted);
    }

    #[test]
    fn tasks_pinned_per_core_after_irqs() {
        let mut cpu = Cpu::new();
        // Run long enough for ≥ 2 timer ticks. With the minimal scheduler
        // each core's task entry stays the same across IRQs.
        cpu.run(200);
        assert!(cpu.timer_ticks >= 2);
        let out = cpu.output();
        // Task B (core 1) printed once and parked in WFI. Output is the disk
        // content with no task-A interleaving, length capped at 64 bytes.
        assert!(out.starts_with('A'), "expected disk-content prefix, got: {:?}", out);
        assert!(out.len() <= 64, "task B should print once, got {} bytes", out.len());
    }

    #[test]
    fn each_core_uses_its_own_slot_region() {
        let mut cpu = Cpu::new();
        cpu.run(400);
        let read_u64 = |mem: &[u8], pa: usize| -> u64 {
            u64::from_le_bytes(mem[pa..pa + 8].try_into().unwrap())
        };
        // Core 0's slot is at 0x4F00, core 1's at 0x5000. With pinned tasks
        // they stay on their initial entries (A on core 0, B on core 1).
        let core0_entry = read_u64(&cpu.mem, 0x4F00);
        let core1_entry = read_u64(&cpu.mem, 0x5000);
        assert_eq!(core0_entry, 0x4D00, "core 0 should be on task A");
        assert_eq!(core1_entry, 0x4E00, "core 1 should be on task B");
        // Save-area pointers stay inside their own slot region.
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
        assert_eq!(&buf[..7], b"AArch64");
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
    fn ap_bits_block_user_access_to_kernel_pages() {
        let mut cpu = Cpu::new();
        // Boot through kernel so MMU is active and at EL0/EL1 for both cores.
        cpu.run(40);
        // Core 0 is at EL0 (running task A). Translation of UART (user) should
        // succeed.
        let r_user = cpu.cores[0].do_translate(&cpu.mem, 0x1000);
        assert_eq!(r_user.pa, Some(0x1000));
        // Translation of AIC at EL0 should fail with permission fault when
        // checked through translate_for_access.
        let res = cpu.cores[0].translate_for_access(&cpu.mem, 0x2000);
        assert!(res.is_err(), "EL0 access to AIC should fault: {:?}", res);
        let err = res.unwrap_err();
        assert!(
            err.contains("permission fault"),
            "expected permission fault, got: {}",
            err
        );

        // Force core 0 into EL1 — kernel can access AIC freely.
        cpu.cores[0].current_el = 1;
        let res = cpu.cores[0].translate_for_access(&cpu.mem, 0x2000);
        assert_eq!(res, Ok(0x2000));
    }

    #[test]
    fn wfi_sleeps_with_irqs_masked() {
        // Hand-built program: print 'X', WFI, B-back. With DAIF.I masked the
        // timer won't wake us — we test that WFI parks the core indefinitely
        // and the rest of the system keeps ticking.
        let mut cpu = Cpu::new();
        let prog = [
            movz(1, UART_OUT as u32, 0),
            movz(0, b'X' as u32, 0),
            str_imm(0, 1, 0),
            wfi(),
            b_offset(-1),
        ];
        write_words(&mut cpu.mem, ENTRY_PC, &prog);
        cpu.cores[0].sctlr_el1 = 0;
        cpu.cores[0].daif = 0xF; // IRQs masked, WFI is permanent here
        cpu.cores[1].halted = true;

        cpu.run(20);
        assert!(cpu.cores[0].wfi_halted, "WFI not entered");
        assert_eq!(cpu.output(), "X");

        // The system_steps clock keeps advancing while a core is WFI'd.
        let before = cpu.system_steps;
        cpu.run(50);
        assert!(cpu.system_steps > before, "system_steps didn't advance during WFI");
        assert!(cpu.cores[0].wfi_halted, "WFI lifted with IRQs masked?");
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

    /// Helper for the LL/SC tests below — a tiny program does
    /// LDXR/ADD/STXR at PA 0x100 then halts.
    fn ll_sc_program() -> [u32; 5] {
        [
            movz(4, 0x100, 0),       // X4 = &counter
            ldxr(5, 4),              // X5 = *X4
            add_imm(5, 5, 1),        // X5 += 1
            stxr(6, 5, 4),           // try CAS
            b_self(),                // halt
        ]
    }

    #[test]
    fn stxr_succeeds_after_clean_ldxr() {
        let mut cpu = Cpu::new();
        write_words(&mut cpu.mem, ENTRY_PC, &ll_sc_program());
        cpu.mem[0x100..0x108].copy_from_slice(&41u64.to_le_bytes());
        cpu.cores[0].sctlr_el1 = 0;
        cpu.cores[1].halted = true;
        cpu.run(20);
        let cnt = u64::from_le_bytes(cpu.mem[0x100..0x108].try_into().unwrap());
        assert_eq!(cnt, 42, "STXR did not commit");
        assert_eq!(cpu.cores[0].x[6], 0, "STXR success flag should be 0");
    }

    #[test]
    fn stxr_fails_after_clrex() {
        // LDXR; CLREX; STXR — the STXR must fail because CLREX wiped the monitor.
        let mut cpu = Cpu::new();
        let prog = [
            movz(4, 0x100, 0),
            ldxr(5, 4),
            clrex(),
            stxr(6, 5, 4),
            b_self(),
        ];
        write_words(&mut cpu.mem, ENTRY_PC, &prog);
        cpu.mem[0x100..0x108].copy_from_slice(&7u64.to_le_bytes());
        cpu.cores[0].sctlr_el1 = 0;
        cpu.cores[1].halted = true;
        cpu.run(20);
        let cnt = u64::from_le_bytes(cpu.mem[0x100..0x108].try_into().unwrap());
        assert_eq!(cnt, 7, "STXR after CLREX must NOT commit");
        assert_eq!(cpu.cores[0].x[6], 1, "STXR fail flag should be 1");
    }

    #[test]
    fn irq_clears_local_monitor() {
        // After LDXR the monitor is set; IRQ entry must clear it so a later
        // STXR fails. We simulate by calling take_irq() directly.
        let mut cpu = Cpu::new();
        cpu.cores[0].exclusive_monitor = Some(0x100);
        cpu.cores[0].vbar_el1 = 0x4400;
        cpu.cores[0].take_irq();
        assert_eq!(cpu.cores[0].exclusive_monitor, None);
    }

    #[test]
    fn cross_core_store_invalidates_monitor() {
        // Core 0 reserves PA 0x100. Core 1 stores to 0x100. Core 0's
        // subsequent STXR must fail and the counter must reflect core 1's
        // store, not core 0's.
        let mut cpu = Cpu::new();
        cpu.mem[0x100..0x108].copy_from_slice(&100u64.to_le_bytes());
        cpu.cores[0].sctlr_el1 = 0;
        cpu.cores[1].sctlr_el1 = 0;
        // Core 0: LDXR; B-here (busy-wait until core 1 stores); STXR; halt.
        // We sequence the cores by setting them up at different PCs and
        // stepping each manually.
        let core0_prog = [
            movz(4, 0x100, 0),
            ldxr(5, 4),
            add_imm(5, 5, 1),
            // Pause here — caller advances core 1 first.
            b_self(),
            stxr(6, 5, 4),
            b_self(),
        ];
        let core1_prog = [
            movz(4, 0x100, 0),
            movz(5, 999, 0),
            str_imm(5, 4, 0),
            b_self(),
        ];
        const C0_BASE: u64 = ENTRY_PC;
        const C1_BASE: u64 = ENTRY_PC + 0x80;
        write_words(&mut cpu.mem, C0_BASE, &core0_prog);
        write_words(&mut cpu.mem, C1_BASE, &core1_prog);
        cpu.cores[1].pc = C1_BASE;

        // Step core 0 through MOVZ + LDXR + ADD; reservation now held.
        for _ in 0..3 {
            cpu.step_core(0);
        }
        assert_eq!(cpu.cores[0].exclusive_monitor, Some(0x100));

        // Step core 1 through MOVZ + MOVZ + STR — its store invalidates
        // core 0's monitor.
        for _ in 0..3 {
            cpu.step_core(1);
        }
        assert_eq!(cpu.cores[0].exclusive_monitor, None,
            "core 1's store should have wiped core 0's reservation");

        // Now run core 0's STXR — must fail (W6 = 1) and leave counter at 999.
        cpu.cores[0].pc = C0_BASE + 4 * 4; // skip the b_self
        cpu.step_core(0);
        let cnt = u64::from_le_bytes(cpu.mem[0x100..0x108].try_into().unwrap());
        assert_eq!(cnt, 999, "core 1's value must survive — STXR failed");
        assert_eq!(cpu.cores[0].x[6], 1, "STXR fail flag should be 1");
    }

    #[test]
    fn task_a_atomic_counter_increments() {
        // Boot the demo and let task A churn for many cycles; the atomic
        // counter at 0x6FF8 must be strictly increasing.
        let mut cpu = Cpu::new();
        cpu.run(2000);
        let v1 = cpu.atomic_counter();
        cpu.run(2000);
        let v2 = cpu.atomic_counter();
        assert!(v1 > 0, "task A's counter never advanced past zero");
        assert!(v2 > v1, "counter stalled: {v1} -> {v2}");
    }

    #[test]
    fn aic_ipi_raises_target_only() {
        // Direct AIC.mmio_write to IPI_SET should raise pending on the
        // target core only, and bump total_ipis / last_ipi_target.
        let mut aic = Aic::new();
        aic.mmio_write(0, AIC_REG_IPI_SET, 1);
        assert!(aic.has_pending(1), "IPI did not raise pending on core 1");
        assert!(!aic.has_pending(0), "IPI leaked onto sender");
        let snap = aic.snapshot();
        assert_eq!(snap.total_ipis, 1);
        assert_eq!(snap.last_ipi_target, Some(1));
        // ACK from core 1 returns IRQ_IPI and clears the bit.
        let id = aic.read_ack(1);
        assert_eq!(id, IRQ_IPI);
        assert!(!aic.has_pending(1));
    }

    #[test]
    fn aic_ipi_to_invalid_core_is_dropped() {
        let mut aic = Aic::new();
        aic.mmio_write(0, AIC_REG_IPI_SET, 99);
        assert_eq!(aic.snapshot().total_ipis, 0);
        assert!(aic.snapshot().last_ipi_target.is_none());
    }

    #[test]
    fn task_a_svc_dispatches_ipis() {
        // End-to-end demo run: task A's SVC #0 handler should write to
        // AIC_REG_IPI_SET and the AIC counter should grow.
        let mut cpu = Cpu::new();
        cpu.run(3000);
        let snap = cpu.aic.snapshot();
        assert!(snap.total_ipis > 0, "no IPIs dispatched after 3000 steps");
        assert_eq!(snap.last_ipi_target, Some(1), "expected IPIs to target core 1");
    }

    #[test]
    fn ipi_wakes_a_wfi_d_core() {
        // Park core 1 in WFI with IRQs unmasked (DAIF=0). Have core 0 send
        // an IPI directly via the AIC. Core 1 must take the IRQ.
        let mut cpu = Cpu::new();
        // Boot far enough that core 1 is parked in task B's WFI.
        cpu.run(500);
        assert!(cpu.cores[1].wfi_halted, "core 1 should be parked in WFI");
        let pc_before = cpu.cores[1].pc;
        cpu.aic.mmio_write(0, AIC_REG_IPI_SET, 1);
        // One step is enough for take_irq to fire on core 1.
        cpu.step();
        assert!(!cpu.cores[1].wfi_halted, "WFI not lifted by IPI");
        assert_ne!(cpu.cores[1].pc, pc_before, "core 1 PC unchanged after IPI");
    }
}
