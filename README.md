# aarch64-study

A web-first **AArch64 SoC simulator** built to teach operating-system internals
one architectural concept at a time. The simulator core is written in Rust
and compiled to WebAssembly — every fetch, every page-table walk, every
exception is observable in the browser.

> Live demo · <https://labs.golia.jp/aarch64/>

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-WASM-orange)](https://www.rust-lang.org/)
[![React](https://img.shields.io/badge/React-19-61dafb)](https://react.dev/)
[![TypeScript](https://img.shields.io/badge/TypeScript-6-3178c6)](https://www.typescriptlang.org/)

**Other languages** · [中文](README.zh.md) · [日本語](README.ja.md)

## What it is

Two M-series-flavoured cores (a "P-core" and an "E-core"), a 4-lane bus
(DATA / ADDR / IRQ / CTRL), an Apple-style interrupt controller, a tiny
virtio-blk-shaped block device, and 64 KiB of RAM — running entirely in your
browser. Each released version (`v0.1` … `v0.16`) introduces exactly one
architectural concept on top of the previous one, so you can read the diff and
internalise the change.

## What it currently models

- AArch64 user-mode subset (MOVZ, ADD/SUB imm/reg, LDR/STR/LDP/STP, LDRB,
  B/CBZ/CBNZ, MSR/MRS, SVC, ERET, WFI, NOP/ISB/DMB, DAIFSet/Clr)
- EL0 / EL1 / EL2 with `ELR_EL{1,2}`, `SPSR_EL{1,2}`, `ESR_EL{1,2}`,
  `VBAR_EL{1,2}`, `DAIF`
- Stage-1 MMU walk (4 KiB granule, 39-bit VA, configurable T0SZ) with
  AP-bit enforcement so kernel pages reject EL0 access
- Two cores derived from `MPIDR_EL1` with a real per-core context-save area
- Apple-style interrupt controller (broadcast timer + per-core pending mask)
- virtio-blk-shaped block device (8 × 64-byte sectors, MMIO at `0x3000`)
- Live disassembler for everything the simulator can decode

## Try it locally

```bash
bun install            # install web deps + the local aarch64-sim crate
bun run build:sim      # rebuild the Rust simulator into WASM
bun run dev            # vite dev server on http://127.0.0.1:32030
```

Other scripts:

```bash
bun run test:sim       # cargo test the Rust simulator
bun run test           # vitest the React side
bun run check          # tsc + eslint + prettier
bun run build          # production bundle into ./dist
```

## Roadmap

| Version | What was added                                                                |
| ------- | ----------------------------------------------------------------------------- |
| v0.1    | Registers + 5 instructions (MOVZ ADD LDR STR B) + MMIO UART                   |
| v0.2    | MMU stage-1 page-table walk (4 KiB granule)                                   |
| v0.3    | MSR/MRS + `SCTLR_EL1.M` honoured for fetch/load/store                         |
| v0.4    | EL2 boot + ERET drops to EL1                                                  |
| v0.5    | SVC raises EL0 → EL1 + ERET returns                                           |
| v0.6    | Two cores (P-core / E-core) sharing memory + `MPIDR_EL1`                      |
| v0.7    | DAIF + AIC timer IRQ + IRQ vector at `VBAR + 0x480`                           |
| v0.8    | AIC abstraction + scheduler swaps tasks A/B every tick                        |
| v0.9    | LDP/STP + real context switch (X0–X3 persist across switches)                 |
| v0.10   | Block device (virtio-blk-shaped) — kernel reads sector 0 at boot              |
| v0.11   | LDRB / CBZ / CBNZ / SUB-imm; task B walks disk buffer and prints it           |
| v0.12   | Per-core scheduler — cores 0/1 run A/B concurrently via MPIDR                 |
| v0.13   | WFI — task A sleeps until the next IRQ, idle cores stop spinning              |
| v0.14   | AP-bit enforcement — kernel pages (AIC/Block) reject EL0 access               |
| v0.15   | Disassembler, core monitors, editable disk text                               |
| v0.16   | UI overhaul: pin-out SoC schematic, theme tokens, monospace data, scheduler pinning |
| v1.0    | Bare-metal port via m1n1 — same Rust crate runs on real Apple Silicon         |

## Repository layout

```
aarch64-study/
├── crates/
│   └── aarch64-sim/        # Rust simulator → wasm-pack → pkg/
│       ├── src/lib.rs
│       └── pkg/             # generated, imported by web
├── src/
│   ├── views/cpu.tsx        # the main interactive panel
│   ├── views/about.tsx
│   ├── components/          # reusable UI pieces
│   ├── sim/                 # domain types + format helpers
│   ├── app.tsx              # router + layout
│   └── main.tsx
├── Cargo.toml               # workspace
├── package.json
└── README.md
```

## Contributing

Issues and pull requests are welcome. The development model is git-flow on
`develop`, with releases cut as `release/vX.Y.Z` branches. Each release is one
focused architectural concept, so new contributions should keep the same
shape: one concept, observable in the browser. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT — see [LICENSE](LICENSE).
