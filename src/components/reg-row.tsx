import { fmtHex64 } from '../sim/format'

interface Props {
  highlight?: boolean
  label: string
  value: bigint
}

export function RegRow({ highlight, label, value }: Props) {
  return (
    <div className={`flex justify-between gap-2 ${highlight ? 'text-accent' : ''}`}>
      <span className="text-fg-muted">{label}</span>
      <span className={`mono-data ${value === 0n ? 'text-fg-muted' : 'text-fg'}`}>
        {fmtHex64(value)}
      </span>
    </div>
  )
}

/** Compact 32-bit variant for the per-core save areas (column too narrow for 64-bit). */
export function SaveRegRow({ highlight, label, value }: Props) {
  return (
    <div className={`type-small flex justify-between gap-2 ${highlight ? 'text-accent' : ''}`}>
      <span className="text-fg-muted shrink-0">{label}</span>
      <span className={`mono-data truncate ${value === 0n ? 'text-fg-muted' : 'text-fg'}`}>
        0x{(Number(value & 0xffffffffn) >>> 0).toString(16).padStart(8, '0')}
      </span>
    </div>
  )
}
