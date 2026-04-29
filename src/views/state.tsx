import { Card, GlassButton, GlassCard } from '@goliapkg/gds'
import { atom, useAtomValue, useSetAtom } from 'jotai'

// atoms
const countAtom = atom(0)
const doubleAtom = atom((get) => get(countAtom) * 2)
const historyAtom = atom<number[]>([])

// derived write atom: increment and record history
const incrementAtom = atom(null, (get, set) => {
  const next = get(countAtom) + 1
  set(countAtom, next)
  set(historyAtom, [...get(historyAtom), next])
})

const resetAtom = atom(null, (_get, set) => {
  set(countAtom, 0)
  set(historyAtom, [])
})

export function StateView() {
  const count = useAtomValue(countAtom)
  const double = useAtomValue(doubleAtom)
  const history = useAtomValue(historyAtom)
  const increment = useSetAtom(incrementAtom)
  const reset = useSetAtom(resetAtom)

  return (
    <div className="space-y-8">
      <div>
        <h1
          className="text-fg text-2xl font-bold"
          style={{ textShadow: '0 0 20px var(--gds-accent, #3b82f6)' }}
        >
          State Management
        </h1>
        <p className="text-fg-muted mt-1">Jotai atoms with derived and write-only patterns.</p>
      </div>

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <MetricCard label="Count" value={count} />
        <MetricCard label="Double (derived)" value={double} />
        <MetricCard label="Total clicks" value={history.length} />
      </div>

      <div className="flex gap-3">
        <GlassButton onClick={increment} size="sm" variant="accent">
          Increment
        </GlassButton>
        <GlassButton onClick={reset} size="sm">
          Reset
        </GlassButton>
      </div>

      {history.length > 0 && (
        <Card>
          <div className="p-4">
            <h3 className="text-fg mb-2 text-sm font-semibold">History</h3>
            <div className="text-fg-muted flex flex-wrap gap-1.5 font-mono text-xs">
              {history.map((v, i) => (
                <span className="bg-bg-tertiary rounded px-1.5 py-0.5" key={i}>
                  {v}
                </span>
              ))}
            </div>
          </div>
        </Card>
      )}

      <Card>
        <div className="p-4">
          <h3 className="text-fg mb-2 text-sm font-semibold">Pattern Reference</h3>
          <div className="text-fg-muted space-y-1 text-xs">
            <p>
              <code className="text-accent">countAtom</code> — primitive read/write atom
            </p>
            <p>
              <code className="text-accent">doubleAtom</code> — derived read-only atom
            </p>
            <p>
              <code className="text-accent">incrementAtom</code> — write-only atom (action)
            </p>
            <p>
              <code className="text-accent">useAtomValue</code> — read without subscribe to setter
            </p>
            <p>
              <code className="text-accent">useSetAtom</code> — write without subscribe to value
            </p>
          </div>
        </div>
      </Card>
    </div>
  )
}

function MetricCard({ label, value }: { label: string; value: number }) {
  return (
    <GlassCard className="glow-card text-center">
      <div className="p-4">
        <div className="text-fg text-3xl font-bold tabular-nums">{value}</div>
        <div className="text-fg-muted mt-1 text-xs">{label}</div>
      </div>
    </GlassCard>
  )
}
