import { Badge, Button, Card, GlassButton } from '@goliapkg/gds'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import init, { Cpu, disassemble } from 'aarch64-sim'

import { SystemDiagram } from '../components/system-diagram'
import {
  IRQ_NAMES,
  MEMORY_VIEW_BYTES,
  REG_LABELS,
  RUN_BURST,
  fmtHex32,
  fmtHex64,
  inferTaskLabel,
  parseCoreSlot,
  parseHex,
} from '../sim/format'
import type {
  AicState,
  BlockState,
  CoreSlot,
  CoreState,
  PrevSnapshot,
  SimEvent,
  SystemInfo,
  TaskSave,
  TranslationResult,
  WalkOutcome,
  WalkStep,
} from '../sim/types'

export function CpuView() {
  const [cpu, setCpu] = useState<Cpu | null>(null)
  const [cores, setCores] = useState<CoreState[] | null>(null)
  const [aic, setAic] = useState<AicState | null>(null)
  const [block, setBlock] = useState<BlockState | null>(null)
  const [coreSlots, setCoreSlots] = useState<CoreSlot[] | null>(null)
  const [sysInfo, setSysInfo] = useState<SystemInfo | null>(null)
  const [memory, setMemory] = useState<Uint8Array>(new Uint8Array(MEMORY_VIEW_BYTES))
  const [output, setOutput] = useState('')
  const [running, setRunning] = useState(false)
  const [vaText, setVaText] = useState('0x4000')
  const [translateCoreIdx, setTranslateCoreIdx] = useState(0)
  const [events, setEvents] = useState<SimEvent[]>([])
  const runRafRef = useRef<number | null>(null)
  const prevRef = useRef<PrevSnapshot | null>(null)
  const eventIdRef = useRef(0)

  const refresh = useCallback((c: Cpu) => {
    const s = c.state() as CoreState[]
    setCores(s)
    const aicState = c.aic_state() as AicState
    const blockState = c.block_state() as BlockState
    setAic(aicState)
    setBlock(blockState)
    const ticks = c.timer_ticks()
    setSysInfo({
      systemSteps: c.system_steps(),
      timerPeriod: c.timer_period(),
      timerRemaining: c.timer_remaining(),
      timerTicks: ticks,
    })
    // Center memory on core 0's PC.
    const viewStart = Number(s[0].pc) & ~0xf
    setMemory(c.mem_slice(viewStart, MEMORY_VIEW_BYTES))
    const newOutput = c.output()
    setOutput(newOutput)
    // Pull each core's slot region (0x4F00 = core 0, 0x5000 = core 1).
    const slot0 = parseCoreSlot(c.mem_slice(0x4f00, 0x50))
    const slot1 = parseCoreSlot(c.mem_slice(0x5000, 0x50))
    setCoreSlots([slot0, slot1])

    // --- Event detection: diff against previous snapshot ---
    const now = performance.now()
    const newEvents: SimEvent[] = []
    const prev = prevRef.current
    if (prev) {
      // Store events: output grew. Attribute to whichever core(s) are at EL0
      // and not WFI/halted (the only ones that could've STR'd).
      if (newOutput.length > prev.outputLen) {
        s.forEach((core, i) => {
          if (!core.wfi_halted && !core.halted && core.current_el === 0) {
            newEvents.push({
              id: ++eventIdRef.current,
              kind: 'store',
              source: i === 0 ? 'core0' : 'core1',
              target: 'uart',
              ts: now,
            })
          }
        })
      }
      // Timer fire: AIC raises IRQ on every core.
      if (ticks > prev.ticks) {
        newEvents.push({
          id: ++eventIdRef.current,
          kind: 'timer',
          source: 'aic',
          target: 'core0',
          ts: now,
        })
        newEvents.push({
          id: ++eventIdRef.current,
          kind: 'timer',
          source: 'aic',
          target: 'core1',
          ts: now + 30,
        })
      }
      // Disk read: block.total_reads++ → bytes flowed Block → RAM.
      if (blockState.total_reads > prev.totalReads) {
        newEvents.push({
          id: ++eventIdRef.current,
          kind: 'disk_read',
          source: 'block',
          target: 'ram',
          ts: now,
        })
      }
      // Per-core PC transitions: IRQ taken / SVC entry / ERET drop.
      s.forEach((core, i) => {
        const prevCore = prev.cores[i]
        if (!prevCore) return
        const pc = Number(core.pc)
        const prevPc = Number(prevCore.pc)
        const vbar = 0x4400 // demo VBAR_EL1
        if (pc === vbar + 0x480 && prevPc !== vbar + 0x480) {
          newEvents.push({
            id: ++eventIdRef.current,
            kind: 'irq_taken',
            source: 'aic',
            target: i === 0 ? 'core0' : 'core1',
            ts: now,
          })
        }
        if (pc === vbar + 0x400 && prevPc !== vbar + 0x400) {
          newEvents.push({
            id: ++eventIdRef.current,
            kind: 'svc',
            source: i === 0 ? 'core0' : 'core1',
            target: i === 0 ? 'core0' : 'core1',
            ts: now,
          })
        }
        if (core.current_el < prevCore.current_el) {
          newEvents.push({
            id: ++eventIdRef.current,
            kind: 'eret',
            source: i === 0 ? 'core0' : 'core1',
            target: i === 0 ? 'core0' : 'core1',
            ts: now,
          })
        }
      })
    }
    if (newEvents.length > 0) {
      setEvents((prevEvents) => [...prevEvents.filter((e) => now - e.ts < 800), ...newEvents])
    } else {
      setEvents((prevEvents) => prevEvents.filter((e) => now - e.ts < 800))
    }

    prevRef.current = {
      cores: s.map((c2) => ({
        pc: c2.pc,
        current_el: c2.current_el,
        wfi_halted: c2.wfi_halted,
      })),
      outputLen: newOutput.length,
      ticks,
      totalReads: blockState.total_reads,
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    void (async () => {
      await init()
      if (cancelled) return
      const c = new Cpu()
      setCpu(c)
      refresh(c)
    })()
    return () => {
      cancelled = true
    }
  }, [refresh])

  const onStep = useCallback(() => {
    if (!cpu) return
    cpu.step()
    refresh(cpu)
  }, [cpu, refresh])

  const onStepCore = useCallback(
    (idx: number) => {
      if (!cpu) return
      cpu.step_core(idx)
      refresh(cpu)
    },
    [cpu, refresh]
  )

  const onReset = useCallback(() => {
    if (!cpu) return
    cpu.reset()
    setRunning(false)
    if (runRafRef.current != null) {
      cancelAnimationFrame(runRafRef.current)
      runRafRef.current = null
    }
    refresh(cpu)
  }, [cpu, refresh])

  const onRunToggle = useCallback(() => {
    setRunning((r) => {
      // Resuming from auto-pause: if every core is already idle (halted or
      // WFI), pressing Run again would just immediately re-pause. Reset
      // first so the user gets a fresh boot run.
      if (!r && cpu) {
        const states = cpu.state() as CoreState[]
        if (states.length > 0 && states.every((s) => s.halted || s.wfi_halted)) {
          cpu.reset()
          refresh(cpu)
        }
      }
      return !r
    })
  }, [cpu, refresh])

  useEffect(() => {
    if (!running || !cpu) return
    // The system has nothing left to observe when every core is either
    // halted (trap / b-to-self) or parked in WFI. Auto-run pauses itself in
    // that case — the user can press Step to keep poking, or Reset.
    const allIdle = (states: CoreState[]) => states.every((s) => s.halted || s.wfi_halted)
    const tick = () => {
      const states = cpu.state() as CoreState[]
      if (allIdle(states)) {
        setRunning(false)
        return
      }
      cpu.run(RUN_BURST)
      refresh(cpu)
      const after = cpu.state() as CoreState[]
      if (allIdle(after)) {
        setRunning(false)
        return
      }
      runRafRef.current = requestAnimationFrame(tick)
    }
    runRafRef.current = requestAnimationFrame(tick)
    return () => {
      if (runRafRef.current != null) {
        cancelAnimationFrame(runRafRef.current)
        runRafRef.current = null
      }
    }
  }, [running, cpu, refresh])

  // Re-translate using the selected core's sysregs whenever cores or VA change.
  const trace = useMemo<TranslationResult | null>(() => {
    if (!cpu || !cores) return null
    const va = parseHex(vaText)
    if (va === null) return null
    try {
      return cpu.translate(va, translateCoreIdx) as TranslationResult
    } catch {
      return null
    }
  }, [cpu, cores, vaText, translateCoreIdx])

  if (!cpu || !cores || !sysInfo || !aic || !coreSlots || !block) {
    return <div className="text-fg-muted type-base">Loading WASM…</div>
  }

  const allHalted = cores.every((c) => c.halted)
  const anyTrap = cores.find((c) => c.last_trap != null)
  const memBaseAddr = Number(cores[0].pc) & ~0xf
  const corePcs = cores.map((c) => Number(c.pc))

  return (
    <div className="space-y-4">
      <header className="space-y-2">
        <div className="flex flex-wrap items-center gap-3">
          <h1
            className="text-fg text-2xl font-bold"
            style={{ textShadow: '0 0 20px var(--gds-accent, #3b82f6)' }}
          >
            AArch64 CPU
          </h1>
          <Badge variant="info">v0.16</Badge>
          {cores.map((c) => (
            <CoreChip core={c} key={c.id} />
          ))}
          {anyTrap ? (
            <Badge variant="danger">TRAP</Badge>
          ) : allHalted ? (
            <Badge variant="success">ALL HALTED</Badge>
          ) : running ? (
            <Badge>
              <span className="live-pulse mr-1 inline-block h-1.5 w-1.5 rounded-full bg-emerald-400" />
              RUNNING
            </Badge>
          ) : cores.every((c) => c.wfi_halted) ? (
            <Badge variant="info">IDLE</Badge>
          ) : (
            <Badge variant="info">PAUSED</Badge>
          )}
        </div>
        <p className="text-fg-muted type-small max-w-2xl">
          A 2-core AArch64 SoC running entirely in the browser. The right rail is a{' '}
          <strong>pin-out schematic</strong> with a 4-lane bus and event packets that fly along the
          matching lane (DATA / ADDR / IRQ / CTRL). The <strong>disassembly</strong> panel is
          centered on PC; the <strong>disk sector 0</strong> at the top is editable — type whatever
          you want and task B will print it via the UART.
        </p>
      </header>

      <ControlBar
        aic={aic}
        block={block}
        cores={cores}
        cpu={cpu}
        onRefresh={refresh}
        onReset={onReset}
        onRunToggle={onRunToggle}
        onStep={onStep}
        running={running}
        sysInfo={sysInfo}
        totalCoreSteps={cores.reduce((a, c) => a + c.steps, 0n)}
        uartBytes={output.length}
      />

      {anyTrap && anyTrap.last_trap && (
        <div className="border-danger/40 bg-danger/10 text-danger type-small rounded border px-3 py-2">
          core {anyTrap.id}: {anyTrap.last_trap}
        </div>
      )}

      {/* Strict 3-col grid — every panel is exactly 1/3 width, no col-span. */}
      <div className="grid min-w-0 grid-cols-1 gap-4 lg:grid-cols-3">
        <SystemDiagram
          aic={aic}
          block={block}
          cores={cores}
          events={events}
          output={output}
          slots={coreSlots}
        />
        {/* MMU on top, Output below — Output's <pre> stretches to fill the
            rest of the column's height (so it matches the tall
            SystemDiagram on the left). */}
        <div className="flex h-full flex-col gap-4">
          <MmuPanel
            cores={cores}
            onCoreChange={setTranslateCoreIdx}
            onVaChange={setVaText}
            selectedCoreIdx={translateCoreIdx}
            trace={trace}
            vaText={vaText}
          />
          <div className="min-h-0 flex-1">
            <OutputPanel output={output} />
          </div>
        </div>
        <DisassemblyPanel cpu={cpu} cores={cores} />
        {/* Memory + per-core save areas + AIC stacked vertically. Memory
            is the dominant content and grows to fill any leftover height
            so the column matches the taller right column. */}
        <div className="flex h-full flex-col gap-4">
          <div className="min-h-0 flex-1">
            <MemoryPanel base={memBaseAddr} bytes={memory} pcs={corePcs} />
          </div>
          <SavePanel slots={coreSlots} />
          <AicPanel aic={aic} />
        </div>
        {/* core 0 over core 1 stacked. */}
        <div className="space-y-4">
          {cores.map((c) => (
            <CoreColumn core={c} key={c.id} onStep={() => onStepCore(c.id)} />
          ))}
        </div>
        <BlockPanel block={block} />
      </div>
    </div>
  )
}

const SECTOR_SIZE = 64

function BlockPanel({ block }: { block: BlockState }) {
  const numSectors = Math.floor(block.disk.length / SECTOR_SIZE)
  const statusLabel =
    block.status === 0n
      ? 'IDLE'
      : block.status === 1n
        ? 'OK'
        : block.status === 2n
          ? 'FAULT'
          : `0x${block.status.toString(16)}`
  return (
    <Card padding="none">
      <div className="space-y-3 p-4">
        <div className="text-fg-muted flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            Block device · disk image ({numSectors} × 64-byte sectors @ MMIO 0x3000)
          </span>
          <span className="type-small">
            reads {block.total_reads.toString()} · writes {block.total_writes.toString()}
          </span>
        </div>
        <div className="type-small grid gap-x-6 gap-y-1 sm:grid-cols-4">
          <RegRow label="SECTOR" value={block.sector} />
          <RegRow label="BUF_ADDR" value={block.buf_addr} />
          <RegRow label="CMD" value={block.last_command} />
          <div className="flex justify-between gap-2">
            <span className="text-fg-muted">STATUS</span>
            <span className="text-fg">{statusLabel}</span>
          </div>
        </div>
        <div className="space-y-2">
          {Array.from({ length: numSectors }, (_, i) => {
            const slice = new Uint8Array(block.disk.slice(i * SECTOR_SIZE, (i + 1) * SECTOR_SIZE))
            return (
              <div className="border-border rounded border px-2 py-1.5" key={i}>
                <div className="text-fg-muted type-small mb-1 tracking-wider uppercase">
                  sector {i}
                </div>
                <DiskHexRow bytes={slice} sector={i} />
              </div>
            )
          })}
        </div>
      </div>
    </Card>
  )
}

function DiskHexRow({ bytes, sector }: { bytes: Uint8Array; sector: number }) {
  const rows: Uint8Array[] = []
  for (let i = 0; i < bytes.length; i += 16) {
    rows.push(bytes.slice(i, i + 16))
  }
  const baseAddr = sector * SECTOR_SIZE
  return (
    <div className="mono-data type-small overflow-x-auto">
      {rows.map((row, ri) => {
        let ascii = ''
        for (let i = 0; i < row.length; i++) {
          const c = row[i]
          ascii += c >= 0x20 && c < 0x7f ? String.fromCharCode(c) : '.'
        }
        return (
          <div className="flex gap-4 leading-6" key={ri}>
            <span className="text-fg-muted">
              {(baseAddr + ri * 16).toString(16).padStart(4, '0')}
            </span>
            <span className="text-fg flex-1">
              {Array.from(row, (b) => b.toString(16).padStart(2, '0')).join(' ')}
            </span>
            <span className="text-fg-muted">{ascii}</span>
          </div>
        )
      })}
    </div>
  )
}

function SavePanel({ slots }: { slots: CoreSlot[] }) {
  return (
    <Card padding="none">
      <div className="space-y-2 p-4">
        <div className="text-fg-muted type-small font-semibold tracking-wider uppercase">
          Per-core scheduler slots — entry + save_ptr + 2 save areas each
        </div>
        <div className="grid gap-3">
          {slots.map((slot, i) => (
            <CoreSlotCard base={i === 0 ? 0x4f00 : 0x5000} coreId={i} key={i} slot={slot} />
          ))}
        </div>
        <div className="text-fg-muted type-small">
          Each core derives its slot base from MPIDR_EL1: bit 8 (cluster) maps directly to 0x4F00 /
          0x5000. The scheduler reads "current entry" / "current save_ptr", saves X0–X3 there with
          STP, swaps via the (sum − current) trick, restores the other save area with LDP, and ERETs
          into the other task. X3 is preserved across context switches.
        </div>
      </div>
    </Card>
  )
}

function CoreSlotCard({ base, coreId, slot }: { base: number; coreId: number; slot: CoreSlot }) {
  const taskLabel = slot.entry === 0x4d00n ? 'task A' : slot.entry === 0x4e00n ? 'task B' : '?'
  const activeIdx =
    slot.savePtr === BigInt(base + 0x10) ? 0 : slot.savePtr === BigInt(base + 0x30) ? 1 : null
  return (
    <div className="border-border bg-bg-secondary type-small space-y-2 rounded border px-3 py-2">
      <div className="text-fg-muted type-small flex items-center justify-between tracking-wider uppercase">
        <span>
          core {coreId} slot @ 0x{base.toString(16)}
        </span>
        <span className="text-accent normal-case">running {taskLabel}</span>
      </div>
      <div className="grid gap-x-4 gap-y-0.5">
        <RegRow label="entry" value={slot.entry} />
        <RegRow label="save_ptr" value={slot.savePtr} />
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        <SaveAreaCard active={activeIdx === 0} addr={base + 0x10} index={0} save={slot.save0} />
        <SaveAreaCard active={activeIdx === 1} addr={base + 0x30} index={1} save={slot.save1} />
      </div>
    </div>
  )
}

function SaveAreaCard({
  active,
  addr,
  index,
  save,
}: {
  active: boolean
  addr: number
  index: number
  save: TaskSave
}) {
  const x0Char = Number(save.x0 & 0xffn)
  const ascii = x0Char >= 0x20 && x0Char < 0x7f ? `'${String.fromCharCode(x0Char)}'` : ''
  return (
    <div
      className={`min-w-0 rounded border px-2 py-1 ${
        active ? 'border-accent/60 bg-accent/5' : 'border-border'
      }`}
    >
      <div
        className={`type-small mb-1 flex items-center justify-between gap-2 tracking-wider uppercase ${
          active ? 'text-accent' : 'text-fg-muted'
        }`}
      >
        <span className="min-w-0 truncate">
          save {index} · 0x{addr.toString(16)}
        </span>
        {active && <span className="shrink-0">active</span>}
      </div>
      <SaveRegRow label={`X0 ${ascii}`} value={save.x0} />
      <SaveRegRow label="X1" value={save.x1} />
      <SaveRegRow label="X2" value={save.x2} />
      <SaveRegRow highlight label="X3" value={save.x3} />
    </div>
  )
}

// Compact RegRow for the per-core save areas — these columns are too narrow
// for a 16-hex-digit value, so we render only the lower 32 bits (the upper
// bits are zero for ASCII saves anyway).
function SaveRegRow({
  highlight,
  label,
  value,
}: {
  highlight?: boolean
  label: string
  value: bigint
}) {
  return (
    <div className={`type-small flex justify-between gap-2 ${highlight ? 'text-accent' : ''}`}>
      <span className="text-fg-muted shrink-0">{label}</span>
      <span className={`mono-data truncate ${value === 0n ? 'text-fg-muted' : 'text-fg'}`}>
        0x{(Number(value & 0xffffffffn) >>> 0).toString(16).padStart(8, '0')}
      </span>
    </div>
  )
}

function AicPanel({ aic }: { aic: AicState }) {
  return (
    <Card padding="none">
      <div className="space-y-2 p-4">
        <div className="text-fg-muted flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            AIC · Apple-style interrupt controller
          </span>
          <span className="type-small">
            base 0x2000 · ACK reads cleared {aic.total_acks.toString()}
          </span>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          {aic.pending.map((bits, i) => (
            <div
              className="border-border bg-bg-secondary type-small rounded border px-3 py-2"
              key={i}
            >
              <div className="text-fg-muted type-small mb-1 tracking-wider uppercase">
                core {i} pending
              </div>
              <div className="flex items-center gap-2">
                <span className="text-fg">0x{bits.toString(16).padStart(8, '0')}</span>
                <span className="flex gap-1">
                  {IRQ_NAMES.map((name, b) => (
                    <span
                      className={
                        (bits & (1 << b)) !== 0
                          ? 'rounded border border-amber-500/60 bg-amber-500/20 px-1.5 py-0.5 text-amber-900 dark:text-amber-100'
                          : 'border-border text-fg-muted rounded border px-1.5 py-0.5'
                      }
                      key={name}
                    >
                      {name}
                    </span>
                  ))}
                </span>
              </div>
            </div>
          ))}
        </div>
      </div>
    </Card>
  )
}

function DisassemblyPanel({ cpu, cores }: { cpu: Cpu; cores: CoreState[] }) {
  const pc0 = Number(cores[0].pc)
  const pc1 = Number(cores[1].pc)
  // Show 32 instructions centered on core 0's PC, snapped to a 32-byte
  // boundary so the listing doesn't jitter line-by-line.
  const center = pc0 & ~0x1f
  const start = Math.max(0x4000, center - 16 * 4)
  const end = Math.min(0x10000, start + 36 * 4)
  const bytes = cpu.mem_slice(start, end - start)
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const rows: { pa: number; word: number; mnem: string }[] = []
  for (let i = 0; i + 4 <= bytes.length; i += 4) {
    const word = view.getUint32(i, true)
    const pa = start + i
    rows.push({ pa, word, mnem: disassemble(word, BigInt(pa)) })
  }
  return (
    <Card padding="none">
      <div className="space-y-2 p-4">
        <div className="text-fg-muted flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            Disassembly · around core 0 PC
          </span>
          <span className="type-small">
            range {fmtHex32(start)}–{fmtHex32(end - 1)}
          </span>
        </div>
        <div className="mono-data type-small overflow-x-auto">
          {rows.map((r) => {
            const isCore0 = r.pa === pc0
            const isCore1 = r.pa === pc1
            const isPc = isCore0 || isCore1
            const cls = isCore0
              ? 'bg-cyan-500/10 text-fg'
              : isCore1
                ? 'bg-violet-500/10 text-fg'
                : ''
            return (
              <div className={`flex gap-3 leading-6 ${cls}`} key={r.pa}>
                <span
                  className={`flex w-12 shrink-0 items-center gap-1 ${
                    isCore0
                      ? 'live-pulse text-cyan-700 dark:text-cyan-300'
                      : isCore1
                        ? 'live-pulse text-violet-700 dark:text-violet-300'
                        : 'text-fg-muted'
                  }`}
                >
                  {isPc && <span>►</span>}
                  {isCore0 ? <span>0</span> : isCore1 ? <span>1</span> : null}
                </span>
                <span className={`${isPc ? 'text-fg' : 'text-fg-muted'} w-16 shrink-0`}>
                  {fmtHex32(r.pa)}
                </span>
                <span className={`${isPc ? 'text-fg' : 'text-fg-muted'} w-20 shrink-0`}>
                  {r.word.toString(16).padStart(8, '0')}
                </span>
                <span className="flex-1">{r.mnem}</span>
              </div>
            )
          })}
        </div>
      </div>
    </Card>
  )
}

function ControlBar({
  aic,
  block,
  cores,
  cpu,
  onRefresh,
  onReset,
  onRunToggle,
  onStep,
  running,
  sysInfo,
  totalCoreSteps,
  uartBytes,
}: {
  aic: AicState
  block: BlockState
  cores: CoreState[]
  cpu: Cpu
  onRefresh: (cpu: Cpu) => void
  onReset: () => void
  onRunToggle: () => void
  onStep: () => void
  running: boolean
  sysInfo: SystemInfo
  totalCoreSteps: bigint
  uartBytes: number
}) {
  return (
    <div className="space-y-4">
      {/* Buttons get their own row, all 3 use the GDS Button primitive with
          visible borders/fills (no "highlight on click only" look). */}
      <div className="flex items-center gap-2">
        <Button onClick={onStep} size="sm" variant="primary">
          Step both
        </Button>
        <Button onClick={onRunToggle} size="sm" variant="secondary">
          {running ? 'Pause' : 'Run'}
        </Button>
        <Button onClick={onReset} size="sm" variant="secondary">
          Reset
        </Button>
      </div>
      <Card padding="none">
        <div className="grid grid-cols-1 gap-3 p-3 lg:grid-cols-2">
          <ControlBarDisk block={block} cpu={cpu} onRefresh={onRefresh} />
          <SystemInfoStrip
            aic={aic}
            block={block}
            cores={cores}
            info={sysInfo}
            totalCoreSteps={totalCoreSteps}
            uartBytes={uartBytes}
          />
        </div>
      </Card>
    </div>
  )
}

function SystemInfoStrip({
  aic,
  block,
  cores,
  info,
  totalCoreSteps,
  uartBytes,
}: {
  aic: AicState
  block: BlockState
  cores: CoreState[]
  info: SystemInfo
  totalCoreSteps: bigint
  uartBytes: number
}) {
  const aicPending = aic.pending.reduce((a, b) => a | b, 0)
  const stats: { label: string; value: string; emphasize?: boolean }[] = [
    { label: 'system steps', value: info.systemSteps.toString() },
    { label: 'retired', value: totalCoreSteps.toString() },
    { label: 'timer period', value: info.timerPeriod.toString() },
    {
      label: 'next IRQ',
      value: info.timerRemaining.toString(),
      emphasize: info.timerRemaining === 0n,
    },
    { label: 'timer ticks', value: info.timerTicks.toString() },
    { label: 'aic acks', value: aic.total_acks.toString() },
    { label: 'aic pending', value: '0x' + aicPending.toString(16).padStart(2, '0') },
    { label: 'uart bytes', value: uartBytes.toString() },
    { label: 'blk reads', value: block.total_reads.toString() },
    { label: 'core 0 pc', value: fmtHex32(Number(cores[0]?.pc ?? 0n)) },
    { label: 'core 1 pc', value: fmtHex32(Number(cores[1]?.pc ?? 0n)) },
    { label: 'cores el', value: `${cores[0]?.current_el ?? 0} / ${cores[1]?.current_el ?? 0}` },
  ]
  return (
    <div className="grid grid-cols-3 gap-2 sm:grid-cols-6">
      {stats.map((s) => (
        <Stat emphasize={s.emphasize} key={s.label} label={s.label} value={s.value} />
      ))}
    </div>
  )
}

function Stat({ emphasize, label, value }: { emphasize?: boolean; label: string; value: string }) {
  return (
    <div className="border-border bg-bg-secondary flex flex-col rounded border px-2 py-1">
      <span className="text-fg-muted type-small tracking-wider uppercase">{label}</span>
      <span
        className={`mono-data type-base truncate ${emphasize ? 'text-warning' : 'text-fg'}`}
        title={value}
      >
        {value}
      </span>
    </div>
  )
}

function ControlBarDisk({
  block,
  cpu,
  onRefresh,
}: {
  block: BlockState
  cpu: Cpu
  onRefresh: (cpu: Cpu) => void
}) {
  const initial = useMemo(() => {
    let end = 64
    for (let i = 0; i < 64; i++) {
      if (block.disk[i] === 0) {
        end = i
        break
      }
    }
    const u8 = new Uint8Array(block.disk.slice(0, end))
    return new TextDecoder('utf-8', { fatal: false }).decode(u8)
  }, [block.disk])
  const apply = (next: string) => {
    cpu.set_disk_text(next)
    onRefresh(cpu)
  }
  return (
    <label className="flex flex-1 flex-col gap-1">
      <span className="text-fg-muted type-small">
        disk sector 0 — task B prints this ({initial.length}/64 bytes)
      </span>
      <textarea
        className="mono-data border-border bg-bg-tertiary text-fg focus:border-accent type-base min-h-16 w-full resize-y rounded border px-2 py-1 outline-none"
        defaultValue={initial}
        key={initial}
        maxLength={64}
        onChange={(e) => apply(e.target.value)}
        rows={3}
        spellCheck={false}
      />
    </label>
  )
}

function DaifChip({ daif }: { daif: number }) {
  // Internal layout: bit 3=D, bit 2=A, bit 1=I, bit 0=F. 1 = masked.
  const bits = [
    { name: 'D', set: (daif & 0b1000) !== 0 },
    { name: 'A', set: (daif & 0b0100) !== 0 },
    { name: 'I', set: (daif & 0b0010) !== 0 },
    { name: 'F', set: (daif & 0b0001) !== 0 },
  ]
  return (
    <span
      className="border-border bg-bg-secondary type-base inline-flex items-center gap-1 rounded border px-2 py-1 tracking-wider"
      title="PSTATE.DAIF — 1 = masked, 0 = enabled"
    >
      {bits.map((b) => (
        <span className={b.set ? 'text-fg-muted' : 'text-success font-semibold'} key={b.name}>
          {b.name}
        </span>
      ))}
    </span>
  )
}

function CoreChip({ core }: { core: CoreState }) {
  // Use GDS Badge with the palette tokens so the colours are theme-aware.
  // Each core gets its own palette index (core 0 = palette-0, core 1 =
  // palette-1) so they're visually distinct without hardcoding hue.
  const variant = core.id === 0 ? 'palette-0' : 'palette-1'
  return (
    <Badge
      className="type-base px-2 py-1"
      variant={variant}
      title={`MPIDR ${fmtHex64(core.mpidr)}`}
    >
      <span className="font-semibold">core{core.id}</span>
      <span className="opacity-70">{core.kind}</span>
      <span className="font-semibold">EL{core.current_el}</span>
    </Badge>
  )
}

function CoreColumn({ core, onStep }: { core: CoreState; onStep: () => void }) {
  return (
    <Card padding="none">
      <div className="space-y-3 p-3">
        <div className="flex items-center justify-between">
          <div className="flex flex-wrap items-center gap-2">
            <CoreChip core={core} />
            <DaifChip daif={core.daif} />
            {(() => {
              const label = inferTaskLabel(core.pc)
              return label ? <Badge className="type-base px-2 py-1">{label}</Badge> : null
            })()}
            {core.wfi_halted && (
              <Badge className="type-base px-2 py-1" variant="info">
                WFI · sleeping
              </Badge>
            )}
            {core.halted && (
              <Badge
                className="type-base px-2 py-1"
                variant={core.last_trap ? 'danger' : 'success'}
              >
                {core.last_trap ? 'TRAP' : 'HALTED'}
              </Badge>
            )}
          </div>
          <div className="flex items-center gap-2">
            <span className="text-fg-muted type-small">steps {core.steps.toString()}</span>
            <GlassButton onClick={onStep} size="sm">
              Step
            </GlassButton>
          </div>
        </div>
        {core.last_trap && (
          <div className="border-danger/40 bg-danger/10 text-danger type-small rounded border px-2 py-1">
            {core.last_trap}
          </div>
        )}
        <RegistersSection core={core} />
        <ExceptionSection core={core} />
      </div>
    </Card>
  )
}

function RegistersSection({ core }: { core: CoreState }) {
  return (
    <div className="border-border bg-bg-secondary rounded border p-3">
      <div className="text-fg-muted type-base mb-2 font-semibold tracking-wider uppercase">
        Registers
      </div>
      <div className="type-base grid grid-cols-2 gap-x-4 gap-y-1">
        {REG_LABELS.map((label, i) => (
          <RegRow key={label} label={label} value={core.x[i]} />
        ))}
        <RegRow label="SP" value={core.sp} />
        <RegRow highlight label="PC" value={core.pc} />
      </div>
      <div className="text-fg-muted type-small mt-3">
        NZCV: <span>{fmtHex32(core.nzcv)}</span> · MPIDR_EL1: <span>{fmtHex64(core.mpidr)}</span>
      </div>
    </div>
  )
}

function ExceptionSection({ core }: { core: CoreState }) {
  // ESR_EL1=0 with current_el=1 typically means we entered via IRQ (no syndrome).
  const inIrq = core.current_el === 1 && core.esr_el1 === 0n && core.elr_el1 !== 0n
  const active = inIrq || core.esr_el1 !== 0n
  return (
    <div className="border-border bg-bg-secondary space-y-2 rounded border p-3">
      <div className="text-fg-muted type-base flex items-center gap-2 font-semibold tracking-wider uppercase">
        <span
          aria-label={active ? 'handling exception' : 'idle'}
          className={`inline-block h-1.5 w-1.5 rounded-full transition-colors ${
            active ? 'bg-emerald-400' : 'bg-fg-muted/40'
          }`}
        />
        Exception state
      </div>
      <div className="type-base grid gap-x-4 gap-y-1 sm:grid-cols-2">
        <div className="text-fg-muted type-base col-span-full tracking-wider uppercase">EL2</div>
        <RegRow label="VBAR_EL2" value={core.vbar_el2} />
        <RegRow label="ELR_EL2" value={core.elr_el2} />
        <RegRow label="SPSR_EL2" value={core.spsr_el2} />
        <RegRow label="ESR_EL2" value={core.esr_el2} />
        <div className="text-fg-muted type-base col-span-full mt-1 tracking-wider uppercase">
          EL1
        </div>
        <RegRow label="VBAR_EL1" value={core.vbar_el1} />
        <RegRow label="ELR_EL1" value={core.elr_el1} />
        <RegRow label="SPSR_EL1" value={core.spsr_el1} />
        <RegRow label="ESR_EL1" value={core.esr_el1} />
      </div>
    </div>
  )
}

function RegRow({
  highlight,
  label,
  value,
}: {
  highlight?: boolean
  label: string
  value: bigint
}) {
  return (
    <div className={`flex justify-between gap-2 ${highlight ? 'text-accent' : ''}`}>
      <span className="text-fg-muted">{label}</span>
      <span className={`mono-data ${value === 0n ? 'text-fg-muted' : 'text-fg'}`}>
        {fmtHex64(value)}
      </span>
    </div>
  )
}

function OutputPanel({ output }: { output: string }) {
  // Auto-scroll the box to the bottom whenever new bytes arrive, so the
  // user always sees the latest line even when the visible window is just
  // one line tall.
  const ref = useRef<HTMLPreElement>(null)
  useEffect(() => {
    if (ref.current) ref.current.scrollTop = ref.current.scrollHeight
  }, [output])
  return (
    <Card className="h-full" padding="none">
      <div className="flex h-full flex-col p-4">
        <div className="text-fg-muted type-small mb-3 font-semibold tracking-wider uppercase">
          UART Output (PA 0x1000) — shared
        </div>
        <pre
          ref={ref}
          className="text-fg type-base min-h-[1.45em] flex-1 overflow-y-auto leading-[1.45] whitespace-pre-wrap"
        >
          {output || <span className="text-fg-muted">(no output yet)</span>}
        </pre>
      </div>
    </Card>
  )
}

function MemoryPanel({ base, bytes, pcs }: { base: number; bytes: Uint8Array; pcs: number[] }) {
  const rows: { addr: number; bytes: Uint8Array }[] = []
  for (let off = 0; off < bytes.length; off += 16) {
    rows.push({ addr: base + off, bytes: bytes.slice(off, off + 16) })
  }
  return (
    <Card className="h-full" padding="none">
      <div className="flex h-full flex-col p-4">
        <div className="text-fg-muted mb-3 flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            Memory (around core 0 PC) — shared
          </span>
          <span className="type-small">
            base {fmtHex32(base)} · pc {pcs.map((pc, i) => `c${i}=${fmtHex32(pc)}`).join(' · ')}
          </span>
        </div>
        <div className="mono-data type-small min-h-0 flex-1 overflow-auto">
          {rows.map((r) => (
            <MemoryRow key={r.addr} addr={r.addr} bytes={r.bytes} pcs={pcs} />
          ))}
        </div>
      </div>
    </Card>
  )
}

function MemoryRow({ addr, bytes, pcs }: { addr: number; bytes: Uint8Array; pcs: number[] }) {
  // Color per core's PC: core 0 = accent (blue), core 1 = a different highlight.
  const cells: React.ReactNode[] = []
  for (let i = 0; i < bytes.length; i++) {
    const byteAddr = addr + i
    let cls = 'text-fg'
    if (pcs[0] !== undefined && byteAddr >= pcs[0] && byteAddr < pcs[0] + 4) {
      cls = 'text-accent bg-accent/10'
    } else if (pcs[1] !== undefined && byteAddr >= pcs[1] && byteAddr < pcs[1] + 4) {
      cls = 'bg-violet-500/15 text-violet-700 dark:text-violet-300'
    }
    cells.push(
      <span className={cls} key={i}>
        {bytes[i].toString(16).padStart(2, '0')}
      </span>
    )
    if (i !== bytes.length - 1) cells.push(<span key={`s${i}`}> </span>)
  }
  let ascii = ''
  for (let i = 0; i < bytes.length; i++) {
    const c = bytes[i]
    ascii += c >= 0x20 && c < 0x7f ? String.fromCharCode(c) : '.'
  }
  return (
    <div className="flex gap-4 leading-6">
      <span className="text-fg-muted">{fmtHex32(addr)}</span>
      <span className="flex-1">{cells}</span>
      <span className="text-fg-muted">{ascii}</span>
    </div>
  )
}

function MmuPanel({
  cores,
  onCoreChange,
  onVaChange,
  selectedCoreIdx,
  trace,
  vaText,
}: {
  cores: CoreState[]
  onCoreChange: (idx: number) => void
  onVaChange: (v: string) => void
  selectedCoreIdx: number
  trace: TranslationResult | null
  vaText: string
}) {
  const sel = cores[selectedCoreIdx]
  const t0sz = Number(sel.tcr_el1 & 0x3fn)
  const vaBits = t0sz > 0 ? 64 - t0sz : 0
  const mmuOn = (sel.sctlr_el1 & 1n) !== 0n

  return (
    <Card padding="none">
      <div className="space-y-4 p-4">
        <div className="text-fg-muted flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            MMU · Stage-1 Translation
          </span>
          <Badge variant={mmuOn ? 'success' : undefined}>
            SCTLR_EL{1}.M={mmuOn ? '1' : '0'}
          </Badge>
        </div>

        <div className="type-small flex items-center gap-2">
          <span className="text-fg-muted">walk using</span>
          {cores.map((c) => (
            <button
              className={`type-small rounded border px-2 py-0.5 ${
                c.id === selectedCoreIdx
                  ? 'border-accent bg-accent/10 text-accent'
                  : 'border-border text-fg-muted hover:bg-bg-tertiary'
              }`}
              key={c.id}
              onClick={() => onCoreChange(c.id)}
              type="button"
            >
              core{c.id} ({c.kind})
            </button>
          ))}
        </div>

        <div className="type-small grid gap-x-6 gap-y-1 sm:grid-cols-3">
          <RegRow label="TTBR0_EL1" value={sel.ttbr0_el1} />
          <RegRow label="TCR_EL1" value={sel.tcr_el1} />
          <RegRow label="SCTLR_EL1" value={sel.sctlr_el1} />
        </div>
        <div className="text-fg-muted type-small">
          T0SZ={t0sz} · VA={vaBits} bits · 4 KiB granule · start level{' '}
          {trace && trace.steps.length > 0 ? trace.steps[0].level : '?'}
        </div>

        <div
          className={`type-small rounded border px-3 py-1.5 ${
            mmuOn ? 'border-success/40 bg-success/10 text-success' : 'border-border text-fg-muted'
          }`}
        >
          {mmuOn
            ? 'Translation active — every fetch and LDR/STR runs through this walk.'
            : 'Translation is a query only — fetches and LDR/STR bypass the MMU until SCTLR_EL1.M=1.'}
        </div>

        <div className="flex items-center gap-2">
          <label className="text-fg-muted type-small" htmlFor="va-input">
            translate VA
          </label>
          <input
            className="border-border bg-bg-secondary text-fg focus:border-accent type-small w-40 rounded border px-2 py-1 outline-none"
            id="va-input"
            onChange={(e) => onVaChange(e.target.value)}
            placeholder="0x4000"
            spellCheck={false}
            value={vaText}
          />
          <span className="text-fg-muted type-small">
            try 0x4000, 0x1000, 0x4800, 0x4C00, 0x2000
          </span>
        </div>

        {trace && <WalkDisplay trace={trace} />}
      </div>
    </Card>
  )
}

function WalkDisplay({ trace }: { trace: TranslationResult }) {
  if (trace.steps.length === 0 && trace.fault) {
    // "MMU not configured" before the kernel has set up TTBR/TCR is the
    // expected boot state, not a failure. Render it as a muted note rather
    // than a red error so it doesn't read as a bug at step 0.
    const isPreInit = trace.fault.startsWith('MMU not configured')
    return (
      <div
        className={`type-small rounded border px-3 py-2 ${
          isPreInit ? 'border-border text-fg-muted' : 'border-danger/40 bg-danger/10 text-danger'
        }`}
      >
        {isPreInit
          ? 'MMU not configured yet — kernel will set TTBR0_EL1 + TCR_EL1 during boot.'
          : trace.fault}
      </div>
    )
  }
  return (
    <div className="type-small space-y-2">
      {trace.steps.map((s, i) => (
        <WalkStepRow key={i} step={s} />
      ))}
      <div className="border-border mt-2 flex items-center justify-between border-t pt-2">
        <span className="text-fg-muted">VA {fmtHex64(trace.va)}</span>
        {trace.pa !== null ? (
          <span className="text-accent">→ PA {fmtHex64(trace.pa)}</span>
        ) : (
          <span className="text-danger">{trace.fault ?? 'no PA'}</span>
        )}
      </div>
    </div>
  )
}

function WalkStepRow({ step }: { step: WalkStep }) {
  const tone =
    step.outcome.kind === 'Invalid' || step.outcome.kind === 'Fault' ? 'text-danger' : 'text-fg'
  return (
    <div className={`border-border rounded border px-3 py-2 ${tone}`}>
      <div className="flex flex-wrap items-baseline gap-x-4 gap-y-1">
        <span className="text-accent font-semibold">L{step.level}</span>
        <span className="text-fg-muted">
          table {fmtHex64(step.table_addr)} · idx {step.index}
        </span>
        <span className="text-fg-muted">
          entry@{fmtHex64(step.entry_addr)} = {fmtHex64(step.descriptor)}
        </span>
      </div>
      <div className="text-fg-muted type-small mt-1">
        <OutcomeText outcome={step.outcome} />
      </div>
    </div>
  )
}

function OutcomeText({ outcome }: { outcome: WalkOutcome }) {
  switch (outcome.kind) {
    case 'Table':
      return <>→ next table @ {fmtHex64(outcome.next_table)}</>
    case 'Page':
      return (
        <>
          → page PA {fmtHex64(outcome.pa)} · AF={outcome.attrs.af ? 1 : 0} · AP=
          {outcome.attrs.ap}
        </>
      )
    case 'Block':
      return (
        <>
          → block PA {fmtHex64(outcome.pa)} (span {fmtHex64(outcome.span)}) · AF=
          {outcome.attrs.af ? 1 : 0}
        </>
      )
    case 'Invalid':
      return <>invalid descriptor (V=0)</>
    case 'Fault':
      return <>fault: {outcome.reason}</>
  }
}
