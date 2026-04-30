//! `aarch64-sim` — command-line driver for the simulator core.
//!
//! Three subcommands:
//!   run    — boot the demo, run for N steps, print a snapshot
//!   disasm — decode raw 32-bit AArch64 instruction words
//!   mem    — boot, run for N steps, hex-dump a physical-memory range
//!
//! Build with `cargo build --features cli` (or `cargo run --features cli -- ...`).

use std::process::ExitCode;

use aarch64_sim::{disassemble, AicState, BlockState, CoreState, Cpu, TranslationResult};
use clap::{Parser, Subcommand};
use serde::Serialize;

#[derive(Parser)]
#[command(
    name = "aarch64-sim",
    version,
    about = "AArch64 SoC simulator — runs the demo OS, prints state",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Boot the demo and run for N steps; print a snapshot at the end.
    Run {
        /// Number of `Cpu::step()` calls to issue.
        #[arg(short = 'n', long, default_value_t = 4000)]
        steps: u32,
        /// Replace disk sector 0 with this UTF-8 text before booting.
        #[arg(long)]
        disk_text: Option<String>,
        /// Print every step (PC + disassembly per core). Verbose.
        #[arg(long)]
        trace: bool,
        /// Emit the snapshot as JSON instead of human text.
        #[arg(long)]
        json: bool,
    },

    /// Disassemble raw 32-bit instruction words.
    Disasm {
        /// One or more 32-bit instructions in hex (`0xD2800049` or `D2800049`).
        #[arg(required = true)]
        words: Vec<String>,
        /// Starting PC for branch-target rendering.
        #[arg(long, default_value_t = 0x4000)]
        base: u64,
    },

    /// Boot, run for N steps, then hex-dump a physical-memory range.
    Mem {
        /// Starting PA in hex (`0x6000`) or decimal.
        addr: String,
        /// Number of bytes to dump (default 64).
        #[arg(short = 'l', long, default_value_t = 64)]
        len: u32,
        /// Steps to run before dumping.
        #[arg(short = 'n', long, default_value_t = 4000)]
        steps: u32,
        /// Replace disk sector 0 with this text before booting.
        #[arg(long)]
        disk_text: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run { steps, disk_text, trace, json } => cmd_run(steps, disk_text, trace, json),
        Cmd::Disasm { words, base } => cmd_disasm(words, base),
        Cmd::Mem { addr, len, steps, disk_text } => cmd_mem(addr, len, steps, disk_text),
    }
}

fn cmd_run(steps: u32, disk_text: Option<String>, trace: bool, json: bool) -> ExitCode {
    let mut cpu = Cpu::new();
    if let Some(t) = disk_text.as_deref() {
        cpu.set_disk_text(t);
    }
    if trace {
        run_traced(&mut cpu, steps);
    } else {
        cpu.run(steps);
    }
    if json {
        emit_json(&cpu);
    } else {
        emit_text(&cpu);
    }
    ExitCode::SUCCESS
}

