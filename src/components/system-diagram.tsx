// Dense SoC pin-out schematic. Two cores at the top (each with REGS / PC /
// MMU / EXC / DAIF sub-blocks and labelled bottom-edge pins), a four-lane
// parallel bus (DATA / ADDR / IRQ / CTRL) running across the middle,
// peripheral chips (AIC / UART / BLK) hanging off the bus with top-edge
// pins, and a 4×3 RAM region grid pinned to the bottom.
//
// All colours come from theme tokens (var(--color-…)) so the schematic
// reads cleanly in both light and dark mode. Per-chip accent colours are
// the only intentional hard-coded hues — they're a visual signature of the
// component and shouldn't change with theme.

import { Card } from '@goliapkg/gds'
import { useMemo } from 'react'

import { fmtHex32 } from '../sim/format'
import type { AicState, BlockState, CoreSlot, CoreState, NodeId, SimEvent } from '../sim/types'

// ── Geometry ──────────────────────────────────────────────────────────────
const SVG_W = 480
const SVG_H = 680

const LANE_DATA_Y = 234
const LANE_ADDR_Y = 254
const LANE_IRQ_Y = 274
const LANE_CTRL_Y = 294
const LANE_LEFT_X = 14
const LANE_RIGHT_X = SVG_W - 14

const CORE_W = 226
const CORE_H = 200
const CORE_Y = 14
const CORE0_X = 12
const CORE1_X = SVG_W - CORE_W - 12

const PERIPH_Y = 326
const PERIPH_H = 104
const AIC_X = 20
const UART_X = 174
const BLK_X = 332
const PERIPH_W = 130

const RAM_X = 20
const RAM_Y = 458
const RAM_W = SVG_W - 40
const RAM_H = 210

// Pin positions per chip — used both for drawing stubs and for routing
// any future event packets. `x` is the absolute SVG x; the y is on the
// chip's edge.
const corePins = (coreX: number) => ({
  data: coreX + 32,
  addr: coreX + 92,
  irq: coreX + 152,
  ctrl: coreX + 198,
})
const aicPins = { irq: AIC_X + 32, ack: AIC_X + 78, mask: AIC_X + 110 }
const uartPins = { data: UART_X + 36, csel: UART_X + 92 }
const blkPins = { data: BLK_X + 30, csel: BLK_X + 70, irq: BLK_X + 110 }

// Lane / accent colours. Hard-coded because they're the chip's identity.
const LANE_LABEL = 'var(--color-fg-muted)'
const LANE_DATA_COL = '#60a5fa'
const LANE_ADDR_COL = '#94a3b8'
const LANE_IRQ_COL = '#fbbf24'
const LANE_CTRL_COL = '#a78bfa'
const ACCENT_AIC = LANE_IRQ_COL
const ACCENT_UART = '#34d399'
const ACCENT_BLK = '#fb7185'

const FONT = "'Roboto Flex', system-ui, sans-serif"

// Centre x for each chip, kept around in case we ever wire up event
// packets that need a chip-level anchor.
const NODE_POS: Record<NodeId, { x: number; y: number }> = {
  core0: { x: CORE0_X + CORE_W / 2, y: CORE_Y + CORE_H / 2 },
  core1: { x: CORE1_X + CORE_W / 2, y: CORE_Y + CORE_H / 2 },
  aic: { x: AIC_X + PERIPH_W / 2, y: PERIPH_Y + PERIPH_H / 2 },
  uart: { x: UART_X + PERIPH_W / 2, y: PERIPH_Y + PERIPH_H / 2 },
  block: { x: BLK_X + PERIPH_W / 2, y: PERIPH_Y + PERIPH_H / 2 },
  ram: { x: RAM_X + RAM_W / 2, y: RAM_Y + RAM_H / 2 },
}

// ── Public component ──────────────────────────────────────────────────────
export interface SystemDiagramProps {
  aic: AicState
  block: BlockState
  cores: CoreState[]
  events: SimEvent[]
  output: string
  slots: CoreSlot[]
}

