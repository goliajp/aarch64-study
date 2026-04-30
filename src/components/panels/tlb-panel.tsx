import { Card } from '@goliapkg/gds'

import type { CoreState } from '../../sim/types'

export function TlbPanel({ cores }: { cores: CoreState[] }) {
  return (
    <Card padding="none">
      <div className="space-y-3 p-4">
        <div className="text-fg-muted flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            TLB · per-core stage-1 translation cache
          </span>
          <span className="type-small">{cores.length}-way · 8-entry RR · ASID-tagged</span>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          {cores.map((c) => (
            <CoreTlb core={c} key={c.id} />
          ))}
        </div>
      </div>
    </Card>
  )
}

function CoreTlb({ core }: { core: CoreState }) {
  const { tlb } = core
  const total = tlb.hits + tlb.misses
  const hitPct = total > 0n ? Number((tlb.hits * 100n) / total) : 0
  return (
    <div className="border-border bg-bg-secondary rounded border px-3 py-2">
      <div className="text-fg-muted type-small mb-2 flex justify-between tracking-wider uppercase">
        <span>
          core {core.id} · {core.kind}
        </span>
        <span className="mono-data">
          hit {hitPct}% · miss {tlb.misses.toString()} · fill {tlb.fills.toString()} · flush{' '}
          {tlb.flushes.toString()}
        </span>
      </div>
      <div className="font-mono text-[11px]">
        <div className="text-fg-muted grid grid-cols-[2rem_5.5rem_3rem_5.5rem_3rem] gap-x-2 pb-1">
          <span>#</span>
          <span>VA page</span>
          <span>ASID</span>
          <span>PA page</span>
          <span>AP</span>
        </div>
        {tlb.entries.map((e, i) => (
          <div
            className={`grid grid-cols-[2rem_5.5rem_3rem_5.5rem_3rem] gap-x-2 ${
              e.valid ? 'text-fg' : 'text-fg-muted opacity-50'
            }`}
            key={i}
          >
            <span>{i}</span>
            <span>{e.valid ? '0x' + e.va_page.toString(16).padStart(5, '0') : '—'}</span>
            <span>{e.valid ? '0x' + e.asid.toString(16).padStart(2, '0') : '—'}</span>
            <span>{e.valid ? '0x' + e.pa_page.toString(16).padStart(5, '0') : '—'}</span>
            <span>{e.valid ? e.ap.toString(2).padStart(2, '0') : '—'}</span>
          </div>
        ))}
      </div>
    </div>
  )
}
