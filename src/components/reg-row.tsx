// Shared register-row primitives used by every panel that displays a
// labelled hex value. Two flavours: the full 64-bit RegRow used in the
// registers / exception / MMU panels, and the compact 32-bit SaveRegRow
// used inside the per-core scheduler save areas (where space is tight).

import { fmtHex64 } from '../sim/format'

interface RegRowProps {
  /** Highlights the value with the accent colour — used for PC/X3. */
  highlight?: boolean
  label: string
  value: bigint
}

export function RegRow({ highlight, label, value }: RegRowProps) {
  return (
    <div className={`flex justify-between gap-2 ${highlight ? 'text-accent' : ''}`}>
      <span className="text-fg-muted">{label}</span>
      <span className={`mono-data ${value === 0n ? 'text-fg-muted' : 'text-fg'}`}>
        {fmtHex64(value)}
      </span>
    </div>
  )
}

/** Truncated 32-bit variant for the per-core save areas — the column is
 * narrow, so we drop the upper 32 bits (always zero for ASCII saves). */
export function SaveRegRow({ highlight, label, value }: RegRowProps) {
  return (
    <div className={`type-small flex justify-between gap-2 ${highlight ? 'text-accent' : ''}`}>
      <span className="text-fg-muted shrink-0">{label}</span>
      <span className={`mono-data truncate ${value === 0n ? 'text-fg-muted' : 'text-fg'}`}>
        0x{(Number(value & 0xffffffffn) >>> 0).toString(16).padStart(8, '0')}
      </span>
    </div>
  )
}
