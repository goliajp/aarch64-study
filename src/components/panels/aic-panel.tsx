import { Card } from '@goliapkg/gds'

import { IRQ_NAMES } from '../../sim/format'
import type { AicState } from '../../sim/types'

export function AicPanel({ aic }: { aic: AicState }) {
  return (
    <Card padding="none">
      <div className="space-y-2 p-4">
        <div className="text-fg-muted flex flex-wrap items-center justify-between gap-x-3">
          <span className="type-small font-semibold tracking-wider uppercase">
            AIC · Apple-style interrupt controller
          </span>
          <span className="type-small">
            base 0x2000 · ACK reads cleared {aic.total_acks.toString()} · IPIs sent{' '}
            {aic.total_ipis.toString()}
            {aic.last_ipi_target != null ? ` → core ${aic.last_ipi_target}` : ''}
          </span>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          {aic.pending.map((bits, i) => (
            <PendingBlock bits={bits} core={i} key={i} />
          ))}
        </div>
      </div>
    </Card>
  )
}

function PendingBlock({ bits, core }: { bits: number; core: number }) {
  return (
    <div className="border-border bg-bg-secondary type-small rounded border px-3 py-2">
      <div className="text-fg-muted type-small mb-1 tracking-wider uppercase">
        core {core} pending
      </div>
      <div className="flex items-center gap-2">
        <span className="mono-data text-fg">0x{bits.toString(16).padStart(8, '0')}</span>
        <span className="flex gap-1">
          {IRQ_NAMES.map((name, b) => {
            const isSet = (bits & (1 << b)) !== 0
            return (
              <span
                className={
                  isSet
                    ? 'rounded border border-amber-500/60 bg-amber-500/20 px-1.5 py-0.5 text-amber-900 dark:text-amber-100'
                    : 'border-border text-fg-muted rounded border px-1.5 py-0.5'
                }
                key={name}
              >
                {name}
              </span>
            )
          })}
        </span>
      </div>
    </div>
  )
}
