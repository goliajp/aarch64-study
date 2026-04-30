# aarch64-sim

Tiny AArch64 SoC simulator core: two cores with EL0/EL1/EL2, a stage-1 MMU,
an Apple-style interrupt controller, a virtio-blk-shaped block device, and
real LL/SC + IPIs. Written in safe Rust, runs in three places:

- **Library** (`use aarch64_sim::Cpu`) — embed in any Rust program.
- **CLI** (`cargo install aarch64-sim --features cli`) — `aarch64-sim run`,
  `disasm`, `mem`.
- **Browser** (`--features wasm` + `wasm-pack`) — drives the React UI at
  <https://labs.golia.jp/aarch64/> and the minimal demo in
  `examples/wasm-web/`.

## Library quick start

```rust
use aarch64_sim::Cpu;

let mut cpu = Cpu::new();
cpu.set_disk_text("hello, aarch64-sim!\n");
cpu.run(4000);

println!("steps:    {}", cpu.system_steps());
println!("atomic:   {}", cpu.atomic_counter());
println!("UART out: {:?}", cpu.output());

for c in cpu.state() {
    println!("core {} pc={:#x} el={} wfi={}", c.id, c.pc, c.current_el, c.wfi_halted);
}
```

See `examples/run.rs` and `examples/disassemble.rs` for runnable versions.

## CLI

```bash
cargo install aarch64-sim --features cli

aarch64-sim run                                  # boot demo, run 4000 steps, print snapshot
aarch64-sim run --steps 10000 --json             # JSON output for scripts
aarch64-sim run --disk-text "hi" --trace         # per-step PC + disasm trace
aarch64-sim disasm 0xD2800049 0xC85F7CA5         # decode raw 32-bit words
aarch64-sim mem 0x6000 --len 64 --steps 4000     # hex dump after run
```

## Browser / WASM

```bash
# from the crate root
wasm-pack build --target web --out-dir pkg --release -- --features wasm
```

Then either point a Vite project at `pkg/` (see `package.json`'s
`file:./crates/aarch64-sim/pkg` dep in
[goliajp/aarch64-study](https://github.com/goliajp/aarch64-study)) or open
`examples/wasm-web/index.html` over a local HTTP server for a no-bundler
demo.

The JS API mirrors the Rust API exactly — `cpu.step()`, `cpu.state()`,
`cpu.aic_state()`, etc. Numeric fields with u64 width are exposed as
`BigInt` so values are not truncated.

## What it models

- AArch64 user-mode subset: MOVZ, ADD/SUB (imm/reg), LDR/STR/LDP/STP, LDRB,
  B/CBZ/CBNZ, MSR/MRS, SVC, ERET, WFI, NOP/ISB/DMB, DAIFSet/Clr, plus
  LDXR/STXR/CLREX with a real cross-core exclusive monitor.
- EL0 / EL1 / EL2 with the full system-register set (`ELR_EL{1,2}`,
  `SPSR_EL{1,2}`, `ESR_EL{1,2}`, `VBAR_EL{1,2}`, `DAIF`).
- Stage-1 MMU walk (4 KiB granule, configurable T0SZ) with AP-bit
  enforcement so kernel pages reject EL0 access.
- Two cores derived from `MPIDR_EL1`. Per-core context-save areas.
- Apple-style AIC: per-core pending mask, ACK MMIO register, IPI MMIO
  register that lets the kernel wake a peer core.
- virtio-blk-shaped block device (8 × 64-byte sectors at MMIO `0x3000`).
- Live disassembler covering everything the simulator decodes.

## Cargo features

| Feature | Pulls in                          | What it does                               |
| ------- | --------------------------------- | ------------------------------------------ |
| `wasm`  | `wasm-bindgen`, `serde-wasm-bindgen` | Re-exports the API to JS via wasm-bindgen. |
| `cli`   | `clap`, `serde_json`              | Builds the `aarch64-sim` binary.           |

Both are off by default — using the crate as a plain Rust library brings
nothing beyond `serde`.

## Project

This crate is the simulator core for
[**aarch64-study**](https://github.com/goliajp/aarch64-study) — a web-first
study of OS internals targeting Apple Silicon. Each release adds exactly
one architectural concept; reading the diff between two tags is the lesson.
See [CHANGELOG.md](https://github.com/goliajp/aarch64-study/blob/develop/CHANGELOG.md).

## License

MIT — see [LICENSE](LICENSE).
