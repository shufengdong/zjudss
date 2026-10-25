import { test, expect, type APIRequestContext, type Browser, type Page } from '@playwright/test'
import { createHash } from 'node:crypto'
import { dirname, join } from 'node:path'
import { mkdir, readdir, readFile, writeFile } from 'node:fs/promises'
import {
  agentTurnCompleted,
  expectDomainState,
  expectEngineReleaseEvidence,
  expectToolTrace,
  type ToolTraceEntry,
} from './semantic-assertions'

test.setTimeout(15 * 60_000)

class TransientProviderError extends Error {
  constructor(readonly trace: ToolTraceEntry[]) { super('provider_transient') }
}

interface SegmentReport {
  label: string
  tools: { toolName: string; status: string; stableIds: Record<string, unknown> }[]
}
interface AttemptReport {
  attempt: number
  outcome: 'passed' | 'provider_transient' | 'failed'
  segments: SegmentReport[]
  reason?: string
}

const delay = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))

async function agentBundleEvidence() {
  const root = join(process.cwd(), '../backend/resources/agent/v3')
  const files: string[] = []
  const visit = async (directory: string, prefix = '') => {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const relative = prefix ? `${prefix}/${entry.name}` : entry.name
      if (entry.isDirectory()) await visit(join(directory, entry.name), relative)
      else if (entry.isFile()) files.push(relative)
    }
  }
  await visit(root)
  files.sort()
  const hash = createHash('sha256')
  for (const relative of files) {
    const bytes = await readFile(join(root, relative))
    hash.update(relative).update('\0').update(bytes).update('\0')
  }
  const manifest = JSON.parse(await readFile(join(root, 'manifest.json'), 'utf8'))
  return { version: manifest.agentBundleVersion as string, sha256: hash.digest('hex') }
}

function parseCsvRecords(text: string): string[][] {
  const rows: string[][] = []
  let row: string[] = []
  let field = ''
  let quoted = false
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index]
    if (quoted) {
      if (char === '"' && text[index + 1] === '"') { field += '"'; index += 1 }
      else if (char === '"') quoted = false
      else field += char
    } else if (char === '"') quoted = true
    else if (char === ',') { row.push(field); field = '' }
    else if (char === '\n') { row.push(field.replace(/\r$/, '')); rows.push(row); row = []; field = '' }
    else field += char
  }
  if (field || row.length) { row.push(field.replace(/\r$/, '')); rows.push(row) }
  return rows.filter(values => values.some(value => value.length > 0))
}

async function fixedUserSeed() {
  const bytes = await readFile(join(process.cwd(), '../backend/resources/init_users.csv'))
  const records = parseCsvRecords(bytes.toString('utf8'))
  expect(records[0]).toEqual(['ID', 'NAME'])
  expect(records.length).toBeGreaterThan(1)
  return {
    id: records[1][0],
    userCount: records.length - 1,
    sha256: createHash('sha256').update(bytes).digest('hex'),
  }
}

async function apiJson(api: APIRequestContext, path: string) {
  const response = await api.get(path)
  expect(response.ok(), `${path} returned ${response.status()}`).toBe(true)
  return response.json()
}

async function waitFor<T>(read: () => Promise<T | undefined>, timeoutMs = 120_000): Promise<T> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const value = await read()
    if (value !== undefined) return value
    await delay(500)
  }
  throw new Error('release semantic state timeout')
}

function traceFrom(messages: any[]): ToolTraceEntry[] {
  return messages.filter(message => message.type === 'tool-call').map(message => ({
    callId: message.callId,
    toolName: message.toolName,
    args: message.args ?? {},
    status: message.status,
  }))
}

function summarizeTrace(label: string, trace: ToolTraceEntry[]): SegmentReport {
  return {
    label,
    tools: trace.map(call => ({
      toolName: call.toolName,
      status: call.status,
      stableIds: Object.fromEntries(Object.entries(call.args).filter(([key]) => /Id$/.test(key) || key === 'engineResourceSnapshot')),
    })),
  }
}

