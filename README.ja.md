# aarch64-study

ブラウザで動く **AArch64 SoC のシミュレータ**。各バージョンが「ひとつだけ」アーキテクチャ
概念を追加するスタイルで、OS 内部の挙動を一段ずつ理解していけます。シミュレータコアは
Rust で書かれ WebAssembly にコンパイルされており、命令フェッチ・ページテーブルウォーク・
例外エントリといったすべての挙動がブラウザから直接観察できます。

> ライブデモ · <https://labs.golia.jp/aarch64/>

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**他の言語** · [English](README.md) · [中文](README.zh.md)

## モデル化されているもの

- AArch64 ユーザモードのサブセット命令（MOVZ・ADD/SUB・LDR/STR/LDP/STP・LDRB・
  B・CBZ/CBNZ・MSR/MRS・SVC・ERET・WFI・NOP/ISB/DMB・DAIFSet/Clr）。
- EL0 / EL1 / EL2 と全システムレジスタ群（`ELR / SPSR / ESR / VBAR / DAIF`）。
- ステージ 1 MMU ウォーク（4 KiB グラニュール、可変 T0SZ）— AP ビット保護で
  カーネルページへの EL0 アクセスは fault。
- 2 つのコア（`MPIDR_EL1` で区別）、独立したコンテキスト保存領域つき。
- Apple 風割り込みコントローラ（ブロードキャストタイマ + コア毎 pending mask）。
- virtio-blk 風ブロックデバイス（8 × 64 バイトセクタ、MMIO は `0x3000`）。
- シミュレータが扱う命令を全てカバーする逆アセンブラ。

## クイックスタート

```bash
bun install            # 依存と aarch64-sim crate
bun run build:sim      # Rust シミュレータを WASM へコンパイル
bun run dev            # vite dev server（http://127.0.0.1:32030）
```

| スクリプト             | 内容                                            |
| --------------------- | ----------------------------------------------- |
| `bun run test:sim`    | `cargo test`（20 件）                           |
| `bun run test`        | React 側の vitest                               |
| `bun run check`       | tsc + eslint + prettier                         |
| `bun run build`       | `./dist` への本番ビルド                         |
| `bun run deploy`      | ビルドして `dist/` を本番にデプロイ              |

## ロードマップ

| バージョン | 追加内容                                                                         |
| ---------- | -------------------------------------------------------------------------------- |
| v0.1       | レジスタ + 5 命令 + MMIO UART                                                    |
| v0.2       | MMU ステージ 1 ページテーブルウォーク                                              |
| v0.3       | MSR/MRS + `SCTLR_EL1.M` を尊重                                                    |
| v0.4       | EL2 ブート + ERET で EL1 へ                                                       |
| v0.5       | SVC で EL0 → EL1、ERET で戻る                                                     |
| v0.6       | 2 コア + `MPIDR_EL1`                                                             |
| v0.7       | DAIF + AIC タイマ IRQ + IRQ ベクタ `VBAR + 0x480`                                 |
| v0.8       | AIC 抽象化 + スケジューラがタスク A/B を切替                                       |
| v0.9       | LDP/STP + 本物のコンテキストスイッチ                                               |
| v0.10      | virtio-blk 風ブロックデバイス                                                     |
| v0.11      | LDRB / CBZ / CBNZ / SUB-imm — タスク B がディスクを舐める                         |
| v0.12      | コア毎スケジューラ（MPIDR で A/B を並列実行）                                      |
| v0.13      | WFI — idle コアはスピンしない                                                     |
| v0.14      | AP ビット保護                                                                    |
| v0.15      | 逆アセンブラ + 編集可能なディスク                                                  |
| v0.16      | UI 全面改修：pin-out SoC 図、テーマトークン、コア固定スケジューラ                  |
| v0.17      | LDXR / STXR / CLREX + コア横断の排他モニタ（本物の同期プリミティブ）               |
| v0.18      | コア間 IPI（AIC のソフトウェア IRQ 経由）                                          |
| v1.0       | m1n1 経由で実機 Apple Silicon 上にベアメタル化                                     |

## ディレクトリ構成

```
aarch64-study/
├── crates/aarch64-sim/   Rust シミュレータ → wasm-pack → pkg/
├── src/
│   ├── views/cpu.tsx      メインのインタラクティブ画面
│   ├── components/        パネル、バッジ、コントロールバー、SoC 図
│   └── sim/               ドメイン型、フォーマッタ、イベント派生
├── Cargo.toml
└── package.json
```

## コントリビュート

Issue / PR を歓迎します。git-flow（メインは `develop`、リリースは `release/vX.Y.Z`）で
開発し、各リリースは「ひとつの概念をブラウザから見える形で示す」スタイルを保ちます。

## ライセンス

MIT — [LICENSE](LICENSE) を参照してください。
