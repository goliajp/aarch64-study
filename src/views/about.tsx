import { Card } from '@goliapkg/gds'

const ROADMAP = `v0.1   registers + 5 instructions (MOVZ ADD LDR STR B) + MMIO UART
v0.2   + MMU stage-1 page-table walk (4 KiB granule)
v0.3   + MSR/MRS + SCTLR_EL1.M honoured for fetch / load / store
v0.4   + EL2 boot + ERET drops to EL1
v0.5   + SVC raises EL0 → EL1 + ERET returns
v0.6   + two cores (P-core / E-core) sharing memory + MPIDR_EL1
v0.7   + DAIF + AIC timer IRQ + IRQ vector (VBAR + 0x480)
v0.8   + AIC abstraction + scheduler swaps tasks A/B every tick
v0.9   + LDP/STP + real context switch (X0–X3 persist)
v0.10  + virtio-blk-shaped block device (sector 0 read at boot)
v0.11  + LDRB / CBZ / CBNZ / SUB-imm; task B walks disk buffer
v0.12  + per-core scheduler — cores run A/B concurrently via MPIDR
v0.13  + WFI — idle cores stop spinning
v0.14  + AP-bit enforcement — kernel pages reject EL0 access
v0.15  + disassembler, core monitors, editable disk text
v0.16  + UI overhaul: pin-out SoC schematic, theme tokens, scheduler pinning
v0.17  + atomic LDXR / STXR / CLREX + cross-core exclusive monitor
v0.18  + IPI between cores via AIC software-IRQ + SVC-mediated dispatch
v0.19  + crate is publishable: pure-Rust API + CLI + WASM behind features
v0.20  + per-EL stacks (SP_EL0 / SP_EL1), BL / RET, push/pop frames
v0.21  + TLB + ASID + TLBI invalidation  ← here
v0.22  + I-cache + IC IVAU / DC CIVAC + self-modifying code demo
v0.23  + PCB + round-robin scheduler (replaces v0.16 task-pinning hack)
v1.0   + bare-metal port via m1n1 (same Rust crate runs on real Apple Silicon)`

const LAYOUT = `aarch64-study/
├── crates/aarch64-sim/   Rust simulator → wasm-pack → pkg/
├── src/
│   ├── views/cpu.tsx      main interactive view
│   ├── components/        panels, badges, control bar, SoC diagram
│   └── sim/               domain types, formatters, event derivation
├── Cargo.toml
└── package.json`

const COMMANDS = `bun install         install deps
bun run build:sim   compile aarch64-sim crate to WASM
bun run dev         vite dev server (port 32030)
bun run test:sim    cargo test the simulator
bun run check       tsc + eslint + prettier`

export function AboutView() {
  return (
    <div className="space-y-8">
      <div>
        <h1
          className="text-fg text-2xl font-bold"
          style={{ textShadow: '0 0 20px var(--gds-accent, #3b82f6)' }}
        >
          About
        </h1>
        <p className="text-fg-muted type-base mt-1 max-w-2xl">
          A web-first study of operating-system internals targeting Apple Silicon. The kernel logic
          is written in Rust and compiled to WASM so every step is visible in the browser. The same
          code is intended to later target <code>aarch64-unknown-none</code> on real M-series
          hardware.
        </p>
      </div>

      <Section heading="Roadmap" text={ROADMAP} />
      <Section heading="Layout" text={LAYOUT} />
      <Section heading="Commands" text={COMMANDS} />
    </div>
  )
}

function Section({ heading, text }: { heading: string; text: string }) {
  return (
    <div>
      <h2 className="text-fg type-base mb-3 font-semibold">{heading}</h2>
      <Card>
        <pre className="mono-data text-fg-muted type-small overflow-x-auto p-4 leading-relaxed">
          {text}
        </pre>
      </Card>
    </div>
  )
}
