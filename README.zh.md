# aarch64-study

跑在浏览器里的 **AArch64 SoC 模拟器**——每个版本只引入一个架构概念，把操作系统底层
一砖一瓦讲清楚。模拟器核心用 Rust 写、编译成 WebAssembly，每一次取指、每一次页表查询、
每一次异常进入都直接在浏览器中可见。

> 在线演示 · <https://labs.golia.jp/aarch64/>

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**其他语言** · [English](README.md) · [日本語](README.ja.md)

## 模拟了什么

- AArch64 用户态指令子集（MOVZ、ADD/SUB、LDR/STR/LDP/STP、LDRB、B、CBZ/CBNZ、
  MSR/MRS、SVC、ERET、WFI、NOP/ISB/DMB、DAIFSet/Clr）。
- EL0 / EL1 / EL2 三级特权 + 完整的 `ELR / SPSR / ESR / VBAR / DAIF` 系统寄存器。
- 第一阶段 MMU 翻译（4 KiB 粒度，可配 T0SZ），AP 位保护——kernel 页 EL0 访问触发 fault。
- 两核（P-core / E-core），通过 `MPIDR_EL1` 区分，各有独立上下文保存区。
- Apple 风格中断控制器（广播定时器 + 每核 pending mask）。
- 类 virtio-blk 块设备（8 × 64 字节扇区，MMIO 挂在 `0x3000`）。
- 实时反汇编器，覆盖所有支持的指令。

## 快速上手

```bash
bun install            # Web 依赖 + 本地 aarch64-sim crate
bun run build:sim      # Rust 模拟器编译到 WASM
bun run dev            # vite dev server，端口 32030
```

| 脚本                  | 作用                                             |
| --------------------- | ------------------------------------------------ |
| `bun run test:sim`    | `cargo test` 跑模拟器测试（36 个）               |
| `bun run test`        | React 侧 vitest                                  |
| `bun run check`       | tsc + eslint + prettier                          |
| `bun run build`       | 产线打包到 `./dist`                              |
| `bun run deploy`      | build + rsync `dist/` 到生产目标                  |

## 版本路线图

| 版本   | 概念                                                                        |
| ----- | --------------------------------------------------------------------------- |
| v0.1  | 寄存器 + 5 条指令 + MMIO UART                                               |
| v0.2  | MMU 第一阶段页表查询                                                         |
| v0.3  | MSR/MRS + `SCTLR_EL1.M` 生效                                                 |
| v0.4  | EL2 启动 + ERET 落到 EL1                                                     |
| v0.5  | SVC 把 EL0 提到 EL1 + ERET 返回                                              |
| v0.6  | 两核共享内存 + `MPIDR_EL1`                                                   |
| v0.7  | DAIF + AIC 定时器 IRQ + IRQ 向量在 `VBAR + 0x480`                            |
| v0.8  | AIC 抽象 + scheduler 切换 A/B 任务                                           |
| v0.9  | LDP/STP + 真正的上下文切换                                                   |
| v0.10 | virtio-blk 形状的块设备                                                      |
| v0.11 | LDRB / CBZ / CBNZ / SUB-imm；任务 B 遍历 disk buffer                         |
| v0.12 | 每核 scheduler（通过 MPIDR 同时跑 A/B）                                      |
| v0.13 | WFI——空闲核不再忙转                                                          |
| v0.14 | AP 位保护                                                                    |
| v0.15 | 反汇编器 + 可编辑 disk                                                       |
| v0.16 | UI 大改：pin-out SoC 图、主题 token、scheduler 固定每核任务                   |
| v0.17 | LDXR / STXR / CLREX + 跨核排他监视器（真正的并发原语）                       |
| v0.18 | 跨核 IPI——SVC 触发，AIC 软件 IRQ 派发                                        |
| v0.19 | crate 准备好可发布：纯 Rust API + `cli` 和 `wasm` 两个 feature              |
| v0.20 | 分级栈（`SP_EL0` / `SP_EL1`）+ `BL` / `RET` + 入栈/出栈                       |
| v0.21 | TLB + ASID + `TLBI` 失效                                                     |
| v0.22 | I-cache + `IC IVAU` / `DC CIVAC` + 自修改代码演示                            |
| v0.23 | PCB + round-robin 调度器（取代 v0.16 任务固定）                              |
| v1.0  | 通过 m1n1 跑到真机——同一份 Rust crate 在 Apple Silicon 上裸跑                 |

## 项目结构

```
aarch64-study/
├── crates/aarch64-sim/   Rust 模拟器 → wasm-pack → pkg/
├── src/
│   ├── views/cpu.tsx      主交互页面
│   ├── components/        面板、徽章、控制栏、SoC 图
│   └── sim/               领域类型、格式化、事件派生
├── Cargo.toml
└── package.json
```

## 贡献

欢迎 Issue / PR。分支模型 git-flow，主分支 `develop`，每个 release 切
`release/vX.Y.Z`。每个 release 集中讲一个概念。

## 许可

MIT，见 [LICENSE](LICENSE)。
