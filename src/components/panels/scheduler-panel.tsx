import { Card } from '@goliapkg/gds'

import type { Process } from '../../sim/types'

const TASK_NAME = ['task A', 'task B'] as const

export function SchedulerPanel({ processes }: { processes: Process[] }) {
  return (
    <Card padding="none">
      <div className="space-y-3 p-4">
        <div className="text-fg-muted flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            Scheduler · PCBs in shared memory
          </span>
          <span className="type-small">round-robin · lockstep swap on each timer IRQ</span>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          {processes.map((p) => (
            <ProcessCard key={p.pid} proc={p} />
          ))}
        </div>
      </div>
    </Card>
  )
}

function ProcessCard({ proc }: { proc: Process }) {
  const name = TASK_NAME[proc.pid] ?? `pid ${proc.pid}`
  return (
    <div className="border-border bg-bg-secondary rounded border px-3 py-2">
      <div className="text-fg-muted type-small mb-2 flex items-center justify-between tracking-wider uppercase">
        <span>
          {name} · PCB at 0x{proc.pcb_pa.toString(16)}
        </span>
        <span className={proc.host_core != null ? 'text-success' : 'text-fg-muted'}>
          {proc.host_core != null ? `running on core ${proc.host_core}` : 'idle'}
        </span>
      </div>
      <div className="mono-data grid grid-cols-2 gap-x-4 gap-y-0.5 text-[11px]">
        <span>
          <span className="text-fg-muted">entry</span> 0x{proc.entry.toString(16).padStart(8, '0')}
        </span>
        <span>
          <span className="text-fg-muted">ELR </span> 0x{proc.elr.toString(16).padStart(8, '0')}
        </span>
        <span>
          <span className="text-fg-muted">SP_EL0</span> 0x
          {proc.sp_el0.toString(16).padStart(4, '0')}
        </span>
        <span>
          <span className="text-fg-muted">SPSR</span> 0x{proc.spsr.toString(16)}
        </span>
        <span>
          <span className="text-fg-muted">X29</span> 0x{proc.fp.toString(16)}
        </span>
        <span>
          <span className="text-fg-muted">X30</span> 0x{proc.lr.toString(16).padStart(4, '0')}
        </span>
      </div>
      <div className="mono-data text-fg-muted mt-2 grid grid-cols-4 gap-x-2 text-[10px]">
        {proc.x.map((v, i) => (
          <span key={i}>
            X{i} {(v & 0xffffn).toString(16).padStart(4, '0')}
          </span>
        ))}
      </div>
    </div>
  )
}
