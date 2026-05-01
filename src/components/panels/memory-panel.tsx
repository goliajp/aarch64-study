import { Card } from '@goliapkg/gds'

import { fmtHex32 } from '../../sim/format'

interface Props {
  base: number
  bytes: Uint8Array
  pcs: number[]
}

export function MemoryPanel({ base, bytes, pcs }: Props) {
  const rows: { addr: number; bytes: Uint8Array }[] = []
  for (let off = 0; off < bytes.length; off += 16) {
    rows.push({ addr: base + off, bytes: bytes.slice(off, off + 16) })
  }
  return (
    <Card className="h-full" padding="none">
      <div className="flex h-full flex-col p-4">
        <div className="text-fg-muted mb-3 flex flex-wrap items-center justify-between gap-x-3">
          <span className="type-small font-semibold tracking-wider uppercase">
            Memory (around core 0 PC) — shared
          </span>
          <span className="type-small">
            base {fmtHex32(base)} · pc {pcs.map((pc, i) => `c${i}=${fmtHex32(pc)}`).join(' · ')}
          </span>
        </div>
        <div className="mono-data type-small min-h-0 flex-1 overflow-auto whitespace-nowrap">
          {rows.map((r) => (
            <MemoryRow addr={r.addr} bytes={r.bytes} key={r.addr} pcs={pcs} />
          ))}
        </div>
      </div>
    </Card>
  )
}

function MemoryRow({ addr, bytes, pcs }: { addr: number; bytes: Uint8Array; pcs: number[] }) {
  const cells: React.ReactNode[] = []
  for (let i = 0; i < bytes.length; i++) {
    const byteAddr = addr + i
    let cls = 'text-fg'
    if (pcs[0] !== undefined && byteAddr >= pcs[0] && byteAddr < pcs[0] + 4) {
      cls = 'text-accent bg-accent/10'
    } else if (pcs[1] !== undefined && byteAddr >= pcs[1] && byteAddr < pcs[1] + 4) {
      cls = 'bg-violet-500/15 text-violet-700 dark:text-violet-300'
    }
    cells.push(
      <span className={cls} key={i}>
        {bytes[i].toString(16).padStart(2, '0')}
      </span>
    )
    if (i !== bytes.length - 1) cells.push(<span key={`s${i}`}> </span>)
  }
  let ascii = ''
  for (let i = 0; i < bytes.length; i++) {
    const c = bytes[i]
    ascii += c >= 0x20 && c < 0x7f ? String.fromCharCode(c) : '.'
  }
  return (
    <div className="flex gap-4 leading-6">
      <span className="text-fg-muted">{fmtHex32(addr)}</span>
      <span className="flex-1">{cells}</span>
      <span className="text-fg-muted">{ascii}</span>
    </div>
  )
}
