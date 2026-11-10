import type {
  EngineLog,
  FailureAnalysis,
  FailureAnalysisEvidence,
  FailureAnalysisStatus,
  FailureDiagnosis,
  SimulationRun,
} from '../../types'

const STUDY_TITLES = new Map<string, string>([
  ['ds_radial_pf', '静态潮流'],
  ['ds_newton_raphson_pf', '静态潮流'],
  ['ds_radial_qsts', '时序潮流'],
  ['ds_newton_raphson_qsts', '时序潮流'],
  ['ds_fault_study', '短路计算'],
  ['ds_harmonic_study', '谐波分析'],
  ['ds_motor_dynamics', '电机暂态'],
])

const STATUS_LABELS: Record<SimulationRun['status'], string> = {
  queued: '排队中',
  running: '运行中',
  succeeded: '成功',
  failed: '失败',
}

const STAGE_LABELS = new Map<string, string>([
  ['queued', '排队等待'],
  ['prepare', '准备输入'],
  ['run', '执行计算'],
  ['execute', '执行计算'],
  ['collect', '收集结果'],
  ['completed', '已完成'],
  ['failed', '未知失败阶段'],
  ['recovery', '恢复中断'],
])

function boundedText(value: unknown, limit = 500): string | undefined {
  if (typeof value !== 'string' || !value.trim()) return undefined
  const characters = Array.from(value.trim())
  return characters.length > limit ? `${characters.slice(0, limit).join('')}…` : characters.join('')
}

function boundedLogText(value: unknown, limit: number): string | undefined {
  if (typeof value !== 'string') return undefined
  const characters = Array.from(value)
  return characters.length > limit ? `${characters.slice(0, limit).join('')}…` : value
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
        const title = STUDY_TITLES.get(candidate.function)
        if (title) return title
      }
    }
  }
  return run.scriptName
}

export function isRunCancelled(run: SimulationRun): boolean {
  if (run.status !== 'failed') return false
  if (run.stage === 'cancelled_by_user') return true
  if (!run.error || typeof run.error !== 'object' || Array.isArray(run.error)) return false
  return (run.error as Record<string, unknown>).code === 'cancelled_by_user'
}

export function runStatusLabel(run: SimulationRun): string {
  if (isRunCancelled(run)) return '已取消'
  return Object.prototype.hasOwnProperty.call(STATUS_LABELS, run.status)
    ? STATUS_LABELS[run.status]
    : '未知状态'
}

export function runStageLabel(stage: string): string {
  const normalized = stage.trim()
  return normalized ? (STAGE_LABELS.get(normalized) ?? normalized) : '未知阶段'
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

export function formatTimestamp(value: string | null): string {
  const milliseconds = timestamp(value)
  return milliseconds === null ? '—' : new Date(milliseconds).toLocaleString()
}

export function runFailure(run: SimulationRun): { code?: string; message: string; engineOutput?: string } {
  const diagnosis = parseRunFailureDiagnosis(run)
  const engineOutput = diagnosis.engineLogs.find((log) => log.stream === 'stderr')?.text
  return {
    ...(diagnosis.code ? { code: diagnosis.code } : {}),
    ...(engineOutput ? { engineOutput } : {}),
    message: diagnosis.message,
  }
}

function publicByteCount(value: unknown): number | null {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null
}

function parseEngineLogs(value: unknown): EngineLog[] {
  if (!Array.isArray(value)) return []
  const seen = new Set<string>()
  const logs: EngineLog[] = []
  for (const candidate of value) {
    if (!candidate || typeof candidate !== 'object' || Array.isArray(candidate)) continue
    const log = candidate as Record<string, unknown>
    if ((log.stream !== 'stdout' && log.stream !== 'stderr') || seen.has(log.stream)) continue
    const text = boundedLogText(log.text, 65_536)
    if (text === undefined) continue
    seen.add(log.stream)
    logs.push({
      stream: log.stream,
      text: text ?? '',
      totalBytes: publicByteCount(log.totalBytes),
      sentBytes: publicByteCount(log.sentBytes),
      truncated: log.truncated === true,
    })
  }
  return logs
}

function parseEvidence(value: unknown): FailureAnalysisEvidence[] | null {
  if (!Array.isArray(value) || value.length > 5) return null
  const parsed: FailureAnalysisEvidence[] = []
  for (const candidate of value) {
    if (!candidate || typeof candidate !== 'object' || Array.isArray(candidate)) return null
    const item = candidate as Record<string, unknown>
    const statement = boundedText(item.statement, 800)
    const evidence = boundedText(item.evidence, 800)
    if (!statement || !evidence) return null
    parsed.push({ statement, evidence })
  }
  return parsed
}

function parseTextItems(value: unknown): string[] | null {
  if (!Array.isArray(value) || value.length > 5) return null
  const parsed = value.map((item) => boundedText(item, 800))
  return parsed.every((item): item is string => Boolean(item)) ? parsed : null
}

function parseFailureAnalysis(value: unknown): FailureAnalysis | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const candidate = value as Record<string, unknown>
  const summary = boundedText(candidate.summary, 500)
  const facts = parseEvidence(candidate.facts)
  const possibleCauses = parseEvidence(candidate.possibleCauses)
  const unknowns = parseTextItems(candidate.unknowns)
  const recommendations = parseTextItems(candidate.recommendations)
  if (!summary || !facts || !possibleCauses || !unknowns || !recommendations) return null
  return { summary, facts, possibleCauses, unknowns, recommendations }
}

export function parseRunFailureDiagnosis(run: SimulationRun): FailureDiagnosis {
  if (!run.error || typeof run.error !== 'object' || Array.isArray(run.error)) {
    return {
      message: '运行失败，未提供可用原因',
      exitCode: publicByteCount(run.exitCode),
      durationMs: runDurationMs(run),
      engineLogs: [],
      analysisStatus: 'unavailable',
      analysis: null,
    }
  }
  const error = run.error as Record<string, unknown>
  const code = boundedText(error.code, 128)
  const message = boundedText(error.message)
  const modern = Array.isArray(error.engineLogs)
    || error.analysisStatus === 'pending'
    || error.analysisStatus === 'complete'
    || error.analysisStatus === 'unavailable'
  const engineLogs = modern
    ? parseEngineLogs(error.engineLogs)
    : (() => {
        const engineOutput = boundedLogText(error.engineOutput, 16_384)
        return engineOutput
          ? [{ stream: 'stderr' as const, text: engineOutput, totalBytes: null, sentBytes: null, truncated: false }]
          : []
      })()
  const analysis = parseFailureAnalysis(error.analysis)
  const requestedStatus: FailureAnalysisStatus = error.analysisStatus === 'pending'
    ? 'pending'
    : error.analysisStatus === 'complete' && analysis
      ? 'complete'
      : 'unavailable'
  const stage = boundedText(error.stage, 128) ?? boundedText(run.stage, 128)
  return {
    ...(code ? { code } : {}),
    message: message ?? '运行失败，未提供可用原因',
    ...(stage ? { stage } : {}),
    exitCode: publicByteCount(run.exitCode),
    durationMs: runDurationMs(run),
    engineLogs,
    analysisStatus: requestedStatus,
    analysis: requestedStatus === 'complete' ? analysis : null,
  }
}
