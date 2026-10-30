import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'

export interface CaseExampleSummary {
  exampleId: string
  importable: boolean
  targetFunction: string
}

export interface CaseSelectionFilters {
  exampleId?: string
  facade?: string
}

export interface ManualCaseFailure {
  code: string
  summary: string
}

export interface ManualCaseResult {
  exampleId: string
  bundlePath: string
  projectionProfile: string
  facade: string
  status: 'passed' | 'failed'
  phase: string
  durationMs: number
  phaseDurationsMs?: Record<string, number>
  promptSha256: string
  sessionId?: string
  runId?: string
  datasetIds?: string[]
  error?: ManualCaseFailure
}

export interface ManualCaseReport {
  schemaVersion: 1
  runId: string
  engineResourceSnapshot: string
  agentBundleVersion: string
  agentBundleSha256: string
  startedAt: string
  updatedAt: string
  discoveredExampleIds: string[]
  selectedExampleIds: string[]
  selectedFacades: Record<string, string>
  selectedBundlePaths: Record<string, string>
  selectedProjectionProfiles: Record<string, string>
  promptHashes: Record<string, string>
  cases: ManualCaseResult[]
  summary?: ReportSummary
  facadeSummary?: Record<string, FacadeSummary>
}

export interface ReportSummary {
  discovered: number
  selected: number
  passed: number
  failed: number
  pending: number
}

export interface FacadeSummary {
  selected: number
  passed: number
  failed: number
  pending: number
}

export function selectExampleIds(
  examples: CaseExampleSummary[],
  filters: CaseSelectionFilters,
  resumeReport?: ManualCaseReport,
): string[] {
  if (filters.exampleId && !/^ex_[a-f0-9]+$/.test(filters.exampleId)) {
    throw new Error('manual_case_example_id_invalid')
  }
  let selected = examples.filter(example => example.importable)
  if (filters.exampleId) selected = selected.filter(example => example.exampleId === filters.exampleId)
  if (filters.facade) selected = selected.filter(example => example.targetFunction === filters.facade)
  if (resumeReport) {
    const resumable = new Set(resumeReport.selectedExampleIds)
    const passed = new Set(resumeReport.cases.filter(result => result.status === 'passed').map(result => result.exampleId))
    selected = selected.filter(example => resumable.has(example.exampleId) && !passed.has(example.exampleId))
  }
  return selected.map(example => example.exampleId)
}

export async function initializeReport(reportPath: string, report: ManualCaseReport): Promise<void> {
  await mkdir(dirname(reportPath), { recursive: true })
  await writeAtomic(reportPath, withSummary(report))
}

export async function checkpointCase(reportPath: string, result: ManualCaseResult): Promise<ManualCaseReport> {
  const report = JSON.parse(await readFile(reportPath, 'utf8')) as ManualCaseReport
  const cases = report.cases.filter(current => current.exampleId !== result.exampleId)
  cases.push(result)
  const updated = withSummary({
    ...report,
    updatedAt: new Date().toISOString(),
    cases,
  })
  await writeAtomic(reportPath, updated)
  return updated
}

export function sanitizeCaseFailure(error: unknown, redactions: string[]): ManualCaseFailure {
  const record = error && typeof error === 'object' ? error as Record<string, unknown> : {}
  const rawCode = typeof record.code === 'string' ? record.code : ''
  const code = /^[a-z][a-z0-9_]*$/.test(rawCode) ? rawCode : 'manual_case_journey_failed'
  let summary = typeof record.message === 'string' ? record.message : '案例长测失败'
  for (const value of redactions.filter(Boolean)) summary = summary.split(value).join('<redacted>')
  summary = summary
    .replace(/Bearer\s+\S+/gi, 'Bearer <redacted>')
    .replace(/Cookie:\s*\S+/gi, 'Cookie: <redacted>')
    .replace(/\/(?:Users|home|private|tmp|var)\/[^\s,;]+/g, '<path>')
    .replace(/[\r\n\t\0]+/g, ' ')
    .trim()
  if (summary.length > 240) summary = `${summary.slice(0, 239)}…`
  return { code, summary }
}

function withSummary(report: ManualCaseReport): ManualCaseReport {
  const passed = report.cases.filter(result => result.status === 'passed').length
  const failed = report.cases.filter(result => result.status === 'failed').length
  const facadeSummary: Record<string, FacadeSummary> = {}
  for (const exampleId of report.selectedExampleIds) {
    const facade = report.selectedFacades?.[exampleId] ?? 'unknown'
    const summary = facadeSummary[facade] ?? { selected: 0, passed: 0, failed: 0, pending: 0 }
    summary.selected += 1
    const result = report.cases.find(candidate => candidate.exampleId === exampleId)
    if (result?.status === 'passed') summary.passed += 1
    else if (result?.status === 'failed') summary.failed += 1
    else summary.pending += 1
    facadeSummary[facade] = summary
  }
  return {
    ...report,
    summary: {
      discovered: report.discoveredExampleIds.length,
      selected: report.selectedExampleIds.length,
      passed,
      failed,
      pending: Math.max(0, report.selectedExampleIds.length - passed - failed),
    },
    facadeSummary: Object.fromEntries(Object.entries(facadeSummary).sort(([left], [right]) => left.localeCompare(right))),
  }
}

async function writeAtomic(reportPath: string, report: ManualCaseReport): Promise<void> {
  const temporaryPath = `${reportPath}.${process.pid}.${Date.now()}.tmp`
  await writeFile(temporaryPath, `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 })
  await rename(temporaryPath, reportPath)
}
