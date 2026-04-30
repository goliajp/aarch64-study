import { Badge } from '@goliapkg/gds'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import init, { Cpu } from 'aarch64-sim'

import { CoreChip } from '../components/badges'
import { ControlBar } from '../components/control-bar'
import { AicPanel } from '../components/panels/aic-panel'
import { BlockPanel } from '../components/panels/block-panel'
import { CoreMonitor } from '../components/panels/core-monitor'
import { DisassemblyPanel } from '../components/panels/disassembly-panel'
import { MemoryPanel } from '../components/panels/memory-panel'
import { MmuPanel } from '../components/panels/mmu-panel'
import { OutputPanel } from '../components/panels/output-panel'
import { SavePanel } from '../components/panels/save-panel'
import { SystemDiagram } from '../components/system-diagram'
import { EVENT_TTL_MS, deriveEvents } from '../sim/events'
import { MEMORY_VIEW_BYTES, RUN_BURST, parseCoreSlot, parseHex } from '../sim/format'
import type {
  AicState,
  BlockState,
  CoreSlot,
  CoreState,
  PrevSnapshot,
  SimEvent,
  SystemInfo,
  TranslationResult,
} from '../sim/types'

const allIdle = (cores: CoreState[]) => cores.every((c) => c.halted || c.wfi_halted)

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
    const aicState = c.aic_state() as AicState
    const blockState = c.block_state() as BlockState
    const ticks = c.timer_ticks()
    setCores(s)
    setAic(aicState)
    setBlock(blockState)
    setSysInfo({
      systemSteps: c.system_steps(),
      timerPeriod: c.timer_period(),
      timerRemaining: c.timer_remaining(),
      timerTicks: ticks,
      atomicCounter: c.atomic_counter(),
    })
    const viewStart = Number(s[0].pc) & ~0xf
    setMemory(c.mem_slice(viewStart, MEMORY_VIEW_BYTES))
    const newOutput = c.output()
    setOutput(newOutput)
    setCoreSlots([
      parseCoreSlot(c.mem_slice(0x4f00, 0x50)),
      parseCoreSlot(c.mem_slice(0x5000, 0x50)),
    ])

    const now = performance.now()
    const newEvents = deriveEvents({
      cores: s,
      aicState,
      blockState,
      outputLen: newOutput.length,
      ticks,
      prev: prevRef.current,
      now,
      nextId: () => ++eventIdRef.current,
    })
    setEvents((prev) => {
      const fresh = prev.filter((e) => now - e.ts < EVENT_TTL_MS)
      return newEvents.length > 0 ? [...fresh, ...newEvents] : fresh
    })

    prevRef.current = {
      cores: s.map((core) => ({
        pc: core.pc,
        current_el: core.current_el,
        wfi_halted: core.wfi_halted,
      })),
      outputLen: newOutput.length,
      ticks,
      totalReads: blockState.total_reads,
      totalIpis: aicState.total_ipis,
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
      // Resuming from auto-pause: every core is already idle, so we'd
      // immediately re-pause without progress. Reset for a fresh boot.
      if (!r && cpu) {
        const states = cpu.state() as CoreState[]
        if (states.length > 0 && allIdle(states)) {
          cpu.reset()
          refresh(cpu)
        }
      }
      return !r
    })
  }, [cpu, refresh])

  useEffect(() => {
    if (!running || !cpu) return
    const tick = () => {
      const states = cpu.state() as CoreState[]
      if (allIdle(states)) {
        setRunning(false)
        return
      }
      cpu.run(RUN_BURST)
      refresh(cpu)
      if (allIdle(cpu.state() as CoreState[])) {
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
          <Badge variant="info">v0.19</Badge>
          {cores.map((c) => (
            <CoreChip core={c} key={c.id} />
          ))}
          <StatusBadge cores={cores} running={running} trap={!!anyTrap} />
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

      <div className="grid min-w-0 grid-cols-1 gap-4 lg:grid-cols-3">
        <SystemDiagram
          aic={aic}
          block={block}
          cores={cores}
          events={events}
          output={output}
          slots={coreSlots}
        />

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

        <DisassemblyPanel cores={cores} cpu={cpu} />

        <div className="flex h-full flex-col gap-4">
          <div className="min-h-0 flex-1">
            <MemoryPanel base={memBaseAddr} bytes={memory} pcs={corePcs} />
          </div>
          <SavePanel slots={coreSlots} />
          <AicPanel aic={aic} />
        </div>

        <div className="space-y-4">
          {cores.map((c) => (
            <CoreMonitor core={c} key={c.id} onStep={() => onStepCore(c.id)} />
          ))}
        </div>

        <BlockPanel block={block} />
      </div>
    </div>
  )
}

function StatusBadge({
  cores,
  running,
  trap,
}: {
  cores: CoreState[]
  running: boolean
  trap: boolean
}) {
  if (trap) return <Badge variant="danger">TRAP</Badge>
  if (cores.every((c) => c.halted)) return <Badge variant="success">ALL HALTED</Badge>
  if (running) {
    return (
      <Badge>
        <span className="live-pulse mr-1 inline-block h-1.5 w-1.5 rounded-full bg-emerald-400" />
        RUNNING
      </Badge>
    )
  }
  if (cores.every((c) => c.wfi_halted)) return <Badge variant="info">IDLE</Badge>
  return <Badge variant="info">PAUSED</Badge>
}
