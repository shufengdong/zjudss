import type { SimulationRun } from '../../types'

const STUDY_TITLES: Record<string, string> = {
  ds_radial_pf: '静态潮流',
  ds_newton_raphson_pf: '静态潮流',
  ds_radial_qsts: '时序潮流',
  ds_newton_raphson_qsts: '时序潮流',
  ds_fault_study: '短路计算',
  ds_harmonic_study: '谐波分析',
  ds_motor_dynamics: '电机暂态',
}

const STATUS_LABELS: Record<SimulationRun['status'], string> = {
  queued: '排队中',
  running: '运行中',
  succeeded: '成功',
  failed: '失败',
}

const STAGE_LABELS: Record<string, string> = {
  prepare: '准备输入',
  execute: '执行计算',
  collect: '收集结果',
  completed: '已完成',
}

function boundedText(value: unknown, limit = 500): string | undefined {
  if (typeof value !== 'string' || !value.trim()) return undefined
  const characters = Array.from(value.trim())
  return characters.length > limit ? `${characters.slice(0, limit).join('')}…` : characters.join('')
}

function timestamp(value: string | null): number | null {
  if (!value) return null
  const parsed = Date.parse(value)
  return Number.isFinite(parsed) ? parsed : null
}

function decimal(value: number): string {
  return Number(value.toFixed(1)).toString()
}

export function runTitle(run: SimulationRun): string {
  if (Array.isArray(run.agentSkillRefs)) {
    for (const reference of run.agentSkillRefs) {
      if (!reference || typeof reference !== 'object' || Array.isArray(reference)) continue
      const candidate = reference as Record<string, unknown>
      if (candidate.kind === 'study-plan' && typeof candidate.function === 'string') {
        const title = STUDY_TITLES[candidate.function]
        if (title) return title
      }
    }
  }
  return run.scriptName
}

export function runStatusLabel(status: SimulationRun['status']): string {
  return STATUS_LABELS[status]
}

export function runStageLabel(stage: string): string {
  const normalized = stage.trim()
  return normalized ? (STAGE_LABELS[normalized] ?? normalized) : '未知阶段'
}

export function runDurationMs(run: SimulationRun, nowMs = Date.now()): number | null {
  const start = run.startedAt === null ? timestamp(run.createdAt) : timestamp(run.startedAt)
  const end = run.status === 'succeeded' || run.status === 'failed'
    ? timestamp(run.finishedAt)
    : nowMs
  if (start === null || end === null || !Number.isFinite(end) || end < start) return null
  return end - start
}

export function formatDuration(milliseconds: number | null): string {
  if (milliseconds === null || !Number.isFinite(milliseconds) || milliseconds < 0) return '—'
  if (milliseconds < 60_000) return `${decimal(milliseconds / 1_000)} 秒`
  const minutes = Math.floor(milliseconds / 60_000)
  const seconds = Math.floor((milliseconds % 60_000) / 1_000)
  return seconds ? `${minutes} 分 ${seconds} 秒` : `${minutes} 分`
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—'
  if (bytes < 1_024) return `${Math.round(bytes)} B`
  const units = ['KiB', 'MiB', 'GiB', 'TiB']
  let value = bytes / 1_024
  let unit = 0
  while (value >= 1_024 && unit < units.length - 1) {
    value /= 1_024
    unit += 1
  }
  return `${decimal(value)} ${units[unit]}`
}

export function runFailure(run: SimulationRun): { code?: string; message: string } {
  if (!run.error || typeof run.error !== 'object' || Array.isArray(run.error)) {
    return { message: '运行失败，未提供可用原因' }
  }
  const error = run.error as Record<string, unknown>
  const code = boundedText(error.code, 128)
  const message = boundedText(error.message)
  return {
    ...(code ? { code } : {}),
    message: message ?? '运行失败，未提供可用原因',
  }
}
