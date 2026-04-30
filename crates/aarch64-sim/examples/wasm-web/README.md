# Minimal WASM demo

A single-page demo that loads the simulator into the browser without React /
Vite / a bundler.

## Build

```bash
# from the crate root (crates/aarch64-sim/)
wasm-pack build --target web --out-dir pkg --release -- --features wasm
```

## Serve

```bash
# from the crate root
python3 -m http.server 8080
# open http://localhost:8080/examples/wasm-web/
```

The page imports `../../pkg/aarch64_sim.js` directly. If you copy this
example elsewhere, fix that path to point at the generated `pkg/`.

## What it shows

- `new Cpu()` — boots the demo (kernel + two tasks + page tables).
- `cpu.step()`, `cpu.run(N)`, `cpu.reset()` — drive the simulator.
- `cpu.state()`, `cpu.aic_state()`, `cpu.block_state()`,
  `cpu.atomic_counter()` — inspect snapshot data; numeric fields come back
  as `BigInt` (preserving u64 fidelity).

For a full UI (per-core register monitors, MMU walker, SoC schematic,
disassembly, editable disk), see <https://github.com/goliajp/aarch64-study>.
