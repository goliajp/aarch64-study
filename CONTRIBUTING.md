# Contributing

Thanks for your interest in `aarch64-study`!

## Development model

- Default branch is `develop`. Feature work happens there; the `main` branch
  is only used to mark released states locally.
- Each release is `release/vX.Y.Z` cut from `develop`, tagged on finish, and
  merged back into `develop`.
- Each release should add **one** focused architectural concept that is
  observable in the browser. If the diff doesn't fit on a postcard, the
  concept probably needs to be split.

## Local setup

```bash
bun install            # web deps + the local aarch64-sim crate
bun run build:sim      # rebuild the Rust simulator into WASM
bun run dev            # vite dev server on http://127.0.0.1:32030
```

Other useful scripts:

```bash
bun run test:sim       # cargo test the Rust simulator
bun run test           # vitest the React side
bun run check          # tsc + eslint + prettier
bun run build          # production bundle into ./dist
```

## Style

- TypeScript on the React side; types over `any`. Run `bun run check` before
  pushing.
- Rust on the simulator side; cover new instructions with a unit test in
  `crates/aarch64-sim/src/lib.rs`.
- UI uses GDS theme tokens (`bg-surface`, `text-fg`, `palette-X`, etc.) — try
  to avoid hand-picked colour shades so light and dark themes both render
  cleanly.
- Numeric / hex / register columns are monospace (Roboto Mono via the
  `mono-data` helper). UI text uses Roboto Flex with `tabular-nums`.
- Animations should depict real simulator events. No ambient sci-fi.

## Commit messages

Follow Conventional Commits when you can:

- `feat(scope): …` — new architectural concept or visible feature
- `fix(scope): …` — bug fix
- `chore(scope): …` — build / tooling / docs
- `refactor(scope): …` — internal cleanup with no behaviour change

## License

By contributing, you agree that your contributions will be licensed under the
MIT license that covers the project.
