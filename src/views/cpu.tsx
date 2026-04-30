import { Badge, GlassButton, GlassCard } from '@goliapkg/gds'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import init, { Cpu, disassemble } from 'aarch64-sim'

interface CoreState {
  id: number
  kind: string
  mpidr: bigint
  x: bigint[]
  sp: bigint
  pc: bigint
  nzcv: number
  halted: boolean
  last_trap: string | null
  steps: bigint
  current_el: number
  daif: number
  wfi_halted: boolean
  ttbr0_el1: bigint
  tcr_el1: bigint
  sctlr_el1: bigint
  vbar_el1: bigint
  elr_el1: bigint
  spsr_el1: bigint
  esr_el1: bigint
  vbar_el2: bigint
  elr_el2: bigint
  spsr_el2: bigint
  esr_el2: bigint
}

interface AicState {
  pending: number[]
  total_acks: bigint
}

type SimEventKind = 'store' | 'timer' | 'disk_read' | 'irq_taken' | 'svc' | 'eret'

interface SimEvent {
  id: number
  kind: SimEventKind
  source: NodeId
  target: NodeId
  ts: number
}

type NodeId = 'core0' | 'core1' | 'aic' | 'uart' | 'block' | 'ram'

interface PrevSnapshot {
  cores: { pc: bigint; current_el: number; wfi_halted: boolean }[]
  outputLen: number
  ticks: bigint
  totalReads: bigint
}

interface BlockState {
  sector: bigint
  buf_addr: bigint
  last_command: bigint
  status: bigint
  total_reads: bigint
  total_writes: bigint
  disk: number[]
}

interface TaskSave {
  x0: bigint
  x1: bigint
  x2: bigint
  x3: bigint
}

interface SystemInfo {
  systemSteps: bigint
  timerPeriod: bigint
  timerRemaining: bigint
  timerTicks: bigint
}

interface PageAttrs {
  af: boolean
  ap: number
  attr_idx: number
  sh: number
}

type WalkOutcome =
  | { kind: 'Table'; next_table: bigint }
  | { kind: 'Page'; pa: bigint; attrs: PageAttrs }
  | { kind: 'Block'; pa: bigint; attrs: PageAttrs; span: bigint }
  | { kind: 'Invalid' }
  | { kind: 'Fault'; reason: string }

interface WalkStep {
  level: number
  table_addr: bigint
  index: number
  entry_addr: bigint
  descriptor: bigint
  outcome: WalkOutcome
}

interface TranslationResult {
  va: bigint
  steps: WalkStep[]
  pa: bigint | null
  fault: string | null
  mmu_enabled: boolean
}

const REG_LABELS = Array.from({ length: 31 }, (_, i) => `X${i}`)
const MEMORY_VIEW_BYTES = 256
const RUN_BURST = 2

function fmtHex64(v: bigint): string {
  return '0x' + v.toString(16).padStart(16, '0')
}

function fmtHex32(v: number): string {
  return '0x' + (v >>> 0).toString(16).padStart(8, '0')
}

interface CoreSlot {
  entry: bigint
  savePtr: bigint
  save0: TaskSave
  save1: TaskSave
}

function parseCoreSlot(bytes: Uint8Array): CoreSlot {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const u64 = (off: number) => view.getBigUint64(off, true)
  return {
    entry: u64(0),
    savePtr: u64(8),
    save0: { x0: u64(0x10), x1: u64(0x18), x2: u64(0x20), x3: u64(0x28) },
    save1: { x0: u64(0x30), x1: u64(0x38), x2: u64(0x40), x3: u64(0x48) },
  }
}

