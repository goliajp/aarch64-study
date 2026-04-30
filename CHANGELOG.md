# Changelog

Each entry is one architectural concept added on top of the previous one.

## v0.17

LL/SC — atomic primitives via the exclusive monitor.

- Decoded `LDXR Xt, [Xn]` (`0xC85F_7C00 | …`), `STXR Ws, Xt, [Xn]`
  (`0xC800_7C00 | …`), and `CLREX` (`0xD503_3F5F`).
- Per-core `exclusive_monitor: Option<u64>` is set by LDXR to the resolved
  PA, consumed by STXR (succeeds → writes 0 to Ws and commits the store;
  fails → writes 1 and skips the store), and cleared by CLREX, by IRQ
  entry, and — crucially — by any *other* core's store to the same PA.
  The cross-core invalidation runs in `Cpu::step()` after each core ticks,
  walking peers via `last_store_pa`.
- Task A becomes the smallest demonstrable LL/SC client: an
  `ldxr / add / stxr / cbnz` loop that atomically increments a shared u64
  at PA `0x6FF8` (in the same user-mapped 4 KiB page as the disk buffer),
  then `WFI`s until the next IRQ. The counter is exposed as
  `Cpu::atomic_counter()`.
- Five new tests cover the success path, the CLREX-clears-monitor path,
  the IRQ-clears-monitor path, cross-core invalidation, and end-to-end
  monotonic increment under the demo schedule.

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

## v0.15

Disassembler + editable disk + per-core monitors.

- Disassembler in the Rust crate covers every instruction the simulator
  decodes; live listing centred on core 0's PC.
- `cpu.set_disk_text()` makes sector 0 user-editable from the UI; task B
  prints whatever you type.
- Per-core monitor cards (registers, exception state, current EL,
  last_trap, DAIF) split the noisy single panel into two readable columns.

## v0.14

AP-bit enforcement.

- Page-table descriptors carry AP bits (`AP=00` = kernel-only R/W,
  `AP=01` = kernel R/W + EL0 R/W).
- MMIO pages for AIC / Block are now `AP=00`; an EL0 access faults
  through `ESR_EL1.EC = 0x24` (data abort, lower EL).
- Task code (`0x4D00`, `0x4E00`) and the disk buffer (`0x6000`) stay
  `AP=01` so user-mode tasks can still touch them.

## v0.13

WFI — idle cores stop spinning.

- Decoded `WFI` (`0xD503207F`): sets `wfi_halted = true`, advances PC,
  and the step loop stops fetching for that core.
- An unmasked pending IRQ wakes the core; `take_irq` clears
  `wfi_halted` before branching to the IRQ vector.
- Task A becomes a sleep loop (`add x3, x3, 1; wfi; b -2`) so core 0
  spends most of its time genuinely idle.

## v0.12

Per-core scheduler.

- Each core derives its own slot base from `MPIDR_EL1` bit 8 (cluster):
  `0x4F00` for core 0, `0x5000` for core 1.
- Slot layout: `+0x00` current entry · `+0x08` current save pointer ·
  `+0x10` save area 0 (X0..X3) · `+0x30` save area 1.
- Core 0 boots into task A, core 1 into task B, and the IRQ handler
  swaps them in lockstep with the timer. UART output now actually
  interleaves the two cores instead of duplicating output from a single
  core running both tasks.

## v0.11

LDRB / CBZ / CBNZ / SUB-imm; task B walks the disk.

- Three new opcodes (`LDRB`, `CBZ` / `CBNZ`, `SUB` immediate) plus
  `B` to a same-PC target halts the core (used as `b .` for stable
  end states).
- Task B becomes the "disk printer": loads `0x1000` (UART) into X1,
  loops over `0x6000 + X3` with `LDRB`, `STR` to UART, `ADD` X3, until
  `CBZ` on the null terminator.

## v0.10

Block device (virtio-blk shaped).

- New peripheral at MMIO `0x3000`: `SECTOR`, `BUF_ADDR`, `CMD`, `STATUS`
  registers + an in-memory disk image of 8 × 64-byte sectors.