export function SystemDiagram({ aic, block, cores, events, output, slots }: SystemDiagramProps) {
  const ramRegions = useMemo(
    () =>
      [
        { addr: 0x0000, label: 'vectors' },
        { addr: 0x1000, label: 'UART mmio' },
        { addr: 0x2000, label: 'AIC mmio' },
        { addr: 0x3000, label: 'BLK mmio' },
        { addr: 0x4000, label: 'kernel' },
        { addr: 0x4d00, label: 'task A' },
        { addr: 0x4e00, label: 'task B' },
        { addr: 0x4f00, label: 'core 0 ctx' },
        { addr: 0x5000, label: 'core 1 ctx' },
        { addr: 0x6000, label: 'disk buf' },
        { addr: 0x7000, label: 'free' },
        { addr: 0x8000, label: 'page tbls' },
      ] as const,
    []
  )
  return (
    <Card padding="none">
      <div className="relative p-3">
        <div className="text-fg-muted mb-2 flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            AArch64 SoC · pin-out
          </span>
          <span className="type-small">
            {events.length > 0 ? `${events.length} active` : 'idle'}
          </span>
        </div>
        <svg
          className="block w-full"
          viewBox={`0 0 ${SVG_W} ${SVG_H}`}
          xmlns="http://www.w3.org/2000/svg"
        >
          {/* Solder-mask backing for the whole SoC strip */}
          <rect
            fill="var(--color-bg-secondary)"
            height={SVG_H - 4}
            rx="6"
            stroke="var(--color-border)"
            strokeWidth="0.5"
            width={SVG_W - 4}
            x={2}
            y={2}
          />

          <BusLanes />
          <CorePinStubs coreX={CORE0_X} />
          <CorePinStubs coreX={CORE1_X} />
          <PeripheralStubs />

          {/* RAM connection ribbon (CTRL lane → RAM top edge) */}
          <line
            stroke="rgb(168 85 247 / 0.35)"
            strokeDasharray="2 2"
            strokeWidth="0.8"
            x1={NODE_POS.ram.x}
            x2={NODE_POS.ram.x}
            y1={LANE_CTRL_Y}
            y2={RAM_Y}
          />

          <CoreChipSvg core={cores[0]} coreX={CORE0_X} slot={slots[0]} />
          <CoreChipSvg core={cores[1]} coreX={CORE1_X} slot={slots[1]} />

          <AicChip aic={aic} />
          <UartChip output={output} />
          <BlockChip block={block} />

          <RamGrid regions={ramRegions} />
        </svg>
      </div>
    </Card>
  )
}

// ── Bus + stub primitives ─────────────────────────────────────────────────
function BusLanes() {
  const lanes: { y: number; col: string; name: string }[] = [
    { y: LANE_DATA_Y, col: LANE_DATA_COL, name: 'DATA' },
    { y: LANE_ADDR_Y, col: LANE_ADDR_COL, name: 'ADDR' },
    { y: LANE_IRQ_Y, col: LANE_IRQ_COL, name: 'IRQ' },
    { y: LANE_CTRL_Y, col: LANE_CTRL_COL, name: 'CTRL' },
  ]
  // Labels live in the gap between the two cores (centre of the SoC) so
  // they never overlap with chip pins on either side.
  const labelX = SVG_W / 2
  return (
    <g>
      {lanes.map((l) => (
        <g key={l.name}>
          <line
            stroke={l.col}
            strokeOpacity="0.5"
            strokeWidth="1.4"
            x1={LANE_LEFT_X}
            x2={LANE_RIGHT_X}
            y1={l.y}
            y2={l.y}
          />
          {/* Pill behind the label so the bus line reads cleanly */}
          <rect
            fill="var(--color-bg-tertiary)"
            height="11"
            rx="2"
            stroke={l.col}
            strokeOpacity="0.4"
            strokeWidth="0.5"
            width="36"
            x={labelX - 18}
            y={l.y - 6}
          />
          <text
            fill={l.col}
            fontFamily={FONT}
            fontSize="8"
            fontWeight="700"
            letterSpacing="0.06em"
            textAnchor="middle"
            x={labelX}
            y={l.y + 2}
          >
            {l.name}
          </text>
        </g>
      ))}
    </g>
  )
}

