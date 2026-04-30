// React-side mirrors of the wasm-bindgen interfaces exported by aarch64-sim.

export interface CoreState {
  id: number
  kind: string
  mpidr: bigint
  x: bigint[]
  sp: bigint
  pc: bigint
  nzcv: number
  halted: boolean
  last_trap: string | null
  steps: bigint
  current_el: number
  daif: number
  wfi_halted: boolean
  ttbr0_el1: bigint
  tcr_el1: bigint
  sctlr_el1: bigint
  vbar_el1: bigint
  elr_el1: bigint
  spsr_el1: bigint
  esr_el1: bigint
  vbar_el2: bigint
  elr_el2: bigint
  spsr_el2: bigint
  esr_el2: bigint
}

export interface AicState {
  pending: number[]
  total_acks: bigint
  total_ipis: bigint
  last_ipi_target: number | null
}

export interface BlockState {
  sector: bigint
  buf_addr: bigint
  last_command: bigint
  status: bigint
  total_reads: bigint
  total_writes: bigint
  disk: number[]
}

export interface TaskSave {
  x0: bigint
  x1: bigint
  x2: bigint
  x3: bigint
}

export interface CoreSlot {
  entry: bigint
  savePtr: bigint
  save0: TaskSave
  save1: TaskSave
}

export interface SystemInfo {
  systemSteps: bigint
  timerPeriod: bigint
  timerRemaining: bigint
  timerTicks: bigint
  atomicCounter: bigint
}

export type NodeId = 'core0' | 'core1' | 'aic' | 'uart' | 'block' | 'ram'

export type SimEventKind = 'store' | 'timer' | 'disk_read' | 'irq_taken' | 'svc' | 'eret' | 'ipi'

export interface SimEvent {
  id: number
  kind: SimEventKind
  source: NodeId
  target: NodeId
  ts: number
}

export interface PrevSnapshot {
  cores: { pc: bigint; current_el: number; wfi_halted: boolean }[]
  outputLen: number
  ticks: bigint
  totalReads: bigint
  totalIpis: bigint
}

export interface PageAttrs {
  af: boolean
  ap: number
  attr_idx: number
  sh: number
}

export type WalkOutcome =
  | { kind: 'Table'; next_table: bigint }
  | { kind: 'Page'; pa: bigint; attrs: PageAttrs }
  | { kind: 'Block'; pa: bigint; attrs: PageAttrs; span: bigint }
  | { kind: 'Invalid' }
  | { kind: 'Fault'; reason: string }

export interface WalkStep {
  level: number
  table_addr: bigint
  index: number
  entry_addr: bigint
  descriptor: bigint
  outcome: WalkOutcome
}

export interface TranslationResult {
  va: bigint
  steps: WalkStep[]
  pa: bigint | null
  fault: string | null
  mmu_enabled: boolean
}
