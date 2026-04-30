# Changelog

Each entry is one architectural concept added on top of the previous one.

## v0.16

UI overhaul + project rebrand.

- Renamed from `osstudy-web` to `aarch64-study`, public on GitHub under MIT.
- Dense pin-out SoC schematic: two cores with REGS / PC / MMU / EXC / DAIF
  sub-blocks, a 4-lane parallel bus (DATA / ADDR / IRQ / CTRL), AIC / UART /
  BLK chips with labelled top-edge pins, 4×3 RAM region grid.
- 12-stat header strip: system steps, retired, timer period, next IRQ,
  timer ticks, AIC acks / pending, UART bytes, BLK reads, per-core PC, EL.
- Roboto Mono on every numeric / hex / register column; Roboto Flex on UI
  text with `tabular-nums`.
- Theme tokens everywhere (`bg-surface`, `bg-bg-secondary`, `bg-bg-tertiary`,
  `border-border`, `text-fg`, `palette-X`); light + dark both render
  cleanly.
- Task A becomes a silent WFI sleeper, task B prints the disk image once
  and parks itself in WFI. Scheduler reduced to "ack + ERET to the same
  task" — output stays monotonic, auto-run pauses once both cores park.
- Default disk content `AArch64 disk image - sector 0\n`.
- 15 / 15 simulator tests pass.

## v0.15

Disassembler covers the whole instruction set the simulator decodes; per-core
monitors with editable disk text.

## v0.14

AP-bit enforcement: kernel pages (AIC, Block) reject EL0 accesses.

## v0.13

WFI: idle cores stop spinning, wake on IRQ.

## v0.12

Per-core scheduler — cores 0/1 run tasks A/B concurrently via MPIDR.

## v0.11

LDRB / CBZ / CBNZ / SUB-imm; task B walks the disk buffer and prints it.

## v0.10

virtio-blk-shaped block device; kernel reads sector 0 at boot.

## v0.9

LDP/STP + real context switch (X0–X3 persist across switches).

## v0.8

AIC abstraction; scheduler swaps tasks A/B every timer tick.

## v0.7

DAIF + AIC timer IRQ + IRQ vector at `VBAR + 0x480`.

## v0.6

Two cores (P-core / E-core) sharing memory, distinguished by `MPIDR_EL1`.

## v0.5

SVC raises EL0 → EL1 + ERET returns (full syscall round-trip).

## v0.4

EL2 boot + ERET drops to EL1.

## v0.3

MSR/MRS + `SCTLR_EL1.M` honoured for fetch / load / store.

## v0.2

MMU stage-1 page-table walk (4 KiB granule).

## v0.1

Registers + 5 instructions (MOVZ ADD LDR STR B) + MMIO UART.
