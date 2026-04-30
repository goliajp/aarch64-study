// Formatting + parsing helpers shared across all panels. Keep them tiny and
// pure so they're cheap to call from render code.

import type { CoreSlot } from './types'

export const REG_LABELS = Array.from({ length: 31 }, (_, i) => `X${i}`)

/** How many bytes the memory panel keeps in a window. */
export const MEMORY_VIEW_BYTES = 512

/** How many `cpu.step()` calls a single rAF tick performs in auto-run. */
export const RUN_BURST = 2

/** AArch64 task entries laid out by the demo kernel. */
export const TASK_A_ENTRY = 0x4d00
export const TASK_B_ENTRY = 0x4e00

export const IRQ_NAMES = ['TIMER', 'IPI']

export function fmtHex64(v: bigint): string {
  return '0x' + v.toString(16).padStart(16, '0')
}

export function fmtHex32(v: number): string {
  return '0x' + (v >>> 0).toString(16).padStart(8, '0')
}

/** Parse `0x4000`, `0X4000`, or bare `4000` as a u64. Returns null on
 * empty / malformed input — callers fall back to a safe default. */
export function parseHex(text: string): bigint | null {
  const trimmed = text.trim()
  if (trimmed === '') return null
  try {
    return BigInt(trimmed.startsWith('0x') || trimmed.startsWith('0X') ? trimmed : '0x' + trimmed)
  } catch {
    return null
  }
}

/** Decode the per-core scheduler slot from a 0x100-byte memory window:
 *
 *   +0x00  current task entry (u64)
 *   +0x08  current save-area pointer (u64)
 *   +0x10  save area 0 (X0..X3)
 *   +0x30  save area 1 (X0..X3)
 */
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

/** Best-effort label for a PC — the task it most likely belongs to. */
export function inferTaskLabel(pc: bigint): string | null {
  const p = Number(pc)
  if (p >= TASK_A_ENTRY && p < TASK_A_ENTRY + 0x20) return 'task A'
  if (p >= TASK_B_ENTRY && p < TASK_B_ENTRY + 0x20) return 'task B'
  if (p >= 0x4400 && p < 0x4500) return 'sync handler'
  if (p >= 0x4880 && p < 0x4900) return 'IRQ handler (sched)'
  if (p >= 0x4000 && p < 0x4400) return 'kernel boot'
  return null
}
