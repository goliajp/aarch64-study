//! Tiny AArch64 instruction simulator for v0.1.
//!
//! Supports 5 instruction families: MOVZ, ADD (imm), LDR (unsigned offset),
//! STR (unsigned offset), B. Memory is 64 KiB. Address 0x1000 is a memory-mapped
//! UART: any STR there appends the low byte of the stored value to the output buffer.

use serde::Serialize;
use wasm_bindgen::prelude::*;

const MEM_SIZE: usize = 0x10000;
const UART_OUT: u64 = 0x1000;
const ENTRY_PC: u64 = 0x4000;

#[derive(Serialize, Clone)]
pub struct CpuState {
    pub x: [u64; 31],
    pub sp: u64,
    pub pc: u64,
    pub nzcv: u8,
    pub halted: bool,
    pub last_trap: Option<String>,
    pub steps: u64,
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
        };
        cpu.load_demo();
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
        self.load_demo();
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
        serde_wasm_bindgen::to_value(&CpuState {
            x: self.x,
            sp: self.sp,
            pc: self.pc,
            nzcv: self.nzcv,
            halted: self.halted,
            last_trap: self.last_trap.clone(),
            steps: self.steps,
        })
        .map_err(|e| JsValue::from_str(&e.to_string()))
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
}
