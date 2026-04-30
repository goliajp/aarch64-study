import { Card } from '@goliapkg/gds'

import type { CoreState } from '../../sim/types'

export function ICachePanel({ cores }: { cores: CoreState[] }) {
  return (
    <Card padding="none">
      <div className="space-y-3 p-4">
        <div className="text-fg-muted flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            I-cache · per-core fetch cache
          </span>
          <span className="type-small">4 lines × 8 insns · direct-mapped · PIPT</span>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          {cores.map((c) => (
            <CoreICache core={c} key={c.id} />
          ))}
        </div>
      </div>
    </Card>
  )
}

function CoreICache({ core }: { core: CoreState }) {
  const { icache } = core
  const total = icache.hits + icache.misses
  const hitPct = total > 0n ? Number((icache.hits * 100n) / total) : 0
  return (
    <div className="border-border bg-bg-secondary rounded border px-3 py-2">
      <div className="text-fg-muted type-small mb-2 flex justify-between tracking-wider uppercase">
        <span>
          core {core.id} · {core.kind}
        </span>
        <span className="mono-data">
          hit {hitPct}% · fill {icache.fills.toString()} · ic-ivau {icache.invalidates.toString()}
        </span>
      </div>
      <div className="font-mono text-[11px]">
        <div className="text-fg-muted grid grid-cols-[2rem_5.5rem_4rem_1fr] gap-x-2 pb-1">
          <span>#</span>
          <span>line PA</span>
          <span>valid</span>
          <span>insn[0]</span>
        </div>
        {icache.lines.map((l, i) => (
          <div
            className={`grid grid-cols-[2rem_5.5rem_4rem_1fr] gap-x-2 ${
              l.valid ? 'text-fg' : 'text-fg-muted opacity-50'
            }`}
            key={i}
          >
            <span>{i}</span>
            <span>{l.valid ? '0x' + (l.tag << 5n).toString(16).padStart(5, '0') : '—'}</span>
            <span>{l.valid ? '✓' : '—'}</span>
            <span>{l.valid ? '0x' + (l.insns[0] >>> 0).toString(16).padStart(8, '0') : '—'}</span>
          </div>
        ))}
      </div>
    </div>
  )
}
