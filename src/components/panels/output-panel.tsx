import { Card } from '@goliapkg/gds'
import { useEffect, useRef } from 'react'

export function OutputPanel({ output }: { output: string }) {
  const ref = useRef<HTMLPreElement>(null)
  useEffect(() => {
    if (ref.current) ref.current.scrollTop = ref.current.scrollHeight
  }, [output])
  return (
    <Card className="h-full" padding="none">
      <div className="flex h-full flex-col p-4">
        <div className="text-fg-muted type-small mb-3 font-semibold tracking-wider uppercase">
          UART Output (PA 0x1000) — shared
        </div>
        <pre
          className="mono-data text-fg type-base min-h-[1.45em] flex-1 overflow-y-auto leading-[1.45] whitespace-pre-wrap"
          ref={ref}
        >
          {output || <span className="text-fg-muted">(no output yet)</span>}
        </pre>
      </div>
    </Card>
  )
}