function CorePinStubs({ coreX }: { coreX: number }) {
  const p = corePins(coreX)
  const stub = (x: number, lane: number, col: string) => (
    <line
      key={x}
      stroke={col}
      strokeOpacity="0.6"
      strokeWidth="0.8"
      x1={x}
      x2={x}
      y1={CORE_Y + CORE_H}
      y2={lane}
    />
  )
  return (
    <g>
      {stub(p.data, LANE_DATA_Y, LANE_DATA_COL)}
      {stub(p.addr, LANE_ADDR_Y, LANE_ADDR_COL)}
      {stub(p.irq, LANE_IRQ_Y, LANE_IRQ_COL)}
      {stub(p.ctrl, LANE_CTRL_Y, LANE_CTRL_COL)}
    </g>
  )
}

function PeripheralStubs() {
  const stub = (x: number, lane: number, col: string) => (
    <line
      key={`${x}-${lane}`}
      stroke={col}
      strokeOpacity="0.55"
      strokeWidth="0.8"
      x1={x}
      x2={x}
      y1={lane}
      y2={PERIPH_Y}
    />
  )
  return (
    <g>
      {/* AIC: ack→DATA, mask→ADDR, irq_out→IRQ */}
      {stub(aicPins.ack, LANE_DATA_Y, LANE_DATA_COL)}
      {stub(aicPins.mask, LANE_ADDR_Y, LANE_ADDR_COL)}
      {stub(aicPins.irq, LANE_IRQ_Y, LANE_IRQ_COL)}
      {/* UART: data→DATA, csel→ADDR */}
      {stub(uartPins.data, LANE_DATA_Y, LANE_DATA_COL)}
      {stub(uartPins.csel, LANE_ADDR_Y, LANE_ADDR_COL)}
      {/* BLK: data→DATA, csel→ADDR, irq→IRQ */}
      {stub(blkPins.data, LANE_DATA_Y, LANE_DATA_COL)}
      {stub(blkPins.csel, LANE_ADDR_Y, LANE_ADDR_COL)}
      {stub(blkPins.irq, LANE_IRQ_Y, LANE_IRQ_COL)}
    </g>
  )
}

