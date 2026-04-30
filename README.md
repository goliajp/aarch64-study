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

[English](#english) · [中文](#中文) · [日本語](#日本語)

---

## English

### What it is

Two M-series-flavoured cores (a "P-core" and an "E-core"), a 4-lane bus
(DATA / ADDR / IRQ / CTRL), an Apple-style interrupt controller, a tiny
virtio-blk-shaped block device, and 64 KiB of RAM — running entirely in your
browser. Each released version (`v0.1` … `v0.16`) introduces exactly one
architectural concept on top of the previous one, so you can read the diff and
internalise the change.

### What it currently models

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

### Try it locally

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

### Roadmap

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

### Repository layout

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
│   ├── app.tsx              # router + layout
│   └── main.tsx
├── Cargo.toml               # workspace
├── package.json
└── README.md
```

### Contributing

Issues and pull requests are welcome. The development model is git-flow on
`develop`, with releases cut as `release/vX.Y.Z` branches. Each release is one
focused architectural concept, so new contributions should keep the same
shape: one concept, observable in the browser.

### License

MIT — see [LICENSE](LICENSE).

---

## 中文

### 项目简介

`aarch64-study` 是一个跑在浏览器里的 **AArch64 SoC 模拟器**，目的是把操作系统底层
一砖一瓦地讲清楚——每个版本只引入一个架构概念。模拟器核心用 Rust 写，编译成
WebAssembly 在浏览器中运行：你能直接看到每条指令的取指、每次页表查询、每次异常
进入是怎么发生的。

> 在线演示 · <https://labs.golia.jp/aarch64/>

### 当前模拟了什么

- 两核（P-core / E-core，模仿 Apple Silicon 的形态），通过 `MPIDR_EL1` 区分
- 4 车道总线（DATA / ADDR / IRQ / CTRL）+ Apple 风格中断控制器
- AArch64 用户态指令子集（MOVZ、ADD/SUB、LDR/STR/LDP/STP、LDRB、B、CBZ/CBNZ、
  MSR/MRS、SVC、ERET、WFI、NOP/ISB/DMB、DAIFSet/Clr）
- EL0 / EL1 / EL2 三级特权 + 完整的 `ELR / SPSR / ESR / VBAR / DAIF` 系统寄存器
- 第一阶段 MMU 翻译（4 KiB 粒度，可配 T0SZ），带 AP 位保护——kernel 页 EL0 访问会触发 fault
- 类 virtio-blk 的块设备（8 × 64 字节扇区，MMIO 挂在 `0x3000`）
- 实时反汇编器，能解码模拟器支持的全部指令

### 本地跑起来

```bash
bun install            # 装 Web 依赖 + 本地 aarch64-sim crate
bun run build:sim      # 重新构建 Rust 模拟器到 WASM
bun run dev            # vite dev server，端口 32030
```

其他常用脚本：

```bash
bun run test:sim       # 跑 Rust 模拟器测试
bun run test           # 跑 React 侧 vitest
bun run check          # tsc + eslint + prettier
bun run build          # 产线构建到 ./dist
```

### 版本路线图

每个版本只增加一个新概念，看 commit diff 就能学到对应的体系结构知识点。详细列表见
英文部分的 Roadmap 表格，从 v0.1 到 v0.16，最终目标 v1.0 是把同一份 Rust crate 通过
m1n1 运行到真正的 Apple Silicon 上。

### 项目结构

见英文 `Repository layout` 一节。

### 贡献

欢迎 Issue / PR。开发模型是 git-flow，主分支 `develop`，每个 release 切 `release/vX.Y.Z`。
每个 release 集中讲一个架构概念，新 PR 请保持这个节奏。

### 许可

MIT，见 [LICENSE](LICENSE)。

---

## 日本語

### 概要

`aarch64-study` は **AArch64 SoC のシミュレータ**で、すべての処理がブラウザ内で
動きます。Rust で書かれたシミュレータコアを WebAssembly にコンパイルし、命令の
フェッチ、ページテーブルウォーク、例外エントリといったハードウェアの内部動作を
ブラウザから直接観察できます。各バージョン（`v0.1` 〜 `v0.16`）は、前のバージョンに
**ひとつだけ**アーキテクチャ概念を追加していくスタイルなので、差分を読むだけで
学習が進みます。

> ライブデモ · <https://labs.golia.jp/aarch64/>

### 現在モデル化しているもの

- 2 つのコア（P-core / E-core、Apple Silicon を模した）— `MPIDR_EL1` で区別
- 4 レーンのバス（DATA / ADDR / IRQ / CTRL）+ Apple 風割り込みコントローラ
- AArch64 ユーザモードのサブセット命令（MOVZ・ADD/SUB・LDR/STR/LDP/STP・LDRB・
  B・CBZ/CBNZ・MSR/MRS・SVC・ERET・WFI・NOP/ISB/DMB・DAIFSet/Clr）
- EL0 / EL1 / EL2 の完全なシステムレジスタ群
  （`ELR / SPSR / ESR / VBAR / DAIF`）
- ステージ 1 MMU ウォーク（4 KiB グラニュール、可変 T0SZ）— AP ビットによる保護で
  カーネルページへの EL0 アクセスは fault になる
- virtio-blk 風のブロックデバイス（8 × 64 バイトセクタ、MMIO は `0x3000`）
- シミュレータが扱う命令を全てカバーするリアルタイム逆アセンブラ

### ローカルで動かす

```bash
bun install            # 依存と aarch64-sim crate をインストール
bun run build:sim      # Rust シミュレータを WASM へリビルド
bun run dev            # vite dev server（http://127.0.0.1:32030）
```

その他のスクリプト：

```bash
bun run test:sim       # Rust シミュレータの cargo test
bun run test           # React 側の vitest
bun run check          # tsc + eslint + prettier
bun run build          # 本番ビルドを ./dist に出力
```

### ロードマップ

各リリースで導入される概念は英語セクションの表をご覧ください。`v0.1` 〜 `v0.16` まで
段階的に進み、最終的な `v1.0` は同じ Rust クレートを m1n1 経由で実機の Apple
Silicon 上で動かすことを目標にしています。

### リポジトリ構成

英語版の `Repository layout` を参照してください。

### コントリビュート

Issue / PR を歓迎します。開発は git-flow（メインは `develop`、リリースは
`release/vX.Y.Z`）で行い、各リリースは「ひとつの概念をブラウザから見える形で示す」
というスタイルを保ちます。

### ライセンス

MIT — [LICENSE](LICENSE) を参照してください。
