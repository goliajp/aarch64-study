// Header badges used across the per-core panels — `CoreChip` shows
// `coreN P-core/E-core ELx` and `DaifChip` shows the four DAIF mask bits
// in their masked/enabled state. Both pick their colours from GDS theme
// tokens so they read cleanly in light and dark themes.

import { Badge } from '@goliapkg/gds'

import { fmtHex64 } from '../sim/format'
import type { CoreState } from '../sim/types'

export function CoreChip({ core }: { core: CoreState }) {
  // Each core gets its own palette index (core 0 = palette-0, core 1 =
  // palette-1) so the two are visually distinct without hardcoding hue.
  const variant = core.id === 0 ? 'palette-0' : 'palette-1'
  return (
    <Badge
      className="type-base px-2 py-1"
      title={`MPIDR ${fmtHex64(core.mpidr)}`}
      variant={variant}
    >
      <span className="font-semibold">core{core.id}</span>
      <span className="opacity-70">{core.kind}</span>
      <span className="font-semibold">EL{core.current_el}</span>
    </Badge>
  )
}

/** PSTATE.DAIF mask display — letters in green when *enabled* (i.e. the
 * mask bit is 0 and that interrupt class can fire), muted when masked. */
export function DaifChip({ daif }: { daif: number }) {
  // Bit 3 = D, bit 2 = A, bit 1 = I, bit 0 = F. A 1 here means masked.
  const bits = [
    { name: 'D', set: (daif & 0b1000) !== 0 },
    { name: 'A', set: (daif & 0b0100) !== 0 },
    { name: 'I', set: (daif & 0b0010) !== 0 },
    { name: 'F', set: (daif & 0b0001) !== 0 },
  ]
  return (
    <span
      className="border-border bg-bg-secondary type-base inline-flex items-center gap-1 rounded border px-2 py-1 tracking-wider"
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