// ── Core chip ─────────────────────────────────────────────────────────────
function CoreChipSvg({ core, coreX, slot }: { core: CoreState; coreX: number; slot: CoreSlot }) {
  const accent = core.id === 0 ? '#22d3ee' : '#a78bfa'
  const taskLabel = slot.entry === 0x4d00n ? 'task A' : slot.entry === 0x4e00n ? 'task B' : '—'
  const stateLine = core.last_trap ? 'TRAP' : core.halted ? 'HALT' : core.wfi_halted ? 'WFI' : 'RUN'
  const x = coreX
  const y = CORE_Y
  const innerL = x + 8
  const innerT = y + 24
  const regsW = 96
  const rightX = innerL + regsW + 8
  const rightW = CORE_W - 8 - regsW - 8 - 8
  return (
    <g>
      {/* Chip body */}
      <rect
        fill="var(--color-bg-tertiary)"
        height={CORE_H}
        rx="4"
        stroke={accent}
        strokeOpacity="0.55"
        strokeWidth="1"
        width={CORE_W}
        x={x}
        y={y}
      />
      {/* Header band */}
      <rect
        fill={`color-mix(in oklab, ${accent} 9%, transparent)`}
        height="18"
        rx="4"
        width={CORE_W}
        x={x}
        y={y}
      />
      <text fill={accent} fontFamily={FONT} fontSize="11" fontWeight="700" x={x + 8} y={y + 13}>
        CORE {core.id} · {core.kind}
      </text>
      <text
        fill={accent}
        fillOpacity="0.85"
        fontFamily={FONT}
        fontSize="9"
        x={x + CORE_W - 8}
        y={y + 13}
        textAnchor="end"
      >
        EL{core.current_el} · {stateLine}
      </text>

      {/* REGS sub-block (left column, fills available height) */}
      <SubBlock x={innerL} y={innerT} w={regsW} h={CORE_H - 32} title="REGS">
        <RegsList accent={accent} core={core} x={innerL + 4} y={innerT + 22} />
      </SubBlock>

      {/* Right column: PC, MMU, EXC, DAIF stacked with breathing room */}
      <SubBlock x={rightX} y={innerT} w={rightW} h={32} title="PC / IR">
        <text fill="var(--color-fg)" fontFamily={FONT} fontSize="11" x={rightX + 6} y={innerT + 26}>
          {fmtHex32(Number(core.pc))}
        </text>
      </SubBlock>
      <SubBlock x={rightX} y={innerT + 36} w={rightW} h={38} title="MMU">
        <text
          fill="var(--color-fg-secondary)"
          fontFamily={FONT}
          fontSize="9"
          x={rightX + 6}
          y={innerT + 36 + 22}
        >
          ttbr {fmtHex32(Number(core.ttbr0_el1))}
        </text>
        <text
          fill="var(--color-fg-muted)"
          fontFamily={FONT}
          fontSize="9"
          x={rightX + 6}
          y={innerT + 36 + 33}
        >
          tcr {fmtHex32(Number(core.tcr_el1))}
        </text>
      </SubBlock>
      <SubBlock x={rightX} y={innerT + 78} w={rightW} h={42} title="EXC">
        <text
          fill="var(--color-fg-secondary)"
          fontFamily={FONT}
          fontSize="9"
          x={rightX + 6}
          y={innerT + 78 + 22}
        >
          esr {fmtHex32(Number(core.esr_el1))}
        </text>
        <text
          fill="var(--color-fg-muted)"
          fontFamily={FONT}
          fontSize="9"
          x={rightX + 6}
          y={innerT + 78 + 35}
        >
          elr {fmtHex32(Number(core.elr_el1))}
        </text>
      </SubBlock>
      <SubBlock x={rightX} y={innerT + 124} w={rightW} h={32} title="DAIF">
        <text
          fill="var(--color-fg)"
          fontFamily={FONT}
          fontSize="9"
          x={rightX + 6}
          y={innerT + 124 + 24}
        >
          {core.daif.toString(2).padStart(4, '0')} · {taskLabel}
        </text>
      </SubBlock>

      {/* Bottom-edge pin markers — small coloured studs only, the bus-lane
          colour identifies what each pin carries so no per-pin text is needed. */}
      {(['data', 'addr', 'irq', 'ctrl'] as const).map((k) => {
        const px = corePins(coreX)[k]
        const col =
          k === 'data'
            ? LANE_DATA_COL
            : k === 'addr'
              ? LANE_ADDR_COL
              : k === 'irq'
                ? LANE_IRQ_COL
                : LANE_CTRL_COL
        return (
          <rect
            key={k}
            fill={col}
            fillOpacity="0.85"
            height="4"
            width="8"
            x={px - 4}
            y={y + CORE_H - 2}
          />
        )
      })}
    </g>
  )
}

function SubBlock({
  x,
  y,
  w,
  h,
  title,
  children,
}: {
  x: number
  y: number
  w: number
  h: number
  title: string
  children?: React.ReactNode
}) {
  return (
    <g>
      <rect
        fill="var(--color-bg-secondary)"
        height={h}
        rx="2"
        stroke="var(--color-border)"
        strokeWidth="0.5"
        width={w}
        x={x}
        y={y}
      />
      <text
        fill="var(--color-fg-muted)"
        fontFamily={FONT}
        fontSize="9"
        fontWeight="600"
        letterSpacing="0.02em"
        x={x + 4}
        y={y + 10}
      >
        {title}
      </text>
      {children}
    </g>
  )
}

