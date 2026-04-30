import { Badge, GlassButton, GlassCard } from '@goliapkg/gds'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import init, { Cpu } from 'aarch64-sim'

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

interface BlockState {
  sector: bigint
  buf_addr: bigint
  last_command: bigint
  status: bigint
  total_reads: bigint
  total_writes: bigint
  disk: Uint8Array
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
  const runRafRef = useRef<number | null>(null)

  const refresh = useCallback((c: Cpu) => {
    const s = c.state() as CoreState[]
    setCores(s)
    setAic(c.aic_state() as AicState)
    setBlock(c.block_state() as BlockState)
    setSysInfo({
      systemSteps: c.system_steps(),
      timerPeriod: c.timer_period(),
      timerRemaining: c.timer_remaining(),
      timerTicks: c.timer_ticks(),
    })
    // Center memory on core 0's PC.
    const viewStart = Number(s[0].pc) & ~0xf
    setMemory(c.mem_slice(viewStart, MEMORY_VIEW_BYTES))
    setOutput(c.output())
    // Pull each core's slot region (0x4F00 = core 0, 0x5000 = core 1).
    const slot0 = parseCoreSlot(c.mem_slice(0x4f00, 0x50))
    const slot1 = parseCoreSlot(c.mem_slice(0x5000, 0x50))
    setCoreSlots([slot0, slot1])
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
    return <div className="text-fg-muted text-sm">Loading WASM…</div>
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
          <Badge color="info">v0.13</Badge>
          {cores.map((c) => (
            <CoreChip core={c} key={c.id} />
          ))}
          {allHalted ? (
            <Badge color={anyTrap ? 'danger' : 'success'}>{anyTrap ? 'TRAP' : 'ALL HALTED'}</Badge>
          ) : (
            <Badge>RUNNING</Badge>
          )}
        </div>
        <p className="text-fg-muted max-w-2xl text-xs">
          Task A now uses <code>WFI</code> after each print: print 'A' once, then sleep until the
          next IRQ. The core running A is mostly asleep (look for the "WFI · sleeping" badge); the
          core running task B (disk printer) stays busy. Each timer tick swaps both cores — the busy
          one parks itself, the parked one starts streaming.
        </p>
      </header>

      <SystemInfoBar info={sysInfo} totalCoreSteps={cores.reduce((a, c) => a + c.steps, 0n)} />

      <AicPanel aic={aic} />

      <SavePanel slots={coreSlots} />

      <BlockPanel block={block} />

      <div className="flex flex-wrap items-center gap-2">
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

      {anyTrap && anyTrap.last_trap && (
        <div className="border-danger/40 bg-danger/10 text-danger rounded border px-3 py-2 font-mono text-xs">
          core {anyTrap.id}: {anyTrap.last_trap}
        </div>
      )}

      <div className="grid gap-4 lg:grid-cols-2">
        {cores.map((c) => (
          <CoreColumn core={c} key={c.id} onStep={() => onStepCore(c.id)} />
        ))}
      </div>

      <OutputPanel output={output} />

      <MemoryPanel base={memBaseAddr} bytes={memory} pcs={corePcs} />

      <MmuPanel
        cores={cores}
        onCoreChange={setTranslateCoreIdx}
        onVaChange={setVaText}
        selectedCoreIdx={translateCoreIdx}
        trace={trace}
        vaText={vaText}
      />
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
  const slice = block.disk.slice(sector * SECTOR_SIZE, (sector + 1) * SECTOR_SIZE)
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
          <span className="text-[10px] font-semibold tracking-wider uppercase">
            Block device · disk image (8 × 64-byte sectors @ MMIO 0x3000)
          </span>
          <span className="font-mono text-[10px]">
            reads {block.total_reads.toString()} · writes {block.total_writes.toString()}
          </span>
        </div>
        <div className="grid gap-x-6 gap-y-1 font-mono text-xs sm:grid-cols-4">
          <RegRow label="SECTOR" value={block.sector} />
          <RegRow label="BUF_ADDR" value={block.buf_addr} />
          <RegRow label="CMD" value={block.last_command} />
          <div className="flex justify-between gap-2">
            <span className="text-fg-muted">STATUS</span>
            <span className="text-fg">{statusLabel}</span>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-1">
          <span className="text-fg-muted mr-2 text-xs">view sector</span>
          {Array.from({ length: numSectors }, (_, i) => (
            <button
              className={`rounded border px-2 py-0.5 font-mono text-[11px] ${
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
    <div className="overflow-x-auto font-mono text-xs">
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
        <div className="text-fg-muted text-[10px] font-semibold tracking-wider uppercase">
          Per-core scheduler slots — entry + save_ptr + 2 save areas each
        </div>
        <div className="grid gap-3 lg:grid-cols-2">
          {slots.map((slot, i) => (
            <CoreSlotCard base={i === 0 ? 0x4f00 : 0x5000} coreId={i} key={i} slot={slot} />
          ))}
        </div>
        <div className="text-fg-muted text-[11px]">
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
    <div className="border-border bg-bg/40 space-y-2 rounded border px-3 py-2 font-mono text-[11px]">
      <div className="text-fg-muted flex items-center justify-between text-[10px] tracking-wider uppercase">
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
        className={`mb-1 flex items-center justify-between text-[10px] tracking-wider uppercase ${
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
          <span className="text-[10px] font-semibold tracking-wider uppercase">
            AIC · Apple-style interrupt controller
          </span>
          <span className="font-mono text-[10px]">
            base 0x2000 · ACK reads cleared {aic.total_acks.toString()}
          </span>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          {aic.pending.map((bits, i) => (
            <div
              className="border-border bg-bg/40 rounded border px-3 py-2 font-mono text-xs"
              key={i}
            >
              <div className="text-fg-muted mb-1 text-[10px] tracking-wider uppercase">
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

function SystemInfoBar({ info, totalCoreSteps }: { info: SystemInfo; totalCoreSteps: bigint }) {
  return (
    <div className="border-border bg-bg/40 flex flex-wrap items-center gap-x-6 gap-y-1 rounded border px-3 py-2 font-mono text-xs">
      <span className="text-fg-muted">
        system steps: <span className="text-fg">{info.systemSteps.toString()}</span>
      </span>
      <span className="text-fg-muted">
        retired (sum): <span className="text-fg">{totalCoreSteps.toString()}</span>
      </span>
      <span className="text-fg-muted">
        timer period: <span className="text-fg">{info.timerPeriod.toString()}</span>
      </span>
      <span className="text-fg-muted">
        next IRQ in:{' '}
        <span className={info.timerRemaining === 0n ? 'text-warn text-amber-400' : 'text-fg'}>
          {info.timerRemaining.toString()}
        </span>
      </span>
      <span className="text-fg-muted">
        timer ticks: <span className="text-fg">{info.timerTicks.toString()}</span>
      </span>
    </div>
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
      className="border-border bg-bg/40 inline-flex items-center gap-1 rounded border px-1.5 py-0.5 font-mono text-[10px] tracking-wider"
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
      className={`inline-flex items-center gap-1.5 rounded border px-2 py-0.5 font-mono text-[10px] tracking-wider ${elClass}`}
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
              <span className="border-border bg-bg/40 inline-flex items-center rounded border px-1.5 py-0.5 font-mono text-[10px] tracking-wider">
                {label}
              </span>
            ) : null
          })()}
          {core.wfi_halted && (
            <span className="inline-flex items-center rounded border border-sky-500/40 bg-sky-500/15 px-1.5 py-0.5 font-mono text-[10px] tracking-wider text-sky-300">
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
          <span className="text-fg-muted font-mono text-[10px]">steps {core.steps.toString()}</span>
          <GlassButton onClick={onStep} size="sm">
            Step
          </GlassButton>
        </div>
      </div>
      {core.last_trap && (
        <div className="border-danger/40 bg-danger/10 text-danger rounded border px-2 py-1 font-mono text-[11px]">
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
        <div className="text-fg-muted mb-2 text-[10px] font-semibold tracking-wider uppercase">
          Registers
        </div>
        <div className="grid grid-cols-2 gap-x-4 gap-y-0.5 font-mono text-[11px]">
          {REG_LABELS.map((label, i) => (
            <RegRow key={label} label={label} value={core.x[i]} />
          ))}
          <RegRow label="SP" value={core.sp} />
          <RegRow highlight label="PC" value={core.pc} />
        </div>
        <div className="text-fg-muted mt-2 text-[11px]">
          NZCV: <span className="font-mono">{fmtHex32(core.nzcv)}</span> · MPIDR_EL1:{' '}
          <span className="font-mono">{fmtHex64(core.mpidr)}</span>
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
        <div className="text-fg-muted text-[10px] font-semibold tracking-wider uppercase">
          Exception state
        </div>
        <div className="grid gap-x-4 gap-y-0.5 font-mono text-[11px] sm:grid-cols-2">
          <div className="text-fg-muted col-span-full text-[10px] tracking-wider uppercase">
            EL2
          </div>
          <RegRow label="VBAR_EL2" value={core.vbar_el2} />
          <RegRow label="ELR_EL2" value={core.elr_el2} />
          <RegRow label="SPSR_EL2" value={core.spsr_el2} />
          <RegRow label="ESR_EL2" value={core.esr_el2} />
          <div className="text-fg-muted col-span-full mt-1 text-[10px] tracking-wider uppercase">
            EL1
          </div>
          <RegRow label="VBAR_EL1" value={core.vbar_el1} />
          <RegRow label="ELR_EL1" value={core.elr_el1} />
          <RegRow label="SPSR_EL1" value={core.spsr_el1} />
          <RegRow label="ESR_EL1" value={core.esr_el1} />
        </div>
        {(core.esr_el1 !== 0n || inIrq) && (
          <div className="text-fg-muted text-[11px]">
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
        <div className="text-fg-muted mb-3 text-[10px] font-semibold tracking-wider uppercase">
          UART Output (PA 0x1000) — shared
        </div>
        <pre className="text-fg min-h-12 font-mono text-sm whitespace-pre-wrap">
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
          <span className="text-[10px] font-semibold tracking-wider uppercase">
            Memory (around core 0 PC) — shared
          </span>
          <span className="font-mono text-[10px]">
            base {fmtHex32(base)} · pc {pcs.map((pc, i) => `c${i}=${fmtHex32(pc)}`).join(' · ')}
          </span>
        </div>
        <div className="overflow-x-auto font-mono text-xs">
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
          <span className="text-[10px] font-semibold tracking-wider uppercase">
            MMU · Stage-1 Translation
          </span>
          <Badge color={mmuOn ? 'success' : undefined}>
            SCTLR_EL{1}.M={mmuOn ? '1' : '0'}
          </Badge>
        </div>

        <div className="flex items-center gap-2 text-xs">
          <span className="text-fg-muted">walk using</span>
          {cores.map((c) => (
            <button
              className={`rounded border px-2 py-0.5 font-mono text-[11px] ${
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

        <div className="grid gap-x-6 gap-y-1 font-mono text-xs sm:grid-cols-3">
          <RegRow label="TTBR0_EL1" value={sel.ttbr0_el1} />
          <RegRow label="TCR_EL1" value={sel.tcr_el1} />
          <RegRow label="SCTLR_EL1" value={sel.sctlr_el1} />
        </div>
        <div className="text-fg-muted text-xs">
          T0SZ={t0sz} · VA={vaBits} bits · 4 KiB granule · start level{' '}
          {trace && trace.steps.length > 0 ? trace.steps[0].level : '?'}
        </div>

        <div
          className={`rounded border px-3 py-1.5 text-xs ${
            mmuOn ? 'border-success/40 bg-success/10 text-success' : 'border-border text-fg-muted'
          }`}
        >
          {mmuOn
            ? 'Translation active — every fetch and LDR/STR runs through this walk.'
            : 'Translation is a query only — fetches and LDR/STR bypass the MMU until SCTLR_EL1.M=1.'}
        </div>

        <div className="flex items-center gap-2">
          <label className="text-fg-muted font-mono text-xs" htmlFor="va-input">
            translate VA
          </label>
          <input
            className="border-border bg-bg/40 text-fg focus:border-accent w-40 rounded border px-2 py-1 font-mono text-xs outline-none"
            id="va-input"
            onChange={(e) => onVaChange(e.target.value)}
            placeholder="0x4000"
            spellCheck={false}
            value={vaText}
          />
          <span className="text-fg-muted text-xs">try 0x4000, 0x1000, 0x4800, 0x4C00, 0x2000</span>
        </div>

        {trace && <WalkDisplay trace={trace} />}
      </div>
    </GlassCard>
  )
}

function WalkDisplay({ trace }: { trace: TranslationResult }) {
  if (trace.steps.length === 0 && trace.fault) {
    return (
      <div className="border-danger/40 bg-danger/10 text-danger rounded border px-3 py-2 font-mono text-xs">
        {trace.fault}
      </div>
    )
  }
  return (
    <div className="space-y-2 font-mono text-xs">
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
      <div className="text-fg-muted mt-1 text-[11px]">
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
