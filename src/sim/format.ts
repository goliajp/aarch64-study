import type { CoreSlot } from './types'

export const REG_LABELS = Array.from({ length: 31 }, (_, i) => `X${i}`)
export const MEMORY_VIEW_BYTES = 512
export const RUN_BURST = 2
export const TASK_A_ENTRY = 0x4d00
export const TASK_B_ENTRY = 0x4e00
export const IRQ_NAMES = ['TIMER', 'IPI']

export function fmtHex64(v: bigint): string {
  return '0x' + v.toString(16).padStart(16, '0')
}

export function fmtHex32(v: number): string {
  return '0x' + (v >>> 0).toString(16).padStart(8, '0')
}

/** Returns null on empty / malformed input so callers can fall back. */
export function parseHex(text: string): bigint | null {
  const t = text.trim()
  if (t === '') return null
  try {
    return BigInt(t.startsWith('0x') || t.startsWith('0X') ? t : '0x' + t)
  } catch {
    return null
  }
}

/** Layout: +0x00 entry · +0x08 save_ptr · +0x10 save 0 (X0..X3) · +0x30 save 1. */
export function parseCoreSlot(bytes: Uint8Array): CoreSlot {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const u64 = (off: number) => view.getBigUint64(off, true)
  return {
    entry: u64(0),
    savePtr: u64(8),
    save0: { x0: u64(0x10), x1: u64(0x18), x2: u64(0x20), x3: u64(0x28) },
    save1: { x0: u64(0x30), x1: u64(0x38), x2: u64(0x40), x3: u64(0x48) },
  }
}

export function inferTaskLabel(pc: bigint): string | null {
  const p = Number(pc)
  if (p >= TASK_A_ENTRY && p < TASK_A_ENTRY + 0x20) return 'task A'
  if (p >= TASK_B_ENTRY && p < TASK_B_ENTRY + 0x20) return 'task B'
  if (p >= 0x4400 && p < 0x4500) return 'sync handler'
  if (p >= 0x4880 && p < 0x4900) return 'IRQ handler (sched)'
  if (p >= 0x4000 && p < 0x4400) return 'kernel boot'
  return null
}