function RegsList({
  accent,
  core,
  x,
  y,
}: {
  accent: string
  core: CoreState
  x: number
  y: number
}) {
  // A useful slice — X0..X3 for arg passing, X9..X10 used by the kernel,
  // SP and X30 (LR). 8 lines at 9px each with a 13px row pitch.
  const labels: { label: string; v: bigint }[] = [
    { label: 'x0', v: core.x[0] ?? 0n },
    { label: 'x1', v: core.x[1] ?? 0n },
    { label: 'x2', v: core.x[2] ?? 0n },
    { label: 'x3', v: core.x[3] ?? 0n },
    { label: 'x9', v: core.x[9] ?? 0n },
    { label: 'x10', v: core.x[10] ?? 0n },
    { label: 'sp', v: core.sp ?? 0n },
    { label: 'x30', v: core.x[30] ?? 0n },
  ]
  return (
    <g>
      {labels.map((r, i) => (
        <g key={r.label}>
          <text
            fill="var(--color-fg-muted)"
            fontFamily={FONT}
            fontSize="9"
            x={x}
            y={y + i * 13 + 8}
          >
            {r.label}
          </text>
          <text
            fill={accent}
            fillOpacity="0.95"
            fontFamily={FONT}
            fontSize="9"
            x={x + 24}
            y={y + i * 13 + 8}
          >
            {fmtHex32(Number(r.v & 0xffffffffn))}
          </text>
        </g>
      ))}
    </g>
  )
}

// ── Peripherals ────────────────────────────────────────────────────────────
function PeripheralChip({
  accent,
  pinLabels,
  pinXs,
  bodyLines,
  title,
  topX,
}: {
  accent: string
  pinLabels: string[]
  pinXs: number[]
  bodyLines: string[]
  title: string
  topX: number
}) {
  return (
    <g>
      <rect
        fill="var(--color-bg-tertiary)"
        height={PERIPH_H}
        rx="4"
        stroke={accent}
        strokeOpacity="0.55"
        strokeWidth="1"
        width={PERIPH_W}
        x={topX}
        y={PERIPH_Y}
      />
      <rect
        fill={`color-mix(in oklab, ${accent} 9%, transparent)`}
        height="18"
        rx="4"
        width={PERIPH_W}
        x={topX}
        y={PERIPH_Y}
      />
      <text
        fill={accent}
        fontFamily={FONT}
        fontSize="11"
        fontWeight="700"
        x={topX + 8}
        y={PERIPH_Y + 13}
      >
        {title}
      </text>
      {bodyLines.map((ln, i) => (
        <text
          key={i}
          fill="var(--color-fg-secondary)"
          fontFamily={FONT}
          fontSize="9"
          x={topX + 8}
          y={PERIPH_Y + 32 + i * 12}
        >
          {ln}
        </text>
      ))}
      {pinXs.map((px, i) => (
        <g key={i}>
          <rect fill={accent} fillOpacity="0.7" height="3" width="6" x={px - 3} y={PERIPH_Y - 2} />
          <text
            fill={LANE_LABEL}
            fontFamily={FONT}
            fontSize="9"
            textAnchor="middle"
            x={px}
            y={PERIPH_Y - 4}
          >
            {pinLabels[i]}
          </text>
        </g>
      ))}
    </g>
  )
}

