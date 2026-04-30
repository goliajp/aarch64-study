import { Card } from '@goliapkg/gds'

import type { CoreSlot, TaskSave } from '../../sim/types'
import { RegRow, SaveRegRow } from '../reg-row'

const SLOT_BASE = [0x4f00, 0x5000] as const

export function SavePanel({ slots }: { slots: CoreSlot[] }) {
  return (
    <Card padding="none">
      <div className="space-y-2 p-4">
        <div className="text-fg-muted type-small font-semibold tracking-wider uppercase">
          Per-core scheduler slots — entry + save_ptr + 2 save areas each
        </div>
        <div className="grid gap-3">
          {slots.map((slot, i) => (
            <SlotCard base={SLOT_BASE[i] ?? 0} coreId={i} key={i} slot={slot} />
          ))}
        </div>
        <div className="text-fg-muted type-small">
          Each core derives its slot base from MPIDR_EL1: bit 8 (cluster) maps directly to 0x4F00 /
          0x5000. The scheduler reads "current entry" / "current save_ptr", saves X0–X3 there with
          STP, swaps via the (sum − current) trick, restores the other save area with LDP, and ERETs
          into the other task. X3 is preserved across context switches.
        </div>
      </div>
    </Card>
  )
}

function SlotCard({ base, coreId, slot }: { base: number; coreId: number; slot: CoreSlot }) {
  const taskLabel = slot.entry === 0x4d00n ? 'task A' : slot.entry === 0x4e00n ? 'task B' : '?'
  const activeIdx =
    slot.savePtr === BigInt(base + 0x10) ? 0 : slot.savePtr === BigInt(base + 0x30) ? 1 : null
  return (
    <div className="border-border bg-bg-secondary type-small space-y-2 rounded border px-3 py-2">
      <div className="text-fg-muted type-small flex items-center justify-between tracking-wider uppercase">
        <span>
          core {coreId} slot @ 0x{base.toString(16)}
        </span>
        <span className="text-accent normal-case">running {taskLabel}</span>
      </div>
      <div className="grid gap-x-4 gap-y-0.5">
        <RegRow label="entry" value={slot.entry} />
        <RegRow label="save_ptr" value={slot.savePtr} />
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        <SaveAreaCard active={activeIdx === 0} addr={base + 0x10} index={0} save={slot.save0} />
        <SaveAreaCard active={activeIdx === 1} addr={base + 0x30} index={1} save={slot.save1} />
      </div>
    </div>
  )
}

function SaveAreaCard({
  active,
  addr,
  index,
  save,
}: {
  active: boolean
  addr: number
  index: number
  save: TaskSave
}) {
  const x0Char = Number(save.x0 & 0xffn)
  const ascii = x0Char >= 0x20 && x0Char < 0x7f ? `'${String.fromCharCode(x0Char)}'` : ''
  return (
    <div
      className={`min-w-0 rounded border px-2 py-1 ${
        active ? 'border-accent/60 bg-accent/5' : 'border-border'
      }`}
    >
      <div
        className={`type-small mb-1 flex items-center justify-between gap-2 tracking-wider uppercase ${
          active ? 'text-accent' : 'text-fg-muted'
        }`}
      >
        <span className="min-w-0 truncate">
          save {index} · 0x{addr.toString(16)}
        </span>
        {active && <span className="shrink-0">active</span>}
      </div>
      <SaveRegRow label={`X0 ${ascii}`} value={save.x0} />
      <SaveRegRow label="X1" value={save.x1} />
      <SaveRegRow label="X2" value={save.x2} />
      <SaveRegRow highlight label="X3" value={save.x3} />
    </div>
  )
}