- Kernel boot now does a synchronous `READ` of sector 0 into the
  `0x6000` disk buffer before ERETing into user mode.
- Status register reflects IDLE / OK / FAULT for the panel to read.

## v0.9

LDP/STP + real context switch.

- Decoded `LDP` / `STP` for paired 64-bit register load/store with
  imm7-scaled offset.
- IRQ handler saves `X0..X3` into the current save area with two `STP`s
  before swapping tasks, restores from the other save area with two
  `LDP`s, then `ERET`s.
- Per-task state (X3 as a counter, X0–X2 as printer state) survives
  every context switch.

## v0.8

AIC abstraction + scheduler.

- Pulled the AIC into its own struct (`set_irq`, `has_pending`, `ack`,
  `total_acks` for stats).
- IRQ handler at `VBAR_EL1 + 0x480` becomes a real scheduler: ACKs the
  pending bit, picks "the other task" via the `(TASK_A + TASK_B − current)`
  trick, and ERETs into it.
- The (sum − current) swap avoids a CMP/B.cond pair we don't have yet.

## v0.7

DAIF + AIC timer IRQ + IRQ vector.

- PSTATE.DAIF (D/A/I/F mask bits) honoured for IRQ delivery.
- A primitive AIC raises IRQ when the system timer fires every
  `TIMER_PERIOD` steps; per-core pending mask.
- Setting `VBAR_EL1` and unmasking `I` causes a pending IRQ to branch
  the core to `VBAR_EL1 + 0x480` with `ELR_EL1` and `SPSR_EL1` saved by
  hardware.

## v0.6

Two cores sharing memory.

- `Cpu` holds a `Vec<Core>` with independent X registers, PC, sysregs.
- `MPIDR_EL1` distinguishes them: `0x80000000` (P-core) vs
  `0x80000100` (E-core, bit 8 set for cluster 1).
- `step()` iterates both cores; both share RAM and the same MMU page
  tables.

## v0.5

SVC raises EL0 → EL1 + ERET returns.

- `SVC #imm` triggers a synchronous exception (`ESR_EL1.EC = 0x15`) and
  branches to `VBAR_EL1 + 0x400`.
- Hardware saves `ELR_EL1` (return address) and `SPSR_EL1` (the EL0
  PSTATE).
- `ERET` reads them back, restores PSTATE, and resumes EL0 — a full
  syscall round-trip with no other instructions involved.

## v0.4

EL2 boot, ERET drops to EL1.

- `current_el` field on `Core`; sysregs split into `*_EL1` and `*_EL2`
  pairs (`ELR`, `SPSR`, `ESR`, `VBAR`).
- Boot starts at EL2 with `ELR_EL2` / `SPSR_EL2` configured, and `ERET`
  transitions to EL1 with the configured DAIF / SP mode.

## v0.3

MSR/MRS + `SCTLR_EL1.M` honoured.

- MSR / MRS decoded for the system registers introduced in v0.2
  (`TTBR0_EL1`, `TCR_EL1`, `SCTLR_EL1`).
- When `SCTLR_EL1.M = 1`, every instruction fetch and every LDR / STR
  goes through the stage-1 walk; clearing M reverts to physical
  addressing for diagnostics.

## v0.2

MMU stage-1 page-table walk.

- `translate(va, core)` walks L1 → L2 → L3 with a 4 KiB granule and a
  39-bit VA driven by `T0SZ`.
- Returns a `TranslationResult` with every step (table address, index,
  raw descriptor, decoded outcome) so the UI can show the walk.
- Pure query — no fetch path through it yet (that's v0.3).

## v0.1

The starting point.

- `Cpu` with 31 X registers, PC, halted flag, and an output buffer.
- Five hand-decoded instructions: `MOVZ`, `ADD` (imm + reg), `LDR`,
  `STR`, `B`.
- A single MMIO UART at PA `0x1000` — any STR there appends a byte to
  the output buffer.