function parseHex(text: string): bigint | null {
  const trimmed = text.trim()
  if (trimmed === '') return null
  try {
    return BigInt(trimmed.startsWith('0x') || trimmed.startsWith('0X') ? trimmed : '0x' + trimmed)
  } catch {
    return null
  }
}

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
    setRunning((r) => !r)
  }, [])

  useEffect(() => {
    if (!running || !cpu) return
    const tick = () => {
      const states = cpu.state() as CoreState[]
      if (states.every((s) => s.halted)) {
        setRunning(false)
        return
      }
      cpu.run(RUN_BURST)
      refresh(cpu)
      const after = cpu.state() as CoreState[]
      if (after.every((s) => s.halted)) {
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
    <div className="space-y-6">
      <header className="space-y-2">
        <div className="flex flex-wrap items-center gap-3">
          <h1
            className="text-fg text-2xl font-bold"
            style={{ textShadow: '0 0 20px var(--gds-accent, #3b82f6)' }}
          >
            AArch64 CPU
          </h1>
          <Badge color="info">v0.15</Badge>
          {cores.map((c) => (
            <CoreChip core={c} key={c.id} />
          ))}
          {allHalted ? (
            <Badge color={anyTrap ? 'danger' : 'success'}>{anyTrap ? 'TRAP' : 'ALL HALTED'}</Badge>
          ) : (
            <Badge>RUNNING</Badge>
          )}
        </div>
        <p className="text-fg-muted type-small max-w-2xl">
          UI overhaul: live <strong>SCI-FI core monitors</strong> on the right rail (registers, EL,
          peripheral links, activity blip), a <strong>disassembly panel</strong> centered on PC (we
          built a small ARM disassembler in the WASM crate), and an{' '}
          <strong>editable disk sector 0</strong> at the bottom — type whatever you want and task B
          will print it.
        </p>
      </header>

      <ControlBar
        block={block}
        cpu={cpu}
        onRefresh={refresh}
        onReset={onReset}
        onRunToggle={onRunToggle}
        onStep={onStep}
        running={running}
        sysInfo={sysInfo}
        totalCoreSteps={cores.reduce((a, c) => a + c.steps, 0n)}
      />

      {anyTrap && anyTrap.last_trap && (
        <div className="border-danger/40 bg-danger/10 text-danger type-small rounded border px-3 py-2">
          core {anyTrap.id}: {anyTrap.last_trap}
        </div>
      )}

      <OutputPanel output={output} />

      <div className="grid min-w-0 gap-4 xl:grid-cols-3">
        <DisassemblyPanel cpu={cpu} cores={cores} />
        <MemoryPanel base={memBaseAddr} bytes={memory} pcs={corePcs} />
        <SystemDiagram
          aic={aic}
          block={block}
          cores={cores}
          events={events}
          output={output}
          slots={coreSlots}
        />
      </div>

      <MmuPanel
        cores={cores}
        onCoreChange={setTranslateCoreIdx}
        onVaChange={setVaText}
        selectedCoreIdx={translateCoreIdx}
        trace={trace}
        vaText={vaText}
      />

      <div className="grid gap-4 lg:grid-cols-3">
        <AicPanel aic={aic} />
        <BlockPanel block={block} />
        <SavePanel slots={coreSlots} />
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        {cores.map((c) => (
          <CoreColumn core={c} key={c.id} onStep={() => onStepCore(c.id)} />
        ))}
      </div>
    </div>
  )
}

const IRQ_NAMES = ['TIMER', 'IPI']
const TASK_A_ENTRY = 0x4d00
const TASK_B_ENTRY = 0x4e00

function inferTaskLabel(pc: bigint): string | null {
  const p = Number(pc)
  if (p >= TASK_A_ENTRY && p < TASK_A_ENTRY + 0x20) return 'task A'
  if (p >= TASK_B_ENTRY && p < TASK_B_ENTRY + 0x20) return 'task B'
  if (p >= 0x4880 && p < 0x4900) return 'IRQ handler (sched)'
  if (p >= 0x4800 && p < 0x4880) return 'sync handler'
  if (p >= 0x4000 && p < 0x4400) return 'kernel boot'
  return null
}

const SECTOR_SIZE = 64

function BlockPanel({ block }: { block: BlockState }) {
  const [sector, setSector] = useState(0)
  const numSectors = Math.floor(block.disk.length / SECTOR_SIZE)
  const slice = new Uint8Array(block.disk.slice(sector * SECTOR_SIZE, (sector + 1) * SECTOR_SIZE))
  const statusLabel =
    block.status === 0n
      ? 'IDLE'
      : block.status === 1n
        ? 'OK'
        : block.status === 2n
          ? 'FAULT'
          : `0x${block.status.toString(16)}`
  return (
    <GlassCard>
      <div className="space-y-3 p-4">
        <div className="text-fg-muted flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            Block device · disk image (8 × 64-byte sectors @ MMIO 0x3000)
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
        <div className="flex flex-wrap items-center gap-1">
          <span className="text-fg-muted type-small mr-2">view sector</span>
          {Array.from({ length: numSectors }, (_, i) => (
            <button
              className={`type-small rounded border px-2 py-0.5 ${
                i === sector
                  ? 'border-accent bg-accent/10 text-accent'
                  : 'border-border text-fg-muted hover:bg-bg-tertiary'
              }`}
              key={i}
              onClick={() => setSector(i)}
              type="button"
            >
              {i}
            </button>
          ))}
        </div>
        <DiskHexRow bytes={slice} sector={sector} />
      </div>
    </GlassCard>
  )
}

function DiskHexRow({ bytes, sector }: { bytes: Uint8Array; sector: number }) {
  const rows: Uint8Array[] = []
  for (let i = 0; i < bytes.length; i += 16) {
    rows.push(bytes.slice(i, i + 16))
  }
  const baseAddr = sector * SECTOR_SIZE
  return (
    <div className="type-small overflow-x-auto">
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
    <GlassCard>
      <div className="space-y-2 p-4">
        <div className="text-fg-muted type-small font-semibold tracking-wider uppercase">
          Per-core scheduler slots — entry + save_ptr + 2 save areas each
        </div>
        <div className="grid gap-3 lg:grid-cols-2">
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
    </GlassCard>
  )
}

function CoreSlotCard({ base, coreId, slot }: { base: number; coreId: number; slot: CoreSlot }) {
  const taskLabel = slot.entry === 0x4d00n ? 'task A' : slot.entry === 0x4e00n ? 'task B' : '?'
  const activeIdx =
    slot.savePtr === BigInt(base + 0x10) ? 0 : slot.savePtr === BigInt(base + 0x30) ? 1 : null
  return (
    <div className="border-border bg-bg/40 type-small space-y-2 rounded border px-3 py-2">
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
        <SaveAreaCard
          active={activeIdx === 0}
          label={`save 0 @ 0x${(base + 0x10).toString(16)}`}
          save={slot.save0}
        />
        <SaveAreaCard
          active={activeIdx === 1}
          label={`save 1 @ 0x${(base + 0x30).toString(16)}`}
          save={slot.save1}
        />
      </div>
    </div>
  )
}

function SaveAreaCard({ active, label, save }: { active: boolean; label: string; save: TaskSave }) {
  const x0Char = Number(save.x0 & 0xffn)
  const ascii = x0Char >= 0x20 && x0Char < 0x7f ? `'${String.fromCharCode(x0Char)}'` : ''
  return (
    <div
      className={`rounded border px-2 py-1 ${
        active ? 'border-accent/60 bg-accent/5' : 'border-border'
      }`}
    >
      <div
        className={`type-small mb-1 flex items-center justify-between tracking-wider uppercase ${
          active ? 'text-accent' : 'text-fg-muted'
        }`}
      >
        <span>{label}</span>
        {active && <span>active</span>}
      </div>
      <RegRow label={`X0 ${ascii}`} value={save.x0} />
      <RegRow label="X1" value={save.x1} />
      <RegRow label="X2" value={save.x2} />
      <RegRow highlight label="X3" value={save.x3} />
    </div>
  )
}

function AicPanel({ aic }: { aic: AicState }) {
  return (
    <GlassCard>
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
            <div className="border-border bg-bg/40 type-small rounded border px-3 py-2" key={i}>
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
                          ? 'rounded border border-amber-500/40 bg-amber-500/15 px-1.5 py-0.5 text-amber-300'
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
    </GlassCard>
  )
}

// === System architecture diagram ============================================
// Dense SoC-style block diagram: two cores up top with their internal
// sub-blocks (regs / exception / MMU / DAIF) and pin stubs, four lanes of
// system bus (DATA / ADDR / IRQ / CTRL), peripheral chips (AIC, UART, BLK,
// DISK) hanging off the bus, and a 4×4 RAM grid showing every named region
// at the bottom. SimEvents render as coloured packets flying along the
// matching bus lane.

const SVG_W = 440
const SVG_H = 480
const BUS_Y = 188

const NODE_POS: Record<NodeId, { x: number; y: number }> = {
  core0: { x: 110, y: 78 },
  core1: { x: 330, y: 78 },
  aic: { x: 70, y: 280 },
  uart: { x: 190, y: 280 },
  block: { x: 310, y: 280 },
  ram: { x: 220, y: 410 },
}

function SystemDiagram({
  aic,
  block,
  cores,
  events,
  output,
  slots,
}: {
  aic: AicState
  block: BlockState
  cores: CoreState[]
  events: SimEvent[]
  output: string
  slots: CoreSlot[]
}) {
  const ramRegions = useMemo(
    () =>
      [
        { addr: 0x1000, label: 'UART' },
        { addr: 0x2000, label: 'AIC' },
        { addr: 0x3000, label: 'BLK' },
        { addr: 0x4000, label: 'kernel/tasks' },
        { addr: 0x4f00, label: 'core 0 slots' },
        { addr: 0x5000, label: 'core 1 slots' },
        { addr: 0x6000, label: 'disk buf' },
        { addr: 0x8000, label: 'page tables' },
      ] as const,
    []
  )
  return (
    <div className="border-border bg-bg/60 relative rounded-xl border p-3">
      <div className="text-fg-muted mb-2 flex items-center justify-between">
        <span className="type-small font-semibold tracking-wider uppercase">
          System layout · live event flow
        </span>
        <span className="type-small">{events.length > 0 ? `${events.length} active` : 'idle'}</span>
      </div>
      <svg
        className="block w-full"
        viewBox={`0 0 ${SVG_W} ${SVG_H}`}
        xmlns="http://www.w3.org/2000/svg"
      >
        {/* Static system bus */}
        <line
          stroke="rgb(148 163 184 / 0.35)"
          strokeWidth="1.5"
          x1="20"
          x2={SVG_W - 20}
          y1={BUS_Y}
          y2={BUS_Y}
        />
        <text
          fill="rgb(148 163 184 / 0.6)"
          fontFamily="'Roboto Flex'"
          fontSize="9"
          x={SVG_W - 24}
          y={BUS_Y - 6}
          textAnchor="end"
        >
          system bus
        </text>

        {/* Stub lines from cores down to bus, from bus up/down to peripherals */}
        {(['core0', 'core1'] as const).map((id) => (
          <line
            key={id}
            stroke="rgb(148 163 184 / 0.3)"
            strokeWidth="1"
            x1={NODE_POS[id].x}
            x2={NODE_POS[id].x}
            y1={NODE_POS[id].y + 64}
            y2={BUS_Y}
          />
        ))}
        {(['aic', 'uart', 'block'] as const).map((id) => (
          <line
            key={id}
            stroke="rgb(148 163 184 / 0.3)"
            strokeWidth="1"
            x1={NODE_POS[id].x}
            x2={NODE_POS[id].x}
            y1={BUS_Y}
            y2={NODE_POS[id].y - 32}
          />
        ))}
        {/* RAM bus -- lower half */}
        <line
          stroke="rgb(148 163 184 / 0.3)"
          strokeWidth="1"
          x1={NODE_POS.ram.x}
          x2={NODE_POS.ram.x}
          y1={NODE_POS.block.y + 36}
          y2={NODE_POS.ram.y - 14}
        />

        {/* Cores */}
        <CoreBox core={cores[0]} pos={NODE_POS.core0} slot={slots[0]} />
        <CoreBox core={cores[1]} pos={NODE_POS.core1} slot={slots[1]} />

        {/* Peripherals */}
        <AicBox aic={aic} pos={NODE_POS.aic} />
        <UartBox output={output} pos={NODE_POS.uart} />
        <BlockBox block={block} pos={NODE_POS.block} />
        <RamBox pos={NODE_POS.ram} regions={ramRegions} />
      </svg>

      {/* Animated event packets (HTML overlay positioned over the SVG) */}
      <div
        className="pointer-events-none absolute inset-0"
        style={{ paddingTop: 28, paddingLeft: 12, paddingRight: 12 }}
      >
        <div className="relative h-full w-full">
          {events.map((e) => (
            <EventPacket event={e} key={e.id} />
          ))}
        </div>
      </div>
    </div>
  )
}

function CoreBox({
  core,
  pos,
  slot,
}: {
  core: CoreState
  pos: { x: number; y: number }
  slot: CoreSlot
}) {
  const accent = core.id === 0 ? '#22d3ee' : '#a78bfa'
  const taskLabel = slot.entry === 0x4d00n ? 'task A' : slot.entry === 0x4e00n ? 'task B' : '—'
  const stateLine = core.last_trap ? 'TRAP' : core.halted ? 'HALT' : core.wfi_halted ? 'WFI' : 'RUN'
  return (
    <g>
      <rect
        fill="rgb(15 23 42 / 0.9)"
        height="64"
        rx="4"
        stroke={accent}
        strokeOpacity="0.55"
        strokeWidth="1"
        width="160"
        x={pos.x - 80}
        y={pos.y}
      />
      <text
        fill={accent}
        fontFamily="'Roboto Flex'"
        fontSize="11"
        fontWeight="bold"
        x={pos.x - 72}
        y={pos.y + 14}
      >
        core {core.id}
      </text>
      <text
        fill="rgb(148 163 184 / 0.85)"
        fontFamily="'Roboto Flex'"
        fontSize="9"
        x={pos.x - 72}
        y={pos.y + 26}
      >
        {core.kind}
      </text>
      <text
        fill={accent}
        fontFamily="'Roboto Flex'"
        fontSize="9"
        x={pos.x + 72}
        y={pos.y + 14}
        textAnchor="end"
      >
        EL{core.current_el}
      </text>
      <text
        fill="rgb(241 245 249 / 0.95)"
        fontFamily="'Roboto Flex'"
        fontSize="9"
        x={pos.x + 72}
        y={pos.y + 26}
        textAnchor="end"
      >
        {stateLine}
      </text>
      {/* PC + task */}
      <text
        fill="rgb(241 245 249 / 0.95)"
        fontFamily="'Roboto Flex'"
        fontSize="11"
        x={pos.x - 72}
        y={pos.y + 44}
      >
        pc {fmtHex32(Number(core.pc))}
      </text>
      <text
        fill="rgb(148 163 184 / 0.85)"
        fontFamily="'Roboto Flex'"
        fontSize="9"
        x={pos.x - 72}
        y={pos.y + 56}
      >
        {taskLabel} · daif {core.daif.toString(16).padStart(1, '0')}
      </text>
    </g>
  )
}

function PeripheralBox({
  accent,
  lines,
  pos,
  title,
  width = 92,
}: {
  accent: string
  lines: string[]
  pos: { x: number; y: number }
  title: string
  width?: number
}) {
  const halfW = width / 2
  return (
    <g>
      <rect
        fill="rgb(15 23 42 / 0.85)"
        height="48"
        rx="3"
        stroke={accent}
        strokeOpacity="0.45"
        strokeWidth="1"
        width={width}
        x={pos.x - halfW}
        y={pos.y - 32}
      />
      <text
        fill={accent}
        fontFamily="'Roboto Flex'"
        fontSize="11"
        fontWeight="bold"
        x={pos.x}
        y={pos.y - 18}
        textAnchor="middle"
      >
        {title}
      </text>
      {lines.map((ln, i) => (
        <text
          fill="rgb(203 213 225 / 0.9)"
          fontFamily="'Roboto Flex'"
          fontSize="9"
          key={i}
          x={pos.x}
          y={pos.y - 4 + i * 10}
          textAnchor="middle"
        >
          {ln}
        </text>
      ))}
    </g>
  )
}

function AicBox({ aic, pos }: { aic: AicState; pos: { x: number; y: number } }) {
  const pendingMask = aic.pending.reduce((acc, p) => acc | p, 0)
  return (
    <PeripheralBox
      accent="#fbbf24"
      lines={[`pnd ${pendingMask.toString(16).padStart(2, '0')}`, `ack ${aic.total_acks}`]}
      pos={pos}
      title="AIC"
    />
  )
}

function UartBox({ output, pos }: { output: string; pos: { x: number; y: number } }) {
  const tail = output.slice(-12).replace(/\n/g, '↵')
  return (
    <PeripheralBox
      accent="#34d399"
      lines={[`bytes ${output.length}`, tail || '—']}
      pos={pos}
      title="UART"
    />
  )
}

function BlockBox({ block, pos }: { block: BlockState; pos: { x: number; y: number } }) {
  return (
    <PeripheralBox
      accent="#fb7185"
      lines={[`r ${block.total_reads} w ${block.total_writes}`, `sec ${block.sector}`]}
      pos={pos}
      title="BLK"
    />
  )
}

function RamBox({
  pos,
  regions,
}: {
  pos: { x: number; y: number }
  regions: readonly { addr: number; label: string }[]
}) {
  const W = 320
  const H = 60
  return (
    <g>
      <rect
        fill="rgb(15 23 42 / 0.85)"
        height={H}
        rx="3"
        stroke="rgb(96 165 250 / 0.45)"
        strokeWidth="1"
        width={W}
        x={pos.x - W / 2}
        y={pos.y - 14}
      />
      <text
        fill="#60a5fa"
        fontFamily="'Roboto Flex'"
        fontSize="11"
        fontWeight="bold"
        x={pos.x - W / 2 + 8}
        y={pos.y}
      >
        RAM · 64 KiB
      </text>
      {/* Region strip */}
      {regions.map((r, i) => {
        const cellW = (W - 16) / regions.length
        const x = pos.x - W / 2 + 8 + i * cellW
        return (
          <g key={r.addr}>
            <rect
              fill="rgb(96 165 250 / 0.08)"
              height="22"
              stroke="rgb(96 165 250 / 0.35)"
              strokeWidth="0.5"
              width={cellW}
              x={x}
              y={pos.y + 8}
            />
            <text
              fill="rgb(203 213 225 / 0.85)"
              fontFamily="'Roboto Flex'"
              fontSize="9"
              x={x + cellW / 2}
              y={pos.y + 17}
              textAnchor="middle"
            >
              {fmtHex32(r.addr).slice(2, 6)}
            </text>
            <text
              fill="rgb(148 163 184 / 0.7)"
              fontFamily="'Roboto Flex'"
              fontSize="9"
              x={x + cellW / 2}
              y={pos.y + 26}
              textAnchor="middle"
            >
              {r.label}
            </text>
          </g>
        )
      })}
    </g>
  )
}

function EventPacket({ event }: { event: SimEvent }) {
  // Translate SVG coordinates to overlay coordinates. Container fills the
  // SVG area exactly (we share width via the parent's viewBox aspect).
  const src = nodeAnchor(event.source, 'out')
  const dst = nodeAnchor(event.target, 'in')
  // Scale from SVG units (440×480) to the rendered container, which is the
  // same DOM box but in absolute pixels — we use percentages so it's
  // resolution-independent.
  const fromX = `${(src.x / SVG_W) * 100}%`
  const fromY = `${(src.y / SVG_H) * 100}%`
  const toX = `${((dst.x - src.x) / SVG_W) * 100}%`
  const toY = `${((dst.y - src.y) / SVG_H) * 100}%`
  const color = packetColor(event.kind)
  const delay = event.kind === 'timer' && event.target === 'core1' ? '30ms' : '0ms'
  return (
    <span
      className="absolute h-1.5 w-1.5 rounded-full"
      style={{
        left: fromX,
        top: fromY,
        background: color,
        boxShadow: `0 0 6px ${color}`,
        animation: 'packet-fly 700ms ease-in-out forwards',
        animationDelay: delay,
        // CSS variables consumed by the keyframes
        ['--packet-from-x' as string]: '0px',
        ['--packet-from-y' as string]: '0px',
        ['--packet-to-x' as string]: toX,
        ['--packet-to-y' as string]: toY,
      }}
    />
  )
}

function nodeAnchor(id: NodeId, dir: 'in' | 'out'): { x: number; y: number } {
  const base = NODE_POS[id]
  // For cores: anchor on bottom edge. For peripherals (incl. RAM): top edge.
  if (id === 'core0' || id === 'core1') {
    return { x: base.x, y: base.y + (dir === 'out' ? 64 : 64) }
  }
  if (id === 'ram') {
    return { x: base.x, y: base.y - 14 }
  }
  return { x: base.x, y: base.y - 32 }
}

function packetColor(kind: SimEventKind): string {
  switch (kind) {
    case 'store':
      return '#34d399' // green to UART
    case 'timer':
      return '#fbbf24' // amber from AIC
    case 'disk_read':
      return '#fb7185' // rose from Block
    case 'irq_taken':
      return '#fbbf24'
    case 'svc':
      return '#60a5fa'
    case 'eret':
      return '#a78bfa'
  }
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
    <GlassCard>
      <div className="space-y-2 p-4">
        <div className="text-fg-muted flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            Disassembly · around core 0 PC
          </span>
          <span className="type-small">
            range {fmtHex32(start)}–{fmtHex32(end - 1)}
          </span>
        </div>
        <div className="type-small overflow-x-auto">
          {rows.map((r) => {
            const isCore0 = r.pa === pc0
            const isCore1 = r.pa === pc1
            const cls = isCore0
              ? 'bg-cyan-500/10 text-cyan-200'
              : isCore1
                ? 'bg-violet-500/10 text-violet-200'
                : ''
            return (
              <div className={`flex gap-3 leading-6 ${cls}`} key={r.pa}>
                <span className="text-fg-muted w-12 shrink-0">
                  {isCore0 ? '►0' : isCore1 ? '►1' : '  '}
                </span>
                <span className="text-fg-muted w-16 shrink-0">{fmtHex32(r.pa)}</span>
                <span className="text-fg-muted w-20 shrink-0">
                  {r.word.toString(16).padStart(8, '0')}
                </span>
                <span className="flex-1">{r.mnem}</span>
              </div>
            )
          })}
        </div>
      </div>
    </GlassCard>
  )
}

function ControlBar({
  block,
  cpu,
  onRefresh,
  onReset,
  onRunToggle,
  onStep,
  running,
  sysInfo,
  totalCoreSteps,
}: {
  block: BlockState
  cpu: Cpu
  onRefresh: (cpu: Cpu) => void
  onReset: () => void
  onRunToggle: () => void
  onStep: () => void
  running: boolean
  sysInfo: SystemInfo
  totalCoreSteps: bigint
}) {
  return (
    <div className="border-border bg-bg/60 flex flex-wrap items-stretch gap-3 rounded-xl border p-3">
      <div className="flex items-center gap-2">
        <GlassButton onClick={onStep} size="sm" variant="accent">
          Step both
        </GlassButton>
        <GlassButton onClick={onRunToggle} size="sm">
          {running ? 'Pause' : 'Run'}
        </GlassButton>
        <GlassButton onClick={onReset} size="sm">
          Reset
        </GlassButton>
      </div>
      <div className="border-border/40 hidden border-r xl:block" />
      <SystemInfoStrip info={sysInfo} totalCoreSteps={totalCoreSteps} />
      <div className="border-border/40 hidden border-r xl:block" />
      <ControlBarDisk block={block} cpu={cpu} onRefresh={onRefresh} />
    </div>
  )
}

function SystemInfoStrip({ info, totalCoreSteps }: { info: SystemInfo; totalCoreSteps: bigint }) {
  return (
    <div className="type-small grid flex-1 grid-cols-3 gap-x-6 gap-y-0.5 sm:grid-cols-5">
      <Stat label="system steps" value={info.systemSteps.toString()} />
      <Stat label="retired" value={totalCoreSteps.toString()} />
      <Stat label="period" value={info.timerPeriod.toString()} />
      <Stat
        emphasize={info.timerRemaining === 0n}
        label="next IRQ"
        value={info.timerRemaining.toString()}
      />
      <Stat label="ticks" value={info.timerTicks.toString()} />
    </div>
  )
}

function Stat({ emphasize, label, value }: { emphasize?: boolean; label: string; value: string }) {
  return (
    <div className="flex flex-col">
      <span className="text-fg-muted type-small">{label}</span>
      <span className={emphasize ? 'type-base text-amber-400' : 'text-fg type-base'}>{value}</span>
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
      <input
        className="border-border bg-bg/40 text-fg focus:border-accent type-base w-full rounded border px-2 py-1 outline-none"
        defaultValue={initial}
        key={initial}
        maxLength={64}
        onChange={(e) => apply(e.target.value)}
        spellCheck={false}
        type="text"
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
      className="border-border bg-bg/40 type-small inline-flex items-center gap-1 rounded border px-1.5 py-0.5 tracking-wider"
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
  const elClass =
    core.current_el === 2
      ? 'bg-cyan-500/20 text-cyan-300 border-cyan-500/40'
      : core.current_el === 1
        ? 'bg-violet-500/20 text-violet-300 border-violet-500/40'
        : 'bg-neutral-500/20 text-neutral-300 border-neutral-500/40'
  return (
    <span
      className={`type-small inline-flex items-center gap-1.5 rounded border px-2 py-0.5 tracking-wider ${elClass}`}
      title={`MPIDR ${fmtHex64(core.mpidr)}`}
    >
      <span className="font-semibold">core{core.id}</span>
      <span className="opacity-70">{core.kind}</span>
      <span className="font-semibold">EL{core.current_el}</span>
    </span>
  )
}

function CoreColumn({ core, onStep }: { core: CoreState; onStep: () => void }) {
  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <div className="flex flex-wrap items-center gap-2">
          <CoreChip core={core} />
          <DaifChip daif={core.daif} />
          {(() => {
            const label = inferTaskLabel(core.pc)
            return label ? (
              <span className="border-border bg-bg/40 type-small inline-flex items-center rounded border px-1.5 py-0.5 tracking-wider">
                {label}
              </span>
            ) : null
          })()}
          {core.wfi_halted && (
            <span className="type-small inline-flex items-center rounded border border-sky-500/40 bg-sky-500/15 px-1.5 py-0.5 tracking-wider text-sky-300">
              WFI · sleeping
            </span>
          )}
          {core.halted && (
            <Badge color={core.last_trap ? 'danger' : 'success'}>
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
      <RegistersCard core={core} />
      <ExceptionCard core={core} />
    </div>
  )
}

function RegistersCard({ core }: { core: CoreState }) {
  return (
    <GlassCard>
      <div className="p-3">
        <div className="text-fg-muted type-small mb-2 font-semibold tracking-wider uppercase">
          Registers
        </div>
        <div className="type-small grid grid-cols-2 gap-x-4 gap-y-0.5">
          {REG_LABELS.map((label, i) => (
            <RegRow key={label} label={label} value={core.x[i]} />
          ))}
          <RegRow label="SP" value={core.sp} />
          <RegRow highlight label="PC" value={core.pc} />
        </div>
        <div className="text-fg-muted type-small mt-2">
          NZCV: <span>{fmtHex32(core.nzcv)}</span> · MPIDR_EL1: <span>{fmtHex64(core.mpidr)}</span>
        </div>
      </div>
    </GlassCard>
  )
}

function ExceptionCard({ core }: { core: CoreState }) {
  const ec = Number((core.esr_el1 >> 26n) & 0x3fn)
  // ESR_EL1=0 with current_el=1 typically means we entered via IRQ (no syndrome).
  const inIrq = core.current_el === 1 && core.esr_el1 === 0n && core.elr_el1 !== 0n
  const ecName = inIrq ? 'IRQ (no ESR syndrome)' : decodeEc(ec)
  return (
    <GlassCard>
      <div className="space-y-2 p-3">
        <div className="text-fg-muted type-small font-semibold tracking-wider uppercase">
          Exception state
        </div>
        <div className="type-small grid gap-x-4 gap-y-0.5 sm:grid-cols-2">
          <div className="text-fg-muted type-small col-span-full tracking-wider uppercase">EL2</div>
          <RegRow label="VBAR_EL2" value={core.vbar_el2} />
          <RegRow label="ELR_EL2" value={core.elr_el2} />
          <RegRow label="SPSR_EL2" value={core.spsr_el2} />
          <RegRow label="ESR_EL2" value={core.esr_el2} />
          <div className="text-fg-muted type-small col-span-full mt-1 tracking-wider uppercase">
            EL1
          </div>
          <RegRow label="VBAR_EL1" value={core.vbar_el1} />
          <RegRow label="ELR_EL1" value={core.elr_el1} />
          <RegRow label="SPSR_EL1" value={core.spsr_el1} />
          <RegRow label="ESR_EL1" value={core.esr_el1} />
        </div>
        {(core.esr_el1 !== 0n || inIrq) && (
          <div className="text-fg-muted type-small">
            {inIrq
              ? `entered via ${ecName} (vector VBAR_EL1+0x480)`
              : `ESR_EL1.EC = 0x${ec.toString(16).padStart(2, '0')} → ${ecName}`}
          </div>
        )}
      </div>
    </GlassCard>
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
      <span className={value === 0n ? 'text-fg-muted' : 'text-fg'}>{fmtHex64(value)}</span>
    </div>
  )
}

function OutputPanel({ output }: { output: string }) {
  return (
    <GlassCard>
      <div className="p-4">
        <div className="text-fg-muted type-small mb-3 font-semibold tracking-wider uppercase">
          UART Output (PA 0x1000) — shared
        </div>
        <pre className="text-fg type-base min-h-12 whitespace-pre-wrap">
          {output || <span className="text-fg-muted">(no output yet)</span>}
        </pre>
      </div>
    </GlassCard>
  )
}

function MemoryPanel({ base, bytes, pcs }: { base: number; bytes: Uint8Array; pcs: number[] }) {
  const rows: { addr: number; bytes: Uint8Array }[] = []
  for (let off = 0; off < bytes.length; off += 16) {
    rows.push({ addr: base + off, bytes: bytes.slice(off, off + 16) })
  }
  return (
    <GlassCard>
      <div className="p-4">
        <div className="text-fg-muted mb-3 flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            Memory (around core 0 PC) — shared
          </span>
          <span className="type-small">
            base {fmtHex32(base)} · pc {pcs.map((pc, i) => `c${i}=${fmtHex32(pc)}`).join(' · ')}
          </span>
        </div>
        <div className="type-small overflow-x-auto">
          {rows.map((r) => (
            <MemoryRow key={r.addr} addr={r.addr} bytes={r.bytes} pcs={pcs} />
          ))}
        </div>
      </div>
    </GlassCard>
  )
}

function MemoryRow({ addr, bytes, pcs }: { addr: number; bytes: Uint8Array; pcs: number[] }) {
  // Color per core's PC: core 0 = accent (blue), core 1 = a different highlight.
  const cells: React.ReactNode[] = []
  for (let i = 0; i < bytes.length; i++) {
    const byteAddr = addr + i
    let cls = 'text-fg'
    if (pcs[0] !== undefined && byteAddr >= pcs[0] && byteAddr < pcs[0] + 4) {
      cls = 'text-accent bg-accent/10 rounded px-0.5'
    } else if (pcs[1] !== undefined && byteAddr >= pcs[1] && byteAddr < pcs[1] + 4) {
      cls = 'rounded bg-violet-500/15 px-0.5 text-violet-300'
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
    <GlassCard>
      <div className="space-y-4 p-4">
        <div className="text-fg-muted flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            MMU · Stage-1 Translation
          </span>
          <Badge color={mmuOn ? 'success' : undefined}>
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
            className="border-border bg-bg/40 text-fg focus:border-accent type-small w-40 rounded border px-2 py-1 outline-none"
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
    </GlassCard>
  )
}

function WalkDisplay({ trace }: { trace: TranslationResult }) {
  if (trace.steps.length === 0 && trace.fault) {
    return (
      <div className="border-danger/40 bg-danger/10 text-danger type-small rounded border px-3 py-2">
        {trace.fault}
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
    <div className={`border-border/40 rounded border px-3 py-2 ${tone}`}>
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

function decodeEc(ec: number): string {
  switch (ec) {
    case 0x00:
      return 'unknown'
    case 0x15:
      return 'SVC (AArch64)'
    case 0x16:
      return 'HVC (AArch64)'
    case 0x17:
      return 'SMC (AArch64)'
    case 0x20:
      return 'instruction abort, lower EL'
    case 0x21:
      return 'instruction abort, current EL'
    case 0x24:
      return 'data abort, lower EL'
    case 0x25:
      return 'data abort, current EL'
    case 0x3c:
      return 'BRK (AArch64)'
    default:
      return '—'
  }
}
