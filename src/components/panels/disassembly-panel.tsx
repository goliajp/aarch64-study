import { Card } from '@goliapkg/gds'
import { type Cpu, disassemble } from 'aarch64-sim'

import { fmtHex32 } from '../../sim/format'
import type { CoreState } from '../../sim/types'

const WINDOW_INSTS = 36
const PRE_INSTS = 16

interface Props {
  cpu: Cpu
  cores: CoreState[]
}

export function DisassemblyPanel({ cpu, cores }: Props) {
  const pc0 = Number(cores[0].pc)
  const pc1 = Number(cores[1].pc)
  // Snap the window to a 32-byte boundary so the listing doesn't jitter line-by-line.
  const center = pc0 & ~0x1f
  const start = Math.max(0x4000, center - PRE_INSTS * 4)
  const end = Math.min(0x10000, start + WINDOW_INSTS * 4)
  const bytes = cpu.mem_slice(start, end - start)
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)

  const rows: { pa: number; word: number; mnem: string }[] = []
  for (let i = 0; i + 4 <= bytes.length; i += 4) {
    const word = view.getUint32(i, true)
    const pa = start + i
    rows.push({ pa, word, mnem: disassemble(word, BigInt(pa)) })
  }

  return (
    <Card padding="none">
      <div className="space-y-2 p-4">
        <div className="text-fg-muted flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            Disassembly · around core 0 PC
          </span>
          <span className="type-small">
            range {fmtHex32(start)}–{fmtHex32(end - 1)}
          </span>
        </div>
        <div className="mono-data type-small overflow-x-auto">
          {rows.map((r) => (
            <DisassemblyRow key={r.pa} pa0={pc0} pa1={pc1} row={r} />
          ))}
        </div>
      </div>
    </Card>
  )
}

interface RowData {
  pa: number
  word: number
  mnem: string
}

function DisassemblyRow({ pa0, pa1, row }: { pa0: number; pa1: number; row: RowData }) {
  const isCore0 = row.pa === pa0
  const isCore1 = row.pa === pa1
  const isPc = isCore0 || isCore1
  const bg = isCore0 ? 'bg-cyan-500/10 text-fg' : isCore1 ? 'bg-violet-500/10 text-fg' : ''
  const markerColor = isCore0
    ? 'live-pulse text-cyan-700 dark:text-cyan-300'
    : isCore1
      ? 'live-pulse text-violet-700 dark:text-violet-300'
      : 'text-fg-muted'
  const colColor = isPc ? 'text-fg' : 'text-fg-muted'
  return (
    <div className={`flex gap-3 leading-6 ${bg}`}>
      <span className={`flex w-12 shrink-0 items-center gap-1 ${markerColor}`}>
        {isPc && <span>►</span>}
        {isCore0 ? <span>0</span> : isCore1 ? <span>1</span> : null}
      </span>
      <span className={`${colColor} w-16 shrink-0`}>{fmtHex32(row.pa)}</span>
      <span className={`${colColor} w-20 shrink-0`}>{row.word.toString(16).padStart(8, '0')}</span>
      <span className="flex-1">{row.mnem}</span>
    </div>
  )
}
