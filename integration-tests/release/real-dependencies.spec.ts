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
import { resolveLlmEvidence } from './llm-evidence'

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
  const root = join(process.cwd(), '../backend/resources/agent/v4')
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
  const confirm = page.getByRole('button', { name: '确认用途并发布' }).last()
  await expect(confirm).toBeVisible({ timeout: 120_000 })
  const card = confirm.locator('xpath=ancestor::section[1]')
  const purposeSelects = card.getByRole('combobox', { name: /用途$/ })
  expect(await purposeSelects.count()).toBe(21)
  for (let index = 0; index < await purposeSelects.count(); index += 1) {
    const select = purposeSelects.nth(index)
    const displayName = String(await select.getAttribute('aria-label')).replace(/用途$/, '')
    const alias = displayName.replace(/\.parquet$/, '')
    await select.selectOption(alias === 'para' ? 'data_resource' : 'model_table')
    await card.getByRole('textbox', { name: `${displayName} alias` }).fill(alias)
  }
  const publishResponse = page.waitForResponse(response => response.request().method() === 'POST'
    && /\/api\/upload-batches\/[^/]+\/confirm$/.test(new URL(response.url()).pathname))
  await confirm.click()
  const published = await publishResponse
  expect(published.status()).toBe(200)
  const confirmation = await published.json() as { uploadConfirmation?: { resources?: { alias?: string }[] } }
  expect(confirmation.uploadConfirmation?.resources?.some(resource => resource.alias === 'para')).toBe(true)
  await waitFor(async () => {
    const workspace = await apiJson(api, `/api/sessions/${sessionId}/workspace`)
    return workspace.draft?.tables?.length === 20 ? workspace : undefined
  })
  if (await page.getByRole('button', { name: '展开结果面板' }).isVisible()) {
    await page.getByRole('button', { name: '展开结果面板' }).click()
  }
  await page.getByRole('button', { name: /^展开可变草稿/ }).click()
  await page.getByRole('button', { name: '生成版本' }).click()
  return waitFor(async () => {
    const workspace = await apiJson(api, `/api/sessions/${sessionId}/workspace`)
    return workspace.active && !workspace.draft ? workspace : undefined
  })
}

async function clickRunAndWait(page: Page, api: APIRequestContext, sessionId: string, scriptName: string, expectedCount: number) {
  const run = page.getByRole('article', { name: `脚本 ${scriptName}` }).getByRole('button', { name: '运行' })
  await expect(run).toBeEnabled({ timeout: 30_000 })
  await run.click({ timeout: 30_000 })
  return waitFor(async () => {
    const runs = (await apiJson(api, `/api/sessions/${sessionId}/runs`)).runs
    return runs.length === expectedCount && runs.every((run: any) => run.status === 'succeeded') ? runs[0] : undefined
  })
}

