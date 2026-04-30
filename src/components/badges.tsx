import { Badge } from '@goliapkg/gds'

import { fmtHex64 } from '../sim/format'
import type { CoreState } from '../sim/types'

export function CoreChip({ core }: { core: CoreState }) {
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

/** PSTATE.DAIF — letter green when *enabled* (mask bit 0), muted when masked. */
export function DaifChip({ daif }: { daif: number }) {
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