fn cmd_disasm(words: Vec<String>, base: u64) -> ExitCode {
    for (i, w) in words.iter().enumerate() {
        let pc = base + (i as u64) * 4;
        match parse_u32(w) {
            Some(insn) => {
                let mnem = disassemble(insn, pc);
                println!("{:#010x}  {:08x}  {}", pc, insn, mnem);
            }
            None => {
                eprintln!("error: not a valid 32-bit hex word: {w}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

fn cmd_mem(addr: String, len: u32, steps: u32, disk_text: Option<String>) -> ExitCode {
    let Some(start) = parse_u64(&addr) else {
        eprintln!("error: not a valid address: {addr}");
        return ExitCode::FAILURE;
    };
    let mut cpu = Cpu::new();
    if let Some(t) = disk_text.as_deref() {
        cpu.set_disk_text(t);
    }
    cpu.run(steps);
    let bytes = cpu.mem_slice(start as u32, len);
    hex_dump(start, &bytes);
    ExitCode::SUCCESS
}

fn run_traced(cpu: &mut Cpu, steps: u32) {
    for _ in 0..steps {
        let states = cpu.state();
        for c in &states {
            let pc = c.pc;
            let bytes = cpu.mem_slice(pc as u32, 4);
            if bytes.len() == 4 {
                let insn = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                let mnem = disassemble(insn, pc);
                let flag = if c.wfi_halted {
                    "wfi"
                } else if c.halted {
                    "halt"
                } else {
                    "run"
                };
                println!(
                    "step {:>5}  c{}  EL{}  {:<4}  pc={:#010x}  {}",
                    cpu.system_steps(),
                    c.id,
                    c.current_el,
                    flag,
                    pc,
                    mnem
                );
            }
        }
        if !cpu.step() {
            break;
        }
    }
}

fn emit_text(cpu: &Cpu) {
    let cores: Vec<CoreState> = cpu.state();
    let aic: AicState = cpu.aic_state();
    let block: BlockState = cpu.block_state();
    let out = cpu.output();
    println!("== aarch64-sim ==");
    println!(
        "steps:       {} (timer ticks: {}, AIC acks: {}, IPIs sent: {})",
        cpu.system_steps(),
        cpu.timer_ticks(),
        aic.total_acks,
        aic.total_ipis,
    );
    println!("atomic ctr:  {}", cpu.atomic_counter());
    println!("blk reads:   {}", block.total_reads);
    println!(
        "UART output: {:?} ({} bytes)",
        out,
        out.as_bytes().len()
    );
    println!();
    for c in &cores {
        println!(
            "core {}  {:<6}  pc={:#010x}  el={}  halted={:<5}  wfi={:<5}  trap={}",
            c.id,
            c.kind,
            c.pc,
            c.current_el,
            c.halted,
            c.wfi_halted,
            c.last_trap.as_deref().unwrap_or("-"),
        );
    }
    println!();
    if let Some(target) = aic.last_ipi_target {
        println!("last IPI target: core {target}");
    }
    // First 16 bytes of the disk buffer page so it's obvious the kernel
    // really copied sector 0 in.
    let buf = cpu.mem_slice(0x6000, 16);
    print!("disk[0x6000]:");
    for b in &buf {
        print!(" {:02x}", b);
    }
    println!();
}

#[derive(Serialize)]
struct Snapshot<'a> {
    system_steps: u64,
    timer_ticks: u64,
    timer_period: u64,
    timer_remaining: u64,
    atomic_counter: u64,
    output: &'a str,
    cores: &'a [CoreState],
    aic: &'a AicState,
    block: &'a BlockState,
    translate_entry_pc: Option<TranslationResult>,
}

fn emit_json(cpu: &Cpu) {
    let cores = cpu.state();
    let aic = cpu.aic_state();
    let block = cpu.block_state();
    let out = cpu.output();
    let snap = Snapshot {
        system_steps: cpu.system_steps(),
        timer_ticks: cpu.timer_ticks(),
        timer_period: cpu.timer_period(),
        timer_remaining: cpu.timer_remaining(),
        atomic_counter: cpu.atomic_counter(),
        output: out.as_str(),
        cores: &cores,
        aic: &aic,
        block: &block,
        translate_entry_pc: cpu.translate(cpu.entry_pc(), 0),
    };
    match serde_json::to_string_pretty(&snap) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("error: {e}"),
    }
}

fn hex_dump(base: u64, bytes: &[u8]) {
    for (row, chunk) in bytes.chunks(16).enumerate() {
        let addr = base + (row as u64) * 16;
        print!("{:#010x} ", addr);
        for b in chunk {
            print!(" {:02x}", b);
        }
        // pad
        for _ in chunk.len()..16 {
            print!("   ");
        }
        print!("  |");
        for b in chunk {
            let ch = if (0x20..0x7F).contains(b) { *b as char } else { '.' };
            print!("{ch}");
        }
        println!("|");
    }
}

fn parse_u32(s: &str) -> Option<u32> {
    let t = s.trim();
    let stripped = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u32::from_str_radix(stripped, 16).ok()
}

fn parse_u64(s: &str) -> Option<u64> {
    let t = s.trim();
    if let Some(stripped) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u64::from_str_radix(stripped, 16).ok()
    } else {
        t.parse::<u64>().ok()
    }
}
