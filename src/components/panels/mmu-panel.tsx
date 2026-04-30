import { Badge, Card } from '@goliapkg/gds'

import { fmtHex64 } from '../../sim/format'
import type { CoreState, TranslationResult, WalkOutcome, WalkStep } from '../../sim/types'
import { RegRow } from '../reg-row'

interface Props {
  cores: CoreState[]
  onCoreChange: (idx: number) => void
  onVaChange: (v: string) => void
  selectedCoreIdx: number
  trace: TranslationResult | null
  vaText: string
}

export function MmuPanel({
  cores,
  onCoreChange,
  onVaChange,
  selectedCoreIdx,
  trace,
  vaText,
}: Props) {
  const sel = cores[selectedCoreIdx]
  const t0sz = Number(sel.tcr_el1 & 0x3fn)
  const vaBits = t0sz > 0 ? 64 - t0sz : 0
  const mmuOn = (sel.sctlr_el1 & 1n) !== 0n

  return (
    <Card padding="none">
      <div className="space-y-4 p-4">
        <div className="text-fg-muted flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            MMU · Stage-1 Translation
          </span>
          <Badge variant={mmuOn ? 'success' : undefined}>SCTLR_EL1.M={mmuOn ? '1' : '0'}</Badge>
        </div>

        <div className="type-small flex items-center gap-2">
          <span className="text-fg-muted">walk using</span>
          {cores.map((c) => (
            <button
              className={`type-small rounded border px-2 py-0.5 ${
                c.id === selectedCoreIdx
                  ? 'border-accent bg-accent/10 text-accent'
                  : 'border-border text-fg-muted hover:bg-bg-tertiary'
              }`}
              key={c.id}
              onClick={() => onCoreChange(c.id)}
              type="button"
            >
              core{c.id} ({c.kind})
            </button>
          ))}
        </div>

        <div className="type-small grid gap-x-6 gap-y-1 sm:grid-cols-3">
          <RegRow label="TTBR0_EL1" value={sel.ttbr0_el1} />
          <RegRow label="TCR_EL1" value={sel.tcr_el1} />
          <RegRow label="SCTLR_EL1" value={sel.sctlr_el1} />
        </div>
        <div className="text-fg-muted type-small">
          T0SZ={t0sz} · VA={vaBits} bits · 4 KiB granule · start level{' '}
          {trace && trace.steps.length > 0 ? trace.steps[0].level : '?'}
        </div>

        <div
          className={`type-small rounded border px-3 py-1.5 ${
            mmuOn ? 'border-success/40 bg-success/10 text-success' : 'border-border text-fg-muted'
          }`}
        >
          {mmuOn
            ? 'Translation active — every fetch and LDR/STR runs through this walk.'
            : 'Translation is a query only — fetches and LDR/STR bypass the MMU until SCTLR_EL1.M=1.'}
        </div>

        <div className="flex items-center gap-2">
          <label className="text-fg-muted type-small" htmlFor="va-input">
            translate VA
          </label>
          <input
            className="mono-data border-border bg-bg-secondary text-fg focus:border-accent type-small w-40 rounded border px-2 py-1 outline-none"
            id="va-input"
            onChange={(e) => onVaChange(e.target.value)}
            placeholder="0x4000"
            spellCheck={false}
            value={vaText}
          />
          <span className="text-fg-muted type-small">
            try 0x4000, 0x1000, 0x4800, 0x4C00, 0x2000
          </span>
        </div>

        {trace && <WalkDisplay trace={trace} />}
      </div>
    </Card>
  )
}

function WalkDisplay({ trace }: { trace: TranslationResult }) {
  if (trace.steps.length === 0 && trace.fault) {
    // "MMU not configured" before kernel boots is expected, not a failure.
    const isPreInit = trace.fault.startsWith('MMU not configured')
    return (
      <div
        className={`type-small rounded border px-3 py-2 ${
          isPreInit ? 'border-border text-fg-muted' : 'border-danger/40 bg-danger/10 text-danger'
        }`}
      >
        {isPreInit
          ? 'MMU not configured yet — kernel will set TTBR0_EL1 + TCR_EL1 during boot.'
          : trace.fault}
      </div>
    )
  }
  return (
    <div className="type-small space-y-2">
      {trace.steps.map((s, i) => (
        <WalkStepRow key={i} step={s} />
      ))}
      <div className="border-border mt-2 flex items-center justify-between border-t pt-2">
        <span className="text-fg-muted">VA {fmtHex64(trace.va)}</span>
        {trace.pa !== null ? (
          <span className="text-accent">→ PA {fmtHex64(trace.pa)}</span>
        ) : (
          <span className="text-danger">{trace.fault ?? 'no PA'}</span>
        )}
      </div>
    </div>
  )
}

function WalkStepRow({ step }: { step: WalkStep }) {
  const tone =
    step.outcome.kind === 'Invalid' || step.outcome.kind === 'Fault' ? 'text-danger' : 'text-fg'
  return (
    <div className={`border-border rounded border px-3 py-2 ${tone}`}>
      <div className="flex flex-wrap items-baseline gap-x-4 gap-y-1">
        <span className="text-accent font-semibold">L{step.level}</span>
        <span className="text-fg-muted">
          table {fmtHex64(step.table_addr)} · idx {step.index}
        </span>
        <span className="text-fg-muted">
          entry@{fmtHex64(step.entry_addr)} = {fmtHex64(step.descriptor)}
        </span>
      </div>
      <div className="text-fg-muted type-small mt-1">
        <OutcomeText outcome={step.outcome} />
      </div>
    </div>
  )
}

function OutcomeText({ outcome }: { outcome: WalkOutcome }) {
  switch (outcome.kind) {
    case 'Table':
      return <>→ next table @ {fmtHex64(outcome.next_table)}</>
    case 'Page':
      return (
        <>
          → page PA {fmtHex64(outcome.pa)} · AF={outcome.attrs.af ? 1 : 0} · AP={outcome.attrs.ap}
        </>
      )
    case 'Block':
      return (
        <>
          → block PA {fmtHex64(outcome.pa)} (span {fmtHex64(outcome.span)}) · AF=
          {outcome.attrs.af ? 1 : 0}
        </>
      )
    case 'Invalid':
      return <>invalid descriptor (V=0)</>
    case 'Fault':
      return <>fault: {outcome.reason}</>
  }
}