async function sendAndCollect(page: Page, api: APIRequestContext, sessionId: string, text: string, requiredTools: string[]) {
  const before = (await apiJson(api, `/api/sessions/${sessionId}/messages?limit=100`)).messages as any[]
  const lastId = Math.max(0, ...before.map(message => Number(message.id)))
  let terminalCandidate: { key: string; since: number } | null = null
  await page.getByRole('textbox', { name: '消息' }).fill(text)
  await page.getByRole('button', { name: '发送消息' }).click()
  const messages = await waitFor(async () => {
    const all = (await apiJson(api, `/api/sessions/${sessionId}/messages?limit=100`)).messages as any[]
    const current = all.filter(message => Number(message.id) > lastId)
    if (agentTurnCompleted(current, requiredTools)) return current
    if (!agentTurnCompleted(current)) { terminalCandidate = null; return undefined }
    const key = current.map(message => `${message.id}:${message.status ?? ''}`).join('|')
    if (terminalCandidate?.key !== key) { terminalCandidate = { key, since: Date.now() }; return undefined }
    return Date.now() - terminalCandidate.since >= 5_000 ? current : undefined
  })
  const trace = traceFrom(messages)
  const assistant = messages.filter((message: any) => message.type === 'assistant').at(-1)
  if (assistant?.content === 'agent 运行失败，请稍后重试' && !trace.some(call => call.status === 'failed')) {
    throw new TransientProviderError(trace)
  }
  return trace
}

async function importAndPublish(page: Page, api: APIRequestContext, sessionId: string) {
  const directory = process.env.RELEASE_DATAFRAME_DIR
  if (!directory) throw new Error('release_dataframe_fixture_missing')
  const required = new Set([
    'bus.parquet', 'line.parquet', 'linecode.parquet', 'load.parquet', 'transformer.parquet',
    'voltage_source.parquet', 'motor.parquet', 'transformer_winding.parquet',
    'transformer_impedance.parquet', 'current_source.parquet', 'generator.parquet', 'ibr.parquet',
    'switch.parquet', 'shunt.parquet', 'capacitor.parquet', 'reactor.parquet', 'spectrum.parquet',
    'control_profile.parquet', 'xy_curve.parquet', 'storage.parquet', 'para.parquet',
  ])
  const files = (await readdir(directory)).filter(name => required.has(name)).sort().map(name => join(directory, name))
  expect(files.length).toBe(required.size)
  await page.locator('input[type=file]').first().setInputFiles(files)
  await expect(page.getByRole('button', { name: '确认合并' })).toBeVisible({ timeout: 120_000 })
  await page.getByRole('button', { name: '确认合并' }).click()
  await waitFor(async () => {
    const workspace = await apiJson(api, `/api/sessions/${sessionId}/workspace`)
    return workspace.draft?.tables?.length === required.size ? workspace : undefined
  })
  await page.getByRole('button', { name: '生成版本' }).click()
  return waitFor(async () => {
    const workspace = await apiJson(api, `/api/sessions/${sessionId}/workspace`)
    return workspace.active && !workspace.draft ? workspace : undefined
  })
}

async function clickRunAndWait(page: Page, api: APIRequestContext, sessionId: string, scriptName: string, expectedCount: number) {
  await page.getByRole('article', { name: `脚本 ${scriptName}` }).getByRole('button', { name: '运行' }).click()
  return waitFor(async () => {
    const runs = (await apiJson(api, `/api/sessions/${sessionId}/runs`)).runs
    return runs.length === expectedCount && runs.every((run: any) => run.status === 'succeeded') ? runs[0] : undefined
  })
}

