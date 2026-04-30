import { Button, Card } from '@goliapkg/gds'
import { type Cpu } from 'aarch64-sim'
import { useMemo } from 'react'

import { fmtHex32 } from '../sim/format'
import type { AicState, BlockState, CoreState, SystemInfo } from '../sim/types'

interface Props {
  aic: AicState
  block: BlockState
  cores: CoreState[]
  cpu: Cpu
  onRefresh: (cpu: Cpu) => void
  onReset: () => void
  onRunToggle: () => void
  onStep: () => void
  running: boolean
  sysInfo: SystemInfo
  totalCoreSteps: bigint
  uartBytes: number
}

export function ControlBar({
  aic,
  block,
  cores,
  cpu,
  onRefresh,
  onReset,
  onRunToggle,
  onStep,
  running,
  sysInfo,
  totalCoreSteps,
  uartBytes,
}: Props) {
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <Button onClick={onStep} size="sm" variant="primary">
          Step both
        </Button>
        <Button onClick={onRunToggle} size="sm" variant="secondary">
          {running ? 'Pause' : 'Run'}
        </Button>
        <Button onClick={onReset} size="sm" variant="secondary">
          Reset
        </Button>
      </div>
      <Card padding="none">
        <div className="grid grid-cols-1 gap-3 p-3 lg:grid-cols-2">
          <DiskEditor block={block} cpu={cpu} onRefresh={onRefresh} />
          <Stats
            aic={aic}
            block={block}
            cores={cores}
            info={sysInfo}
            totalCoreSteps={totalCoreSteps}
            uartBytes={uartBytes}
          />
        </div>
      </Card>
    </div>
  )
}

interface StatsProps {
  aic: AicState
  block: BlockState
  cores: CoreState[]
  info: SystemInfo
  totalCoreSteps: bigint
  uartBytes: number
}

function Stats({ aic, block, cores, info, totalCoreSteps, uartBytes }: StatsProps) {
  const aicPending = aic.pending.reduce((a, b) => a | b, 0)
  const stats: { label: string; value: string; emphasize?: boolean }[] = [
    { label: 'system steps', value: info.systemSteps.toString() },
    { label: 'retired', value: totalCoreSteps.toString() },
    { label: 'timer period', value: info.timerPeriod.toString() },
    {
      label: 'next IRQ',
      value: info.timerRemaining.toString(),
      emphasize: info.timerRemaining === 0n,
    },
    { label: 'timer ticks', value: info.timerTicks.toString() },
    { label: 'aic acks', value: aic.total_acks.toString() },
    { label: 'aic pending', value: '0x' + aicPending.toString(16).padStart(2, '0') },
    { label: 'uart bytes', value: uartBytes.toString() },
    { label: 'blk reads', value: block.total_reads.toString() },
    { label: 'core 0 pc', value: fmtHex32(Number(cores[0]?.pc ?? 0n)) },
    { label: 'core 1 pc', value: fmtHex32(Number(cores[1]?.pc ?? 0n)) },
    { label: 'cores el', value: `${cores[0]?.current_el ?? 0} / ${cores[1]?.current_el ?? 0}` },
  ]
  return (
    <div className="grid grid-cols-3 gap-2 sm:grid-cols-6">
      {stats.map((s) => (
        <Stat emphasize={s.emphasize} key={s.label} label={s.label} value={s.value} />
      ))}
    </div>
  )
}

function Stat({ emphasize, label, value }: { emphasize?: boolean; label: string; value: string }) {
  return (
    <div className="border-border bg-bg-secondary flex flex-col rounded border px-2 py-1">
      <span className="text-fg-muted type-small tracking-wider uppercase">{label}</span>
      <span
        className={`mono-data type-base truncate ${emphasize ? 'text-warning' : 'text-fg'}`}
        title={value}
      >
        {value}
      </span>
    </div>
  )
}

function DiskEditor({
  block,
  cpu,
  onRefresh,
}: {
  block: BlockState
  cpu: Cpu
  onRefresh: (cpu: Cpu) => void
}) {
  const initial = useMemo(() => {
    let end = 64
    for (let i = 0; i < 64; i++) {
      if (block.disk[i] === 0) {
        end = i
        break
      }
    }
    const u8 = new Uint8Array(block.disk.slice(0, end))
    return new TextDecoder('utf-8', { fatal: false }).decode(u8)
  }, [block.disk])
  const apply = (next: string) => {
    cpu.set_disk_text(next)
    onRefresh(cpu)
  }
  return (
    <label className="flex flex-1 flex-col gap-1">
      <span className="text-fg-muted type-small">
        disk sector 0 — task B prints this ({initial.length}/64 bytes)
      </span>
      <textarea
        className="mono-data border-border bg-bg-tertiary text-fg focus:border-accent type-base min-h-16 w-full resize-y rounded border px-2 py-1 outline-none"
        defaultValue={initial}
        key={initial}
        maxLength={64}
        onChange={(e) => apply(e.target.value)}
        rows={3}
        spellCheck={false}
      />
    </label>
  )
}
