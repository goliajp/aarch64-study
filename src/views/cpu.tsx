// Top-level "CPU view" — the single page that the user lands on.
// Responsibilities, in order:
//   1. Boot the WASM simulator and hold the live `Cpu` handle in state.
//   2. Run a `refresh()` pass that pulls a fresh snapshot of every
//      observable simulator surface (cores, AIC, block, memory, output,
//      slot regions, sysinfo) and derives a list of recent SimEvents
//      from the diff against the previous snapshot.
//   3. Drive the auto-run loop via requestAnimationFrame and pause it
//      whenever every core is halted or parked in WFI.
//   4. Compose the UI by wiring each panel to its slice of state. All of
//      the display logic lives in the panel modules under
//      `src/components/`.

import { Badge } from '@goliapkg/gds'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import init, { Cpu } from 'aarch64-sim'

import { CoreChip } from '../components/badges'
import { ControlBar } from '../components/control-bar'
import { AicPanel } from '../components/panels/aic-panel'
import { BlockPanel } from '../components/panels/block-panel'
import { CoreColumn } from '../components/panels/core-column'
import { DisassemblyPanel } from '../components/panels/disassembly-panel'
import { MemoryPanel } from '../components/panels/memory-panel'
import { MmuPanel } from '../components/panels/mmu-panel'
import { OutputPanel } from '../components/panels/output-panel'
import { SavePanel } from '../components/panels/save-panel'
import { SystemDiagram } from '../components/system-diagram'
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

// Window during which a derived SimEvent is considered "active" (used by
// SystemDiagram's idle/active indicator).
const EVENT_TTL_MS = 800

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
    // Centre memory on core 0's PC, snapped to a 16-byte boundary.
    const viewStart = Number(s[0].pc) & ~0xf
    setMemory(c.mem_slice(viewStart, MEMORY_VIEW_BYTES))
    const newOutput = c.output()
    setOutput(newOutput)
    // Pull each core's slot region (0x4F00 = core 0, 0x5000 = core 1).
    const slot0 = parseCoreSlot(c.mem_slice(0x4f00, 0x50))
    const slot1 = parseCoreSlot(c.mem_slice(0x5000, 0x50))
    setCoreSlots([slot0, slot1])

    // Derive SimEvents from the diff against the previous snapshot.
    const now = performance.now()
    const newEvents: SimEvent[] = []
    const prev = prevRef.current
    if (prev) {
      // Store events: output grew. Attribute to whichever core(s) are at
      // EL0 and not WFI/halted (the only ones that could have STR'd).
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
      const VBAR_EL1 = 0x4400
      s.forEach((core, i) => {
        const prevCore = prev.cores[i]
        if (!prevCore) return
        const pc = Number(core.pc)
        const prevPc = Number(prevCore.pc)
        const target = i === 0 ? 'core0' : 'core1'
        if (pc === VBAR_EL1 + 0x480 && prevPc !== VBAR_EL1 + 0x480) {
          newEvents.push({
            id: ++eventIdRef.current,
            kind: 'irq_taken',
            source: 'aic',
            target,
            ts: now,
          })
        }
        if (pc === VBAR_EL1 + 0x400 && prevPc !== VBAR_EL1 + 0x400) {
          newEvents.push({
            id: ++eventIdRef.current,
            kind: 'svc',
            source: target,
            target,
            ts: now,
          })
        }
        if (core.current_el < prevCore.current_el) {
          newEvents.push({
            id: ++eventIdRef.current,
            kind: 'eret',
            source: target,
            target,
            ts: now,
          })
        }
      })
    }
    setEvents((prevEvents) => {
      const fresh = prevEvents.filter((e) => now - e.ts < EVENT_TTL_MS)
      return newEvents.length > 0 ? [...fresh, ...newEvents] : fresh
    })

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

  // ── Lifecycle ───────────────────────────────────────────────────────────
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

  // ── Step / Run / Reset ──────────────────────────────────────────────────
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
      // Resuming from auto-pause: if every core is already idle (halted
      // or WFI), pressing Run again would just immediately re-pause.
      // Reset first so the user gets a fresh boot run.
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
    // Auto-run pauses the moment every core is halted or parked in WFI.
    // From there a manual Step (or Reset+Run) is the only way forward.
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

      {/* Strict 3-column grid: every panel is exactly 1/3 width. Where two
          short panels go together they're stacked inside their column with
          `flex h-full flex-col` so the column matches its taller siblings. */}
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
            rest of the column height so the column matches the tall
            SystemDiagram on its left. */}
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

        {/* Memory + per-core save areas + AIC stacked vertically. Memory is
            the dominant content and grows to fill any leftover height so
            the column matches the cores column. */}
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
