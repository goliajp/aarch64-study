import { Badge, GlassButton, GlassCard } from '@goliapkg/gds'
import { useCallback, useEffect, useRef, useState } from 'react'

import init, { Cpu } from 'aarch64-sim'

interface CpuState {
  x: bigint[]
  sp: bigint
  pc: bigint
  nzcv: number
  halted: boolean
  last_trap: string | null
  steps: bigint
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

export function CpuView() {
  const cpuRef = useRef<Cpu | null>(null)
  const [ready, setReady] = useState(false)
  const [state, setState] = useState<CpuState | null>(null)
  const [memory, setMemory] = useState<Uint8Array>(new Uint8Array(MEMORY_VIEW_BYTES))
  const [output, setOutput] = useState('')
  const [running, setRunning] = useState(false)
  const runRafRef = useRef<number | null>(null)

  const refresh = useCallback(() => {
    const cpu = cpuRef.current
    if (!cpu) return
    const s = cpu.state() as CpuState
    setState(s)
    const viewStart = Number(s.pc) & ~0xf
    setMemory(cpu.mem_slice(viewStart, MEMORY_VIEW_BYTES))
    setOutput(cpu.output())
  }, [])

  useEffect(() => {
    let cancelled = false
    void (async () => {
      await init()
      if (cancelled) return
      cpuRef.current = new Cpu()
      setReady(true)
      // initial snapshot
      const cpu = cpuRef.current
      const s = cpu.state() as CpuState
      setState(s)
      setMemory(cpu.mem_slice(Number(cpu.entry_pc()), MEMORY_VIEW_BYTES))
      setOutput(cpu.output())
    })()
    return () => {
      cancelled = true
    }
  }, [])

  const onStep = useCallback(() => {
    cpuRef.current?.step()
    refresh()
  }, [refresh])

  const onReset = useCallback(() => {
    cpuRef.current?.reset()
    setRunning(false)
    if (runRafRef.current != null) {
      cancelAnimationFrame(runRafRef.current)
      runRafRef.current = null
    }
    refresh()
  }, [refresh])

  const onRunToggle = useCallback(() => {
    setRunning((r) => !r)
  }, [])

  // run loop via rAF: each frame, do RUN_BURST steps so the user can watch state change.
  useEffect(() => {
    if (!running) return
    const tick = () => {
      const cpu = cpuRef.current
      if (!cpu) return
      const before = (cpu.state() as CpuState).halted
      if (before) {
        setRunning(false)
        return
      }
      cpu.run(RUN_BURST)
      refresh()
      const after = (cpu.state() as CpuState).halted
      if (after) {
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
  }, [running, refresh])

  if (!ready || !state) {
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
          <Badge color="info">v0.1</Badge>
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
          memory-mapped UART at 0x1000 by storing one character at a time.
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