async function runJourney(browser: Browser, report: AttemptReport) {
  const context = await browser.newContext()
  const page = await context.newPage()
  const api = context.request
  try {
    const seed = await fixedUserSeed()
    expect((await api.post('/api/auth/login', { data: { id: seed.id, password: seed.id } })).status()).toBe(200)
    await page.goto('/')
    const createdResponse = page.waitForResponse(response => response.url().endsWith('/api/sessions') && response.request().method() === 'POST')
    await page.getByRole('button', { name: '新建仿真' }).click()
    const sessionId = String((await (await createdResponse).json()).id)
    const workspace = await importAndPublish(page, api, sessionId)
    const versionId = String(workspace.active.id)

    let trace = await sendAndCollect(page, api, sessionId,
      '计算潮流',
      ['create_script'])
    expectToolTrace(trace, {
      requiredTools: ['inspect_workspace', 'load_agent_skill', 'plan_study', 'create_script'],
      allowedTools: ['inspect_workspace', 'load_agent_skill', 'plan_study', 'search_engine_docs', 'read_engine_doc_section', 'create_script'],
      gatedTools: { plan_study: ['study-planning'], create_script: ['rss-generation'] },
    })
    report.segments.push(summarizeTrace('study_power_flow', trace))
    const scripts = (await apiJson(api, `/api/sessions/${sessionId}/scripts`)).scripts as any[]
    expect(scripts).toHaveLength(1)
    const scriptName = String(scripts[0].name)
    await expect(page.getByRole('article', { name: `脚本 ${scriptName}` })).toBeVisible()
    expect((await apiJson(api, `/api/sessions/${sessionId}/runs`)).runs).toHaveLength(0)

    for (const study of [
      { label: 'study_fault_clarify', prompt: '短路计算' },
      { label: 'study_qsts_clarify', prompt: '做一天时序模拟' },
      { label: 'study_harmonic_clarify', prompt: '谐波分析' },
      { label: 'study_transient_clarify', prompt: '暂态仿真' },
      { label: 'study_motor_model_missing', prompt: '模拟电机启动暂态2秒，步长0.01秒，无外部事件' },
      { label: 'study_emt_unsupported', prompt: '做EMT暂态仿真' },
    ]) {
      trace = await sendAndCollect(page, api, sessionId, study.prompt, ['plan_study'])
      expectToolTrace(trace, {
        requiredTools: ['inspect_workspace', 'load_agent_skill', 'plan_study'],
        allowedTools: ['inspect_workspace', 'load_agent_skill', 'plan_study'],
        gatedTools: { plan_study: ['study-planning'] },
      })
      expect((await apiJson(api, `/api/sessions/${sessionId}/scripts`)).scripts).toHaveLength(1)
      expect((await apiJson(api, `/api/sessions/${sessionId}/runs`)).runs).toHaveLength(0)
      report.segments.push(summarizeTrace(study.label, trace))
    }

    const first = await clickRunAndWait(page, api, sessionId, scriptName, 1)
    await clickRunAndWait(page, api, sessionId, scriptName, 2)
    const runs = (await apiJson(api, `/api/sessions/${sessionId}/runs`)).runs
    const baseline = runs.find((run: any) => run.id === first.id) ?? runs[1]
    const candidate = runs.find((run: any) => run.id !== baseline.id)
    const baselineDatasets = (await apiJson(api, `/api/runs/${baseline.id}/datasets`)).datasets
    const voltage = baselineDatasets.find((dataset: any) => dataset.name === 'voltage') ?? baselineDatasets[0]
    expect(voltage?.id).toMatch(/^\d+$/)

    trace = await sendAndCollect(page, api, sessionId,
      `读取数据集 ID ${voltage.id} 的 schema，加载 result-analysis 技能，并运行一次结果分析，生成名为 derived 的 DataFrame。`,
      ['run_result_analysis'])
    expectToolTrace(trace, {
      requiredTools: ['list_result_datasets', 'load_agent_skill', 'run_result_analysis'],
      allowedTools: ['list_result_datasets', 'load_agent_skill', 'search_engine_docs', 'read_engine_doc_section', 'run_result_analysis'],
      gatedTools: { run_result_analysis: ['result-analysis'] },
    })
    report.segments.push(summarizeTrace('result_analysis', trace))

    trace = await sendAndCollect(page, api, sessionId,
      `加载 comparison 技能，比较基准运行 ID ${baseline.id} 与候选运行 ID ${candidate.id}，并保存比较结果。`,
      ['compare_simulations'])
    expectToolTrace(trace, {
      requiredTools: ['load_agent_skill', 'compare_simulations'],
      allowedTools: ['load_agent_skill', 'compare_simulations'],
      gatedTools: { compare_simulations: ['comparison'] },
    })
    report.segments.push(summarizeTrace('comparison', trace))

    const deleted = await api.delete(`/api/datasets/${voltage.id}`)
    expect(deleted.ok(), `dataset deletion returned ${deleted.status()}`).toBe(true)
    expect((await api.get(`/api/datasets/${voltage.id}`)).status()).toBe(404)
    expect((await api.get(`/api/datasets/${voltage.id}/rows`)).status()).toBe(404)
    expect((await api.get(`/api/datasets/${voltage.id}/download`)).status()).toBe(404)
    expect((await api.get(`/api/runs/${baseline.id}`)).status()).toBe(200)

    return expectDomainState(api, {
      sessionId,
      expectedVersions: 1,
      expectedRuns: 2,
      expectedDatasets: 1,
      expectedAnalysisRuns: 2,
      expectedComparisons: 1,
      activeVersionId: versionId,
    })
  } finally {
    await context.close()
  }
}

