import { Badge, GlassButton, GlassCard } from '@goliapkg/gds'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import init, { Cpu } from 'aarch64-sim'

interface CpuState {
  x: bigint[]
  sp: bigint
  pc: bigint
  nzcv: number
  halted: boolean
  last_trap: string | null
  steps: bigint
  ttbr0_el1: bigint
  tcr_el1: bigint
  sctlr_el1: bigint
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
const RUN_BURST = 1

function fmtHex64(v: bigint): string {
  return '0x' + v.toString(16).padStart(16, '0')
}

function fmtHex32(v: number): string {
  return '0x' + (v >>> 0).toString(16).padStart(8, '0')
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
  const [state, setState] = useState<CpuState | null>(null)
  const [memory, setMemory] = useState<Uint8Array>(new Uint8Array(MEMORY_VIEW_BYTES))
  const [output, setOutput] = useState('')
  const [running, setRunning] = useState(false)
  const [vaText, setVaText] = useState('0x4000')
  const runRafRef = useRef<number | null>(null)

  const refresh = useCallback((c: Cpu) => {
    const s = c.state() as CpuState
    setState(s)
    const viewStart = Number(s.pc) & ~0xf
    setMemory(c.mem_slice(viewStart, MEMORY_VIEW_BYTES))
    setOutput(c.output())
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

  // run loop via rAF: each frame, do RUN_BURST steps so the user can watch state change.
  useEffect(() => {
    if (!running || !cpu) return
    const tick = () => {
      if ((cpu.state() as CpuState).halted) {
        setRunning(false)
        return
      }
      cpu.run(RUN_BURST)
      refresh(cpu)
      if ((cpu.state() as CpuState).halted) {
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

  // Re-translate whenever the user's VA changes or the simulator advances
  // (memory writes can change the walk; state.steps is the change signal).
  const trace = useMemo<TranslationResult | null>(() => {
    if (!cpu || !state) return null
    const va = parseHex(vaText)
    if (va === null) return null
    try {
      return cpu.translate(va) as TranslationResult
    } catch {
      return null
    }
  }, [cpu, state, vaText])

  if (!cpu || !state) {
    return <div className="text-fg-muted text-sm">Loading WASM…</div>
  }

  const memBaseAddr = Number(state.pc) & ~0xf

  return (
    <div className="space-y-6">
      <header className="space-y-2">
        <div className="flex items-center gap-3">
          <h1
            className="text-fg text-2xl font-bold"
            style={{ textShadow: '0 0 20px var(--gds-accent, #3b82f6)' }}
          >
            AArch64 CPU
          </h1>
          <Badge color="info">v0.2</Badge>
          {state.halted ? (
            <Badge color={state.last_trap ? 'danger' : 'success'}>
              {state.last_trap ? 'TRAP' : 'HALTED'}
            </Badge>
          ) : (
            <Badge>RUNNING</Badge>
          )}
        </div>
        <p className="text-fg-muted max-w-2xl text-xs">
          Tiny AArch64 simulator running in WASM. The demo program writes "Hello\n" to a
          memory-mapped UART at 0x1000. Stage-1 page tables are pre-loaded — translation is shown
          live below; SCTLR_EL1.M=0 so LDR/STR still bypass the MMU.
        </p>
      </header>

      <div className="flex flex-wrap gap-2">
        <GlassButton onClick={onStep} size="sm" variant="accent">
          Step
        </GlassButton>
        <GlassButton onClick={onRunToggle} size="sm">
          {running ? 'Pause' : 'Run'}
        </GlassButton>
        <GlassButton onClick={onReset} size="sm">
          Reset
        </GlassButton>
        <div className="text-fg-muted ml-auto self-center font-mono text-xs">
          steps: {state.steps.toString()}
        </div>
      </div>

      {state.last_trap && (
        <div className="border-danger/40 bg-danger/10 text-danger rounded border px-3 py-2 font-mono text-xs">
          {state.last_trap}
        </div>
      )}

      <div className="grid gap-4 lg:grid-cols-[2fr_1fr]">
        <RegistersPanel state={state} />
        <OutputPanel output={output} />
      </div>

      <MemoryPanel base={memBaseAddr} bytes={memory} pc={Number(state.pc)} />

      <MmuPanel state={state} trace={trace} vaText={vaText} onVaChange={setVaText} />
    </div>
  )
}

function RegistersPanel({ state }: { state: CpuState }) {
  return (
    <GlassCard>
      <div className="p-4">
        <div className="text-fg-muted mb-3 text-[10px] font-semibold tracking-wider uppercase">
          Registers
        </div>
        <div className="grid grid-cols-2 gap-x-6 gap-y-1 font-mono text-xs sm:grid-cols-3">
          {REG_LABELS.map((label, i) => (
            <RegRow key={label} label={label} value={state.x[i]} />
          ))}
          <RegRow label="SP" value={state.sp} />
          <RegRow highlight label="PC" value={state.pc} />
        </div>
        <div className="text-fg-muted mt-3 text-xs">
          NZCV: <span className="font-mono">{fmtHex32(state.nzcv)}</span>
        </div>
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
          UART Output (0x1000)
        </div>
        <pre className="text-fg min-h-24 font-mono text-sm whitespace-pre-wrap">
          {output || <span className="text-fg-muted">(no output yet)</span>}
        </pre>
      </div>
    </GlassCard>
  )
}

function MemoryPanel({ base, bytes, pc }: { base: number; bytes: Uint8Array; pc: number }) {
  const rows: { addr: number; bytes: Uint8Array }[] = []
  for (let off = 0; off < bytes.length; off += 16) {
    rows.push({ addr: base + off, bytes: bytes.slice(off, off + 16) })
  }
  return (
    <GlassCard>
      <div className="p-4">
        <div className="text-fg-muted mb-3 flex items-center justify-between">
          <span className="text-[10px] font-semibold tracking-wider uppercase">
            Memory (around PC)
          </span>
          <span className="font-mono text-[10px]">
            base {fmtHex32(base)} · pc {fmtHex32(pc)}
          </span>
        </div>
        <div className="overflow-x-auto font-mono text-xs">
          {rows.map((r) => (
            <MemoryRow key={r.addr} addr={r.addr} bytes={r.bytes} pc={pc} />
          ))}
        </div>
      </div>
    </GlassCard>
  )
}

function MmuPanel({
  onVaChange,
  state,
  trace,
  vaText,
}: {
  onVaChange: (v: string) => void
  state: CpuState
  trace: TranslationResult | null
  vaText: string
}) {
  const t0sz = Number(state.tcr_el1 & 0x3fn)
  const vaBits = t0sz > 0 ? 64 - t0sz : 0
  const mmuOn = (state.sctlr_el1 & 1n) !== 0n

  return (
    <GlassCard>
      <div className="space-y-4 p-4">
        <div className="text-fg-muted flex items-center justify-between">
          <span className="text-[10px] font-semibold tracking-wider uppercase">
            MMU · Stage-1 Translation
          </span>
          <Badge color={mmuOn ? 'success' : undefined}>SCTLR_EL1.M={mmuOn ? '1' : '0'}</Badge>
        </div>

        <div className="grid gap-x-6 gap-y-1 font-mono text-xs sm:grid-cols-3">
          <RegRow label="TTBR0_EL1" value={state.ttbr0_el1} />
          <RegRow label="TCR_EL1" value={state.tcr_el1} />
          <RegRow label="SCTLR_EL1" value={state.sctlr_el1} />
        </div>
        <div className="text-fg-muted text-xs">
          T0SZ={t0sz} · VA={vaBits} bits · 4 KiB granule · start level{' '}
          {trace && trace.steps.length > 0 ? trace.steps[0].level : '?'}
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
          <span className="text-fg-muted text-xs">
            try 0x4000 (program), 0x1000 (UART), 0x2000 (unmapped)
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

function MemoryRow({ addr, bytes, pc }: { addr: number; bytes: Uint8Array; pc: number }) {
  const cells: React.ReactNode[] = []
  for (let i = 0; i < bytes.length; i++) {
    const byteAddr = addr + i
    const inPcWord = byteAddr >= pc && byteAddr < pc + 4
    cells.push(
      <span className={inPcWord ? 'text-accent bg-accent/10 rounded px-0.5' : 'text-fg'} key={i}>
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
