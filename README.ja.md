# aarch64-study

`aarch64-study` は **AArch64 SoC のシミュレータ**で、すべての処理がブラウザ内で
動きます。Rust で書かれたシミュレータコアを WebAssembly にコンパイルし、命令の
フェッチ、ページテーブルウォーク、例外エントリといったハードウェアの内部動作を
ブラウザから直接観察できます。各バージョン（`v0.1` 〜 `v0.16`）は、前のバージョンに
**ひとつだけ**アーキテクチャ概念を追加していくスタイルなので、差分を読むだけで
学習が進みます。

> ライブデモ · <https://labs.golia.jp/aarch64/>

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-WASM-orange)](https://www.rust-lang.org/)
[![React](https://img.shields.io/badge/React-19-61dafb)](https://react.dev/)
[![TypeScript](https://img.shields.io/badge/TypeScript-6-3178c6)](https://www.typescriptlang.org/)

**他の言語** · [English](README.md) · [中文](README.zh.md)

## 現在モデル化しているもの

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

## ローカルで動かす

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

## ロードマップ

| バージョン | 追加された内容                                                                |
| ---------- | ----------------------------------------------------------------------------- |
| v0.1       | レジスタ + 5 命令（MOVZ ADD LDR STR B）+ MMIO UART                            |
| v0.2       | ステージ 1 MMU のページテーブルウォーク（4 KiB グラニュール）                  |
| v0.3       | MSR/MRS + `SCTLR_EL1.M` を尊重したフェッチ / load / store                     |
| v0.4       | EL2 ブート + ERET で EL1 に降りる                                              |
| v0.5       | SVC で EL0 → EL1 へ昇格 + ERET で戻る                                         |
| v0.6       | 2 コア（P-core / E-core）でメモリ共有 + `MPIDR_EL1`                            |
| v0.7       | DAIF + AIC タイマ IRQ + IRQ ベクタ `VBAR + 0x480`                             |
| v0.8       | AIC 抽象化 + スケジューラがティック毎にタスク A/B を入れ替え                   |
| v0.9       | LDP/STP + 本物のコンテキストスイッチ（X0–X3 が切り替えを跨ぐ）                 |
| v0.10      | virtio-blk 風ブロックデバイス — カーネルが起動時にセクタ 0 を読む              |
| v0.11      | LDRB / CBZ / CBNZ / SUB-imm — タスク B がディスクバッファを舐めて出力          |
| v0.12      | コアごとのスケジューラ — core 0/1 が MPIDR で A/B を並列実行                  |
| v0.13      | WFI — タスク A は次の IRQ まで眠り、idle コアはスピンしない                    |
| v0.14      | AP ビット保護 — kernel ページ（AIC/Block）は EL0 から拒否される                |
| v0.15      | 逆アセンブラ、コア別モニタ、編集可能なディスクテキスト                          |
| v0.16      | UI 全面改修：pin-out SoC 図、テーマトークン、等幅データ列、コア固定スケジューラ |
| v1.0       | m1n1 経由でベアメタル化 — 同じ Rust crate を実機 Apple Silicon で動かす        |

## リポジトリ構成

```
aarch64-study/
├── crates/
│   └── aarch64-sim/        # Rust シミュレータ → wasm-pack → pkg/
│       ├── src/lib.rs
│       └── pkg/             # 生成物。Web から import される
├── src/
│   ├── views/cpu.tsx        # メインのインタラクティブ画面
│   ├── views/about.tsx
│   ├── components/          # 再利用 UI 部品
│   ├── sim/                 # ドメイン型 + フォーマットヘルパー
│   ├── app.tsx              # ルータ + レイアウト
│   └── main.tsx
├── Cargo.toml               # workspace
├── package.json
└── README.md
```

## コントリビュート

Issue / PR を歓迎します。開発は git-flow（メインは `develop`、リリースは
`release/vX.Y.Z`）で行い、各リリースは「ひとつの概念をブラウザから見える形で示す」
というスタイルを保ちます。詳細は [CONTRIBUTING.md](CONTRIBUTING.md) を参照してください。

## ライセンス

MIT — [LICENSE](LICENSE) を参照してください。
