import { Card } from '@goliapkg/gds'

import type { BlockState } from '../../sim/types'
import { RegRow } from '../reg-row'

const SECTOR_SIZE = 64

export function BlockPanel({ block }: { block: BlockState }) {
  const numSectors = Math.floor(block.disk.length / SECTOR_SIZE)
  return (
    <Card padding="none">
      <div className="space-y-3 p-4">
        <div className="text-fg-muted flex items-center justify-between">
          <span className="type-small font-semibold tracking-wider uppercase">
            Block device · disk image ({numSectors} × 64-byte sectors @ MMIO 0x3000)
          </span>
          <span className="type-small">
            reads {block.total_reads.toString()} · writes {block.total_writes.toString()}
          </span>
        </div>
        <div className="type-small grid gap-x-6 gap-y-1 sm:grid-cols-4">
          <RegRow label="SECTOR" value={block.sector} />
          <RegRow label="BUF_ADDR" value={block.buf_addr} />
          <RegRow label="CMD" value={block.last_command} />
          <div className="flex justify-between gap-2">
            <span className="text-fg-muted">STATUS</span>
            <span className="text-fg">{describeStatus(block.status)}</span>
          </div>
        </div>
        <div className="space-y-2">
          {Array.from({ length: numSectors }, (_, i) => (
            <SectorBlock
              bytes={new Uint8Array(block.disk.slice(i * SECTOR_SIZE, (i + 1) * SECTOR_SIZE))}
              key={i}
              sector={i}
            />
          ))}
        </div>
      </div>
    </Card>
  )
}

function describeStatus(status: bigint): string {
  if (status === 0n) return 'IDLE'
  if (status === 1n) return 'OK'
  if (status === 2n) return 'FAULT'
  return `0x${status.toString(16)}`
}

function SectorBlock({ bytes, sector }: { bytes: Uint8Array; sector: number }) {
  return (
    <div className="border-border rounded border px-2 py-1.5">
      <div className="text-fg-muted type-small mb-1 tracking-wider uppercase">sector {sector}</div>
      <DiskHexRows bytes={bytes} sector={sector} />
    </div>
  )
}

function DiskHexRows({ bytes, sector }: { bytes: Uint8Array; sector: number }) {
  const rows: Uint8Array[] = []
  for (let i = 0; i < bytes.length; i += 16) {
    rows.push(bytes.slice(i, i + 16))
  }
  const baseAddr = sector * SECTOR_SIZE
  return (
    <div className="mono-data type-small overflow-x-auto">
      {rows.map((row, ri) => {
        let ascii = ''
        for (let i = 0; i < row.length; i++) {
          const c = row[i]
          ascii += c >= 0x20 && c < 0x7f ? String.fromCharCode(c) : '.'
        }
        return (
          <div className="flex gap-4 leading-6" key={ri}>
            <span className="text-fg-muted">
              {(baseAddr + ri * 16).toString(16).padStart(4, '0')}
            </span>
            <span className="text-fg flex-1">
              {Array.from(row, (b) => b.toString(16).padStart(2, '0')).join(' ')}
            </span>
            <span className="text-fg-muted">{ascii}</span>
          </div>
        )
      })}
    </div>
  )
}
