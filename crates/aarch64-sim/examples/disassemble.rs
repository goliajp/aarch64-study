//! Decode a sequence of 32-bit AArch64 instructions.
//! Run: `cargo run --example disassemble -p aarch64-sim`.

use aarch64_sim::disassemble;

fn main() {
    let base: u64 = 0x4000;
    let words: &[u32] = &[
        0xD2800049, // movz x9, #0x2
        0x91000400, // add  x0, x0, #0x1
        0xC85F7CA5, // ldxr x5, [x5]
        0xC8067CA5, // stxr w6, x5, [x5]
        0xD503305F, // clrex
        0xD400_0001, // svc #0
        0xD69F03E0, // eret
        0xD503207F, // wfi
    ];

    for (i, &insn) in words.iter().enumerate() {
        let pc = base + (i as u64) * 4;
        println!("{:#010x}  {:08x}  {}", pc, insn, disassemble(insn, pc));
    }
}
