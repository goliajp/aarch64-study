import { Card } from '@goliapkg/gds'

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
        <p className="text-fg-muted mt-1 max-w-2xl text-sm">
          A web-first study of operating-system internals targeting Apple Silicon. The kernel logic
          is written in Rust and compiled to WASM so every step is visible in the browser. The same
          code is intended to later target <code>aarch64-unknown-none</code> on real M-series
          hardware.
        </p>
      </div>

      <div>
        <h2 className="text-fg mb-3 text-sm font-semibold">Roadmap</h2>
        <Card>
          <pre className="text-fg-muted overflow-x-auto p-4 font-mono text-xs leading-relaxed">
            {`v0.1  registers + 5 instructions (MOVZ ADD LDR STR B) + MMIO UART
v0.2  + MMU stage-1 page-table walk (4 KiB granule, identity-map demo)
v0.3  + MSR/MRS + SCTLR_EL1.M honoured (LDR/STR/fetch all through MMU)
v0.4  + EL2 boot + ERET drops to EL1 (current_el visible, ELR/SPSR sysregs)
v0.5  + SVC raises EL0 → EL1 + ERET returns (full syscall round-trip)
v0.6  + two cores (P-core / E-core) sharing memory + MPIDR_EL1
v0.7  + DAIF + AIC timer IRQ + IRQ vector (VBAR+0x480)
v0.8  + AIC abstraction + scheduler swaps tasks A/B on every tick
v0.9  + LDP/STP + real context switch (X0–X3 persist across switches)
v0.10 + Block device (virtio-blk-shaped) — kernel reads sector 0 at boot  ← here
v1.0  + bare-metal port via m1n1 (same Rust crate runs on real Apple Silicon)`}
          </pre>
        </Card>
      </div>

      <div>
        <h2 className="text-fg mb-3 text-sm font-semibold">Layout</h2>
        <Card>
          <pre className="text-fg-muted overflow-x-auto p-4 font-mono text-xs leading-relaxed">
            {`osstudy-web/
├── crates/
│   └── aarch64-sim/        # Rust simulator → wasm-pack → pkg/
│       ├── src/lib.rs
│       └── pkg/             # generated, imported by web
├── src/
│   ├── views/cpu.tsx        # registers / memory / output panels
│   ├── views/about.tsx
│   ├── app.tsx
│   └── main.tsx
├── Cargo.toml               # workspace
└── package.json`}
          </pre>
        </Card>
      </div>

      <div>
        <h2 className="text-fg mb-3 text-sm font-semibold">Commands</h2>
        <Card>
          <pre className="text-fg-muted overflow-x-auto p-4 font-mono text-xs leading-relaxed">
            {`bun install         # install deps
bun run build:sim   # rebuild aarch64-sim wasm
bun run test:sim    # cargo test the simulator
bun run dev         # vite dev server (port 32030)
bun run check       # typecheck + lint + format check`}
          </pre>
        </Card>
      </div>
    </div>
  )
}
