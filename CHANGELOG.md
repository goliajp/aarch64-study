# Changelog

All notable changes are recorded here. The shape of each release is "one
architectural concept on top of the previous one, observable in the browser."

## v0.16.0 (current)

- Rebrand: project renamed from `osstudy-web` to `aarch64-study`,
  open-sourced under the MIT license.
- UI overhaul:
  - dense pin-out SoC schematic with two cores, a 4-lane bus
    (DATA / ADDR / IRQ / CTRL), AIC / UART / BLK chips with labelled pins,
    and a 4×3 RAM region grid;
  - 12-stat header strip (system steps, retired, timer period, next IRQ,
    timer ticks, AIC acks, AIC pending, UART bytes, BLK reads, per-core PC,
    per-core EL);
  - all data columns rendered in Roboto Mono so hex letters align across
    rows; UI text stays Roboto Flex with tabular numerals;
  - GDS theme tokens everywhere (`bg-surface`, `bg-bg-secondary`,
    `bg-bg-tertiary`, `border-border`, `text-fg / text-fg-muted`,
    `palette-X`) so light and dark themes both render cleanly.
- Simulator:
  - task A becomes a silent WFI sleeper; task B prints the disk image
    once and parks itself in WFI (the previous loop was confusing and
    interleaved badly with task A's output);
  - scheduler reduced to "ack the AIC and ERET to the same task" — tasks
    are pinned per core, so the UART output stays monotonic and the
    auto-run loop stops once both cores park;
  - default disk content reads `AArch64 disk image - sector 0\n`;
  - 15 / 15 simulator tests pass.

## v0.15.0

- Disassembler covers the entire instruction set the simulator decodes.
- Per-core monitors with editable disk text.

## v0.14.0

- AP-bit enforcement: kernel pages (AIC, Block) reject EL0 accesses.

## v0.13.0

- WFI: idle cores stop spinning, wake on IRQ.

## v0.12.0

- Per-core scheduler — cores 0/1 run tasks A/B concurrently via MPIDR.

## v0.11.0

- LDRB / CBZ / CBNZ / SUB-imm; task B walks the disk buffer and prints it.

## v0.10.0

- virtio-blk-shaped block device; kernel reads sector 0 at boot.

## v0.9.0

- LDP/STP + real context switch (X0–X3 persist across switches).

## v0.8.0

- AIC abstraction; scheduler swaps tasks A/B on every timer tick.

## v0.7.0

- DAIF + AIC timer IRQ + IRQ vector at `VBAR + 0x480`.

## v0.6.0

- Two cores (P-core / E-core) sharing memory, distinguished by `MPIDR_EL1`.

## v0.5.0

- SVC raises EL0 → EL1 + ERET returns (full syscall round-trip).

## v0.4.0

- EL2 boot + ERET drops to EL1.

## v0.3.0

- MSR/MRS + `SCTLR_EL1.M` honoured for fetch/load/store.

## v0.2.0

- MMU stage-1 page-table walk (4 KiB granule).

## v0.1.0

- Registers + 5 instructions (MOVZ ADD LDR STR B) + MMIO UART.