test('real DeepSeek and tensoreval complete the V3 semantic release journey', async ({ browser }) => {
  const reportPath = process.env.RELEASE_LLM_REPORT
  if (!reportPath) throw new Error('release_llm_report_missing')
  const attempts: AttemptReport[] = []
  let finalState: unknown = null
  let success = false
  try {
    for (let attempt = 1; attempt <= 3; attempt += 1) {
      const attemptReport: AttemptReport = { attempt, outcome: 'failed', segments: [] }
      attempts.push(attemptReport)
      try {
        finalState = await runJourney(browser, attemptReport)
        attemptReport.outcome = 'passed'
        success = true
        break
      } catch (error) {
        if (error instanceof TransientProviderError) {
          attemptReport.outcome = 'provider_transient'
          attemptReport.reason = error.message
          attemptReport.segments.push(summarizeTrace('provider_transient', error.trace))
          if (attempt < 3) continue
        } else attemptReport.reason = error instanceof Error ? error.message : 'semantic_failure'
        throw error
      }
    }
    expect(success, 'provider retries exhausted').toBe(true)
  } finally {
    await mkdir(dirname(reportPath), { recursive: true })
    const engineReportPath = process.env.RELEASE_ENGINE_REPORT
    const engineReport = engineReportPath ? JSON.parse(await readFile(engineReportPath, 'utf8')) : null
    expectEngineReleaseEvidence(engineReport)
    const agentBundle = await agentBundleEvidence()
    const seed = await fixedUserSeed()
    await writeFile(reportPath, JSON.stringify({
      schemaVersion: 1,
      testIds: ['ENGINE-RELEASE-001', 'REAL-RELEASE-001', 'AGENT-STUDY-PLANNING-001'],
      provider: 'deepseek',
      model: process.env.LLM_MODEL ?? 'deepseek-chat',
      agentBundleVersion: agentBundle.version,
      agentBundleSha256: agentBundle.sha256,
      userSeedSha256: seed.sha256,
      userCount: seed.userCount,
      engineContract: engineReport?.engineContract ?? null,
      engineContractChecks: engineReport?.contractChecks ?? null,
      engineBinarySha256: engineReport?.binarySha256 ?? null,
      engineResourceSnapshot: engineReport?.engineResourceSnapshot ?? null,
      engineExampleCounts: engineReport ? {
        discovered: engineReport.bundleExamples,
        importable: engineReport.importableExamples,
        nonImportable: engineReport.nonImportableExamples,
        bundles: engineReport.bundleExamples,
      } : null,
      attempts,
      finalState,
      success,
    }, null, 2))
  }
})
