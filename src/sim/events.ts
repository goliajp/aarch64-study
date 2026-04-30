import type { BlockState, CoreState, PrevSnapshot, SimEvent } from './types'

export const EVENT_TTL_MS = 800

const VBAR_EL1 = 0x4400
const IRQ_VECTOR = VBAR_EL1 + 0x480
const SYNC_VECTOR = VBAR_EL1 + 0x400

interface DeriveArgs {
  cores: CoreState[]
  blockState: BlockState
  outputLen: number
  ticks: bigint
  prev: PrevSnapshot | null
  now: number
  nextId: () => number
}

/** Produce SimEvents for whatever changed since `prev`. Pure function. */
export function deriveEvents(args: DeriveArgs): SimEvent[] {
  const { cores, blockState, outputLen, ticks, prev, now, nextId } = args
  if (!prev) return []
  const out: SimEvent[] = []

  // Output grew → some EL0 core wrote to UART.
  if (outputLen > prev.outputLen) {
    cores.forEach((c, i) => {
      if (!c.wfi_halted && !c.halted && c.current_el === 0) {
        out.push({
          id: nextId(),
          kind: 'store',
          source: i === 0 ? 'core0' : 'core1',
          target: 'uart',
          ts: now,
        })
      }
    })
  }

  if (ticks > prev.ticks) {
    out.push({ id: nextId(), kind: 'timer', source: 'aic', target: 'core0', ts: now })
    out.push({ id: nextId(), kind: 'timer', source: 'aic', target: 'core1', ts: now + 30 })
  }

  if (blockState.total_reads > prev.totalReads) {
    out.push({ id: nextId(), kind: 'disk_read', source: 'block', target: 'ram', ts: now })
  }

  cores.forEach((core, i) => {
    const prevCore = prev.cores[i]
    if (!prevCore) return
    const pc = Number(core.pc)
    const prevPc = Number(prevCore.pc)
    const target = i === 0 ? ('core0' as const) : ('core1' as const)
    if (pc === IRQ_VECTOR && prevPc !== IRQ_VECTOR) {
      out.push({ id: nextId(), kind: 'irq_taken', source: 'aic', target, ts: now })
    }
    if (pc === SYNC_VECTOR && prevPc !== SYNC_VECTOR) {
      out.push({ id: nextId(), kind: 'svc', source: target, target, ts: now })
    }
    if (core.current_el < prevCore.current_el) {
      out.push({ id: nextId(), kind: 'eret', source: target, target, ts: now })
    }
  })

  return out
}
