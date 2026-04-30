# aarch64-study

`aarch64-study` 是一个跑在浏览器里的 **AArch64 SoC 模拟器**，目的是把操作系统底层
一砖一瓦地讲清楚——每个版本只引入一个架构概念。模拟器核心用 Rust 写，编译成
WebAssembly 在浏览器中运行：你能直接看到每条指令的取指、每次页表查询、每次异常
进入是怎么发生的。

> 在线演示 · <https://labs.golia.jp/aarch64/>

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-WASM-orange)](https://www.rust-lang.org/)
[![React](https://img.shields.io/badge/React-19-61dafb)](https://react.dev/)
[![TypeScript](https://img.shields.io/badge/TypeScript-6-3178c6)](https://www.typescriptlang.org/)

**其他语言** · [English](README.md) · [日本語](README.ja.md)

## 当前模拟了什么

- 两核（P-core / E-core，模仿 Apple Silicon 的形态），通过 `MPIDR_EL1` 区分
- 4 车道总线（DATA / ADDR / IRQ / CTRL）+ Apple 风格中断控制器
- AArch64 用户态指令子集（MOVZ、ADD/SUB、LDR/STR/LDP/STP、LDRB、B、CBZ/CBNZ、
  MSR/MRS、SVC、ERET、WFI、NOP/ISB/DMB、DAIFSet/Clr）
- EL0 / EL1 / EL2 三级特权 + 完整的 `ELR / SPSR / ESR / VBAR / DAIF` 系统寄存器
- 第一阶段 MMU 翻译（4 KiB 粒度，可配 T0SZ），带 AP 位保护——kernel 页 EL0 访问会触发 fault
- 类 virtio-blk 的块设备（8 × 64 字节扇区，MMIO 挂在 `0x3000`）
- 实时反汇编器，能解码模拟器支持的全部指令

## 本地跑起来

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

## 版本路线图

| 版本   | 加了什么                                                                   |
| ----- | -------------------------------------------------------------------------- |
| v0.1  | 寄存器 + 5 条指令（MOVZ ADD LDR STR B）+ MMIO UART                          |
| v0.2  | 第一阶段 MMU 页表查询（4 KiB 粒度）                                         |
| v0.3  | MSR/MRS + `SCTLR_EL1.M`（取指 / load / store 都过 MMU）                     |
| v0.4  | EL2 启动 + ERET 落到 EL1                                                    |
| v0.5  | SVC 把 EL0 提到 EL1 + ERET 返回（一次完整的系统调用往返）                    |
| v0.6  | 两核（P-core / E-core）共享内存 + `MPIDR_EL1`                               |
| v0.7  | DAIF + AIC 定时器 IRQ + IRQ 向量在 `VBAR + 0x480`                           |
| v0.8  | AIC 抽象 + scheduler 每个 tick 切换 A/B 任务                                |
| v0.9  | LDP/STP + 真正的上下文切换（X0–X3 跨切换保留）                               |
| v0.10 | 块设备（virtio-blk 形状）——kernel 启动时读 sector 0                          |
| v0.11 | LDRB / CBZ / CBNZ / SUB-imm；任务 B 遍历 disk buffer 并打印                  |
| v0.12 | 每核独立 scheduler——core 0/1 通过 MPIDR 同时跑 A/B                           |
| v0.13 | WFI——task A 睡到下一个 IRQ，空闲核不再忙转                                   |
| v0.14 | AP 位保护——内核页（AIC/Block）拒绝 EL0 访问                                  |
| v0.15 | 反汇编器、各核监视面板、可编辑的 disk text                                   |
| v0.16 | UI 大改：pin-out SoC 示意图、主题 token、等宽数据列、scheduler 固定每核任务   |
| v1.0  | 通过 m1n1 跑到真机——同一份 Rust crate 在 Apple Silicon 上裸跑                 |

## 项目结构

```
aarch64-study/
├── crates/
│   └── aarch64-sim/        # Rust 模拟器 → wasm-pack → pkg/
│       ├── src/lib.rs
│       └── pkg/             # 生成的 WASM 包，被 Web 端 import
├── src/
│   ├── views/cpu.tsx        # 主交互页面
│   ├── views/about.tsx
│   ├── components/          # 复用 UI 模块
│   ├── sim/                 # 领域类型 + 格式化辅助
│   ├── app.tsx              # 路由 + 布局
│   └── main.tsx
├── Cargo.toml               # workspace
├── package.json
└── README.md
```

## 贡献

欢迎 Issue / PR。开发模型是 git-flow，主分支 `develop`，每个 release 切
`release/vX.Y.Z`。每个 release 集中讲一个架构概念，新 PR 请保持这个节奏。
详见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可

MIT，见 [LICENSE](LICENSE)。
