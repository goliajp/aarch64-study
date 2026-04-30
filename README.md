# aarch64-study

A web-first **AArch64 SoC simulator** that teaches operating-system
internals one concept at a time. The simulator core is written in Rust
and compiled to WebAssembly — every fetch, every page-table walk, every
exception is observable in the browser.

> Live demo · <https://labs.golia.jp/aarch64/>

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**Other languages** · [中文](README.zh.md) · [日本語](README.ja.md)

## What it models

- AArch64 user-mode subset (MOVZ, ADD/SUB imm/reg, LDR/STR/LDP/STP, LDRB,
  B/CBZ/CBNZ, MSR/MRS, SVC, ERET, WFI, NOP/ISB/DMB, DAIFSet/Clr).
- EL0 / EL1 / EL2 with the full system-register set (`ELR_EL{1,2}`,
  `SPSR_EL{1,2}`, `ESR_EL{1,2}`, `VBAR_EL{1,2}`, `DAIF`).
- Stage-1 MMU walk (4 KiB granule, configurable T0SZ) with AP-bit
  enforcement, so kernel pages reject EL0 access.
- Two cores derived from `MPIDR_EL1` with a real per-core context-save
  area.
- Apple-style interrupt controller (broadcast timer + per-core pending
  mask).
- virtio-blk-shaped block device (8 × 64-byte sectors, MMIO at `0x3000`).
- Live disassembler for everything the simulator can decode.

## Quick start

```bash
bun install            # web deps + the local aarch64-sim crate
bun run build:sim      # compile the Rust simulator to WASM
bun run dev            # vite dev server on http://127.0.0.1:32030
```

| Script               | What it runs                                    |
| -------------------- | ----------------------------------------------- |
| `bun run test:sim`   | `cargo test` the Rust simulator (24 tests)      |
| `cargo run --features cli --bin aarch64-sim -- run` | run the simulator from the command line |
| `bun run test`       | Vitest the React side                           |
| `bun run check`      | tsc + eslint + prettier                         |
| `bun run build`      | production bundle into `./dist`                 |
| `bun run deploy`     | build + rsync `dist/` to the production target  |

## Roadmap

| Version | Concept                                                                    |
| ------- | -------------------------------------------------------------------------- |
| v0.1    | Registers + 5 instructions + MMIO UART                                     |
| v0.2    | MMU stage-1 page-table walk                                                |
| v0.3    | MSR/MRS + `SCTLR_EL1.M` honoured                                           |
| v0.4    | EL2 boot, ERET drops to EL1                                                |
| v0.5    | SVC raises EL0 → EL1, ERET returns                                         |
| v0.6    | Two cores sharing memory + `MPIDR_EL1`                                     |
| v0.7    | DAIF, AIC timer IRQ, IRQ vector at `VBAR + 0x480`                          |
| v0.8    | AIC abstraction, scheduler swaps tasks                                     |
| v0.9    | LDP/STP + real context switch                                              |
| v0.10   | virtio-blk-shaped block device                                             |
| v0.11   | LDRB / CBZ / CBNZ / SUB-imm — task B walks the disk                        |
| v0.12   | Per-core scheduler (cores run A/B via MPIDR)                               |
| v0.13   | WFI — idle cores stop spinning                                             |
| v0.14   | AP-bit enforcement                                                         |
| v0.15   | Disassembler + editable disk                                               |
| v0.16   | UI overhaul: pin-out SoC schematic, theme tokens, scheduler pinning        |
| v0.17   | LDXR / STXR / CLREX + cross-core exclusive monitor (real concurrency)      |
| v0.18   | IPI between cores via AIC software-IRQ — SVC-mediated dispatch             |
| v0.19   | Crate becomes publishable: pure-Rust API + `cli` and `wasm` features       |
| v1.0    | Bare-metal port via m1n1 — same Rust crate runs on real Apple Silicon      |

## Layout

```
aarch64-study/
├── crates/aarch64-sim/   Rust simulator → wasm-pack → pkg/
├── src/
│   ├── views/cpu.tsx      main interactive view
│   ├── components/        panels, badges, control bar, SoC diagram
│   └── sim/               domain types, formatters, event derivation
├── Cargo.toml
└── package.json
```

## Contributing

Issues and PRs welcome. Branching is git-flow on `develop`; releases are
`release/vX.Y.Z`. Each release is one focused concept observable in the
browser.

## License

MIT — see [LICENSE](LICENSE).
