import { Badge, Card, GlassButton } from '@goliapkg/gds'

import { REG_LABELS, fmtHex32, fmtHex64, inferTaskLabel } from '../../sim/format'
import type { CoreState } from '../../sim/types'
import { CoreChip, DaifChip } from '../badges'
import { RegRow } from '../reg-row'

interface Props {
  core: CoreState
  onStep: () => void
}

export function CoreMonitor({ core, onStep }: Props) {
  const taskLabel = inferTaskLabel(core.pc)
  return (
    <Card padding="none">
      <div className="space-y-3 p-3">
        <div className="flex items-center justify-between">
          <div className="flex flex-wrap items-center gap-2">
            <CoreChip core={core} />
            <DaifChip daif={core.daif} />
            {taskLabel && <Badge className="type-base px-2 py-1">{taskLabel}</Badge>}
            {core.wfi_halted && (
              <Badge className="type-base px-2 py-1" variant="info">
                WFI · sleeping
              </Badge>
            )}
            {core.halted && (
              <Badge
                className="type-base px-2 py-1"
                variant={core.last_trap ? 'danger' : 'success'}
              >
                {core.last_trap ? 'TRAP' : 'HALTED'}
              </Badge>
            )}
          </div>
          <div className="flex items-center gap-2">
            <span className="text-fg-muted type-small">steps {core.steps.toString()}</span>
            <GlassButton onClick={onStep} size="sm">
              Step
            </GlassButton>
          </div>
        </div>
        {core.last_trap && (
          <div className="border-danger/40 bg-danger/10 text-danger type-small rounded border px-2 py-1">
            {core.last_trap}
          </div>
        )}
        <RegistersSection core={core} />
        <ExceptionSection core={core} />
      </div>
    </Card>
  )
}

function RegistersSection({ core }: { core: CoreState }) {
  return (
    <div className="border-border bg-bg-secondary rounded border p-3">
      <div className="text-fg-muted type-base mb-2 font-semibold tracking-wider uppercase">
        Registers
      </div>
      <div className="type-base grid grid-cols-2 gap-x-4 gap-y-1">
        {REG_LABELS.map((label, i) => (
          <RegRow key={label} label={label} value={core.x[i]} />
        ))}
        <RegRow label="SP_EL0" value={core.sp_el0} />
        <RegRow label="SP_EL1" value={core.sp_el1} />
        <RegRow highlight label="PC" value={core.pc} />
      </div>
      <div className="text-fg-muted type-small mt-3">
        NZCV: <span>{fmtHex32(core.nzcv)}</span> · MPIDR_EL1: <span>{fmtHex64(core.mpidr)}</span>
      </div>
    </div>
  )
}

function ExceptionSection({ core }: { core: CoreState }) {
  // ESR_EL1=0 with current_el=1 typically means we entered via IRQ (no syndrome).
  const inIrq = core.current_el === 1 && core.esr_el1 === 0n && core.elr_el1 !== 0n
  const active = inIrq || core.esr_el1 !== 0n
  return (
    <div className="border-border bg-bg-secondary space-y-2 rounded border p-3">
      <div className="text-fg-muted type-base flex items-center gap-2 font-semibold tracking-wider uppercase">
        <span
          aria-label={active ? 'handling exception' : 'idle'}
          className={`inline-block h-1.5 w-1.5 rounded-full transition-colors ${
            active ? 'bg-emerald-400' : 'bg-fg-muted/40'
          }`}
        />
        Exception state
      </div>
      <div className="type-base grid gap-x-4 gap-y-1 sm:grid-cols-2">
        <div className="text-fg-muted type-base col-span-full tracking-wider uppercase">EL2</div>
        <RegRow label="VBAR_EL2" value={core.vbar_el2} />
        <RegRow label="ELR_EL2" value={core.elr_el2} />
        <RegRow label="SPSR_EL2" value={core.spsr_el2} />
        <RegRow label="ESR_EL2" value={core.esr_el2} />
        <div className="text-fg-muted type-base col-span-full mt-1 tracking-wider uppercase">
          EL1
        </div>
        <RegRow label="VBAR_EL1" value={core.vbar_el1} />
        <RegRow label="ELR_EL1" value={core.elr_el1} />
        <RegRow label="SPSR_EL1" value={core.spsr_el1} />
        <RegRow label="ESR_EL1" value={core.esr_el1} />
      </div>
    </div>
  )
}