function AicChip({ aic }: { aic: AicState }) {
  const pendingMask = aic.pending.reduce((acc, p) => acc | p, 0)
  return (
    <PeripheralChip
      accent={ACCENT_AIC}
      bodyLines={[
        `pnd ${pendingMask.toString(16).padStart(2, '0')}`,
        `ack ${aic.total_acks}`,
        `targets ${aic.pending.length}`,
      ]}
      pinLabels={['IRQ', 'ACK', 'MSK']}
      pinXs={[aicPins.irq, aicPins.ack, aicPins.mask]}
      title="AIC"
      topX={AIC_X}
    />
  )
}

function UartChip({ output }: { output: string }) {
  // Strip newlines and cap to 8 chars so the line fits inside a 130px chip.
  const tail = output.replace(/\n/g, ' ').trimEnd().slice(-8)
  return (
    <PeripheralChip
      accent={ACCENT_UART}
      bodyLines={[`bytes ${output.length}`, `tail ${tail || '—'}`, 'tx fifo 16']}
      pinLabels={['DAT', 'CS']}
      pinXs={[uartPins.data, uartPins.csel]}
      title="UART"
      topX={UART_X}
    />
  )
}

function BlockChip({ block }: { block: BlockState }) {
  return (
    <PeripheralChip
      accent={ACCENT_BLK}
      bodyLines={[
        `r ${block.total_reads} · w ${block.total_writes}`,
        `sec ${block.sector}`,
        `buf 0x${(Number(block.buf_addr) >>> 0).toString(16).padStart(4, '0')}`,
      ]}
      pinLabels={['DAT', 'CS', 'IRQ']}
      pinXs={[blkPins.data, blkPins.csel, blkPins.irq]}
      title="BLK"
      topX={BLK_X}
    />
  )
}

// ── RAM as 4×3 region grid ────────────────────────────────────────────────
function RamGrid({ regions }: { regions: readonly { addr: number; label: string }[] }) {
  const cols = 4
  const cellW = (RAM_W - 8) / cols
  const cellH = (RAM_H - 24) / 3
  return (
    <g>
      <rect
        fill="var(--color-bg-tertiary)"
        height={RAM_H}
        rx="4"
        stroke={LANE_DATA_COL}
        strokeOpacity="0.5"
        strokeWidth="1"
        width={RAM_W}
        x={RAM_X}
        y={RAM_Y}
      />
      <rect
        fill={`color-mix(in oklab, ${LANE_DATA_COL} 8%, transparent)`}
        height="18"
        rx="4"
        width={RAM_W}
        x={RAM_X}
        y={RAM_Y}
      />
      <text
        fill={LANE_DATA_COL}
        fontFamily={FONT}
        fontSize="11"
        fontWeight="700"
        x={RAM_X + 8}
        y={RAM_Y + 13}
      >
        RAM · 64 KiB · 16-bank grid
      </text>
      <text
        fill={LANE_LABEL}
        fontFamily={FONT}
        fontSize="9"
        x={RAM_X + RAM_W - 8}
        y={RAM_Y + 13}
        textAnchor="end"
      >
        {regions.length} regions
      </text>
      {regions.map((r, i) => {
        const c = i % cols
        const row = Math.floor(i / cols)
        const cx = RAM_X + 4 + c * cellW
        const cy = RAM_Y + 22 + row * cellH
        return (
          <g key={r.addr}>
            <rect
              fill="rgb(96 165 250 / 0.06)"
              height={cellH - 2}
              rx="2"
              stroke="rgb(96 165 250 / 0.35)"
              strokeWidth="0.5"
              width={cellW - 2}
              x={cx}
              y={cy}
            />
            <text
              fill="#60a5fa"
              fillOpacity="0.95"
              fontFamily={FONT}
              fontSize="9"
              fontWeight="600"
              x={cx + 6}
              y={cy + 14}
            >
              0x{r.addr.toString(16).padStart(4, '0').toUpperCase()}
            </text>
            <text
              fill="var(--color-fg-secondary)"
              fontFamily={FONT}
              fontSize="9"
              x={cx + 6}
              y={cy + 28}
            >
              {r.label}
            </text>
          </g>
        )
      })}
    </g>
  )
}