async function runJourney(browser: Browser, baseURL: string, report: AttemptReport) {
  const context = await browser.newContext({ baseURL })
  const page = await context.newPage()
  const api = context.request
  try {
    const seed = await fixedUserSeed()
    expect((await api.post('/api/auth/login', { data: { id: seed.id, password: seed.id } })).status()).toBe(200)
    await page.goto('/')
    const existing = await waitFor(async () => {
      const sessions = (await apiJson(api, '/api/sessions')).sessions as { id: string }[]
      return sessions.length > 0 ? sessions : undefined
    })
    const existingIds = new Set(existing.map(session => String(session.id)))
    await page.getByRole('button', { name: '新建仿真' }).click()
    const created = await waitFor(async () => {
      const sessions = (await apiJson(api, '/api/sessions')).sessions as { id: string }[]
      return sessions.find(session => !existingIds.has(String(session.id)))
    })
    const sessionId = String(created.id)
    const workspace = await importAndPublish(page, api, sessionId)
    const versionId = String(workspace.active.id)

    let trace = await sendAndCollect(page, api, sessionId, '求解牛顿法潮流', ['prepare_study'])
    expectToolTrace(trace, {
      requiredTools: ['inspect_workspace', 'load_agent_skill', 'search_study_facades', 'prepare_study'],
      allowedTools: ['inspect_workspace', 'load_agent_skill', 'search_study_facades', 'prepare_study'],
      gatedTools: { prepare_study: ['study-planning'] },
    })
    report.segments.push(summarizeTrace('managed_study_power_flow', trace))
    const configuration = page.getByRole('article', { name: '研究配置卡' }).last()
    await expect(configuration).toBeVisible({ timeout: 120_000 })
    const loadState = configuration.getByRole('checkbox', { name: /load_state/ })
    await expect(loadState).toBeVisible()
    await loadState.check()
    const selectResultsResponse = page.waitForResponse(response => response.request().method() === 'PUT'
      && /\/api\/study-configurations\/\d+\/results$/.test(new URL(response.url()).pathname))
    await configuration.getByRole('button', { name: '确认结果' }).click()
    expect((await selectResultsResponse).ok()).toBe(true)
    const confirmParameters = configuration.getByRole('button', { name: '确认参数' })
    if (await confirmParameters.isVisible()) {
      const frequency = configuration.getByLabel(/base_frequency_hz/)
      if (await frequency.isVisible() && await frequency.inputValue() === '') await frequency.fill('50')
      const parameterResponse = page.waitForResponse(response => response.request().method() === 'PUT'
        && /\/api\/study-configurations\/\d+\/structs\//.test(new URL(response.url()).pathname))
      await confirmParameters.click()
      const saved = await (await parameterResponse).json() as { status: string }
      expect(['collecting_inputs', 'ready_to_confirm']).toContain(saved.status)
    }
    const confirmStudy = configuration.getByRole('button', { name: '确认并生成脚本' })
    const confirmResults = configuration.getByRole('button', { name: '确认结果' })
    try {
      await expect.poll(async () => await confirmStudy.isVisible() || await confirmResults.isEnabled(), { timeout: 10_000 }).toBe(true)
    } catch {
      await page.reload()
      await expect.poll(async () => await confirmStudy.isVisible() || await confirmResults.isEnabled(), { timeout: 30_000 }).toBe(true)
    }
    if (!await confirmStudy.isVisible()) await confirmResults.click()
    await expect(confirmStudy).toBeVisible()
    const confirmResponse = page.waitForResponse(response => response.request().method() === 'POST'
      && /\/api\/study-configurations\/\d+\/confirm$/.test(new URL(response.url()).pathname))
    await confirmStudy.click()
    const workflowStart = await (await confirmResponse).json() as { id: number }
    const managedWorkflow = await waitFor(async () => {
      const workflow = await apiJson(api, `/api/workflows/${workflowStart.id}`)
      if (workflow.status === 'failed') throw new Error(`managed_workflow_failed:${JSON.stringify(workflow.error)}`)
      return workflow.status === 'succeeded' ? workflow : undefined
    }, 5 * 60_000)
    const scripts = (await apiJson(api, `/api/sessions/${sessionId}/scripts`)).scripts as any[]
    expect(scripts).toHaveLength(1)
    const scriptName = String(scripts[0].name)
    await expect(page.getByRole('article', { name: `脚本 ${scriptName}` })).toBeVisible()
    expect((await apiJson(api, `/api/sessions/${sessionId}/runs`)).runs).toHaveLength(1)

    await clickRunAndWait(page, api, sessionId, scriptName, 2)
    const runs = (await apiJson(api, `/api/sessions/${sessionId}/runs`)).runs
    const baseline = runs.find((run: any) => String(run.id) === String(managedWorkflow.runId))
    expect(baseline, 'managed workflow baseline run is missing').toBeTruthy()
    const candidate = runs.find((run: any) => String(run.id) !== String(baseline.id))
    expect(candidate, 'explicit rerun is missing').toBeTruthy()
    const baselineDatasets = (await apiJson(api, `/api/runs/${baseline.id}/datasets`)).datasets
    const voltage = baselineDatasets.find((dataset: any) => dataset.name === 'bus_voltage') ?? baselineDatasets[0]
    expect(voltage?.id).toMatch(/^\d+$/)

    trace = await sendAndCollect(page, api, sessionId,
      `读取数据集 ID ${voltage.id} 的 schema并加载 result-analysis 技能。随后运行结果分析：把该数据集绑定为 source，不筛选、不聚合、不改变任何列，使用合法 RSS 脚本 \`derived = select(col(bus), col(v_mag), col(v_ang), source);\` 将完整数据输出为名为 derived 的 DataFrame。`,
      ['run_result_analysis'])
    expectToolTrace(trace, {
      requiredTools: ['list_result_datasets', 'load_agent_skill', 'run_result_analysis'],
      gatedTools: { run_result_analysis: ['result-analysis'] },
    })
    report.segments.push(summarizeTrace('result_analysis', trace))

    trace = await sendAndCollect(page, api, sessionId,
      `加载 comparison 技能，比较基准运行 ID ${baseline.id} 与候选运行 ID ${candidate.id}，并保存比较结果。`,
      ['compare_simulations'])
    expectToolTrace(trace, {
      requiredTools: ['load_agent_skill', 'compare_simulations'],
      gatedTools: { compare_simulations: ['comparison'] },
    })
    report.segments.push(summarizeTrace('comparison', trace))

    const deleted = await api.delete(`/api/datasets/${voltage.id}`)
    expect(deleted.ok(), `dataset deletion returned ${deleted.status()}`).toBe(true)
    expect((await api.get(`/api/datasets/${voltage.id}`)).status()).toBe(404)
    expect((await api.get(`/api/runs/${baseline.id}`)).status()).toBe(200)

    const finalState = await expectDomainState(api, {
      sessionId,
      expectedVersions: 1,
      expectedRuns: 2,
      expectedDatasets: 3,
      expectedAnalysisRuns: 2,
      expectedComparisons: 1,
      activeVersionId: versionId,
    })
    return {
      finalState,
      managedWorkflow: {
        status: managedWorkflow.status,
        correctionsUsed: managedWorkflow.correctionsUsed,
        attemptCount: managedWorkflow.attempts.length,
      },
    }
  } finally {
    await context.close()
  }
}

test('real DeepSeek and tensoreval complete the V4 semantic release journey', async ({ browser, baseURL }) => {
  if (!baseURL) throw new Error('release_base_url_missing')
  const reportPath = process.env.RELEASE_LLM_REPORT
  if (!reportPath) throw new Error('release_llm_report_missing')
  const attempts: AttemptReport[] = []
  let finalState: unknown = null
  let managedWorkflow: unknown = null
  let success = false
  try {
    for (let attempt = 1; attempt <= 3; attempt += 1) {
      const attemptReport: AttemptReport = { attempt, outcome: 'failed', segments: [] }
      attempts.push(attemptReport)
      try {
        const result = await runJourney(browser, baseURL, attemptReport)
        finalState = result.finalState
        managedWorkflow = result.managedWorkflow
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
    const llm = resolveLlmEvidence(process.env)
    await writeFile(reportPath, JSON.stringify({
      schemaVersion: 1,
      testIds: ['ENGINE-RELEASE-001', 'REAL-RELEASE-001', 'AGENT-STUDY-PLANNING-001'],
      provider: llm.provider,
      model: llm.model,
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
      engineFacadeCatalog: engineReport ? {
        entries: engineReport.facadeCatalogEntries,
        enabled: engineReport.enabledFacades,
        diagnostics: engineReport.facadeDiagnostics?.length ?? -1,
      } : null,
      engineProjectionCatalog: engineReport ? {
        entries: engineReport.projectionCatalogEntries,
        enabled: engineReport.enabledProjectionProfiles,
        referenced: engineReport.referencedProjectionProfiles,
        diagnostics: engineReport.projectionDiagnostics?.length ?? -1,
        representativeRuns: engineReport.representativeRuns?.length ?? -1,
      } : null,
      attempts,
      finalState,
      managedWorkflow,
      success,
    }, null, 2))
  }
})
