import { test, expect, type APIRequestContext, type Browser, type Page } from '@playwright/test'
import { readdir, readFile, writeFile, mkdir } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import {
  agentTurnCompleted,
  expectDomainState,
  expectToolTrace,
  type ToolTraceEntry,
} from './semantic-assertions'

test.setTimeout(15 * 60_000)

class TransientProviderError extends Error {
  constructor(readonly trace: ToolTraceEntry[]) {
    super('provider_transient')
  }
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

const delay = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

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
  return messages
    .filter((message) => message.type === 'tool-call')
    .map((message) => ({
      callId: message.callId,
      toolName: message.toolName,
      args: message.args ?? {},
      status: message.status,
    }))
}

function summarizeTrace(label: string, trace: ToolTraceEntry[]): SegmentReport {
  return {
    label,
    tools: trace.map((call) => ({
      toolName: call.toolName,
      status: call.status,
      stableIds: Object.fromEntries(Object.entries(call.args).filter(([key]) => /Id$/.test(key) || key === 'engineResourceSnapshot')),
    })),
  }
}

async function sendAndCollect(
  page: Page,
  api: APIRequestContext,
  sessionId: string,
  text: string,
  requiredTools: string[],
) {
  const before = (await apiJson(api, `/api/sessions/${sessionId}/messages?limit=100`)).messages as any[]
  const lastId = Math.max(0, ...before.map((message) => Number(message.id)))
  await page.getByRole('textbox', { name: '消息' }).fill(text)
  await page.getByRole('button', { name: '发送消息' }).click()
  const messages = await waitFor(async () => {
    const all = (await apiJson(api, `/api/sessions/${sessionId}/messages?limit=100`)).messages as any[]
    const current = all.filter((message) => Number(message.id) > lastId)
    return agentTurnCompleted(current, requiredTools) ? current : undefined
  })
  const trace = traceFrom(messages)
  const assistant = messages.filter((message: any) => message.type === 'assistant').at(-1)
  if (assistant?.content === 'agent 运行失败，请稍后重试' && !trace.some((call) => call.status === 'failed')) {
    throw new TransientProviderError(trace)
  }
  return trace
}

async function uploadDataframes(page: Page, api: APIRequestContext, sessionId: string) {
  const directory = process.env.RELEASE_DATAFRAME_DIR
  if (!directory) throw new Error('release_dataframe_fixture_missing')
  const requiredFrames = new Set([
    'bus.parquet',
    'line.parquet',
    'linecode.parquet',
    'load.parquet',
    'transformer.parquet',
    'voltage_source.parquet',
  ])
  const files = (await readdir(directory))
    .filter((name) => requiredFrames.has(name))
    .sort()
    .map((name) => join(directory, name))
  expect(files.length).toBe(requiredFrames.size)
  await page.locator('input[type=file]').first().setInputFiles(files)
  return waitFor(async () => {
    const workspace = await apiJson(api, `/api/sessions/${sessionId}/workspace`)
    return workspace.draft?.status === 'collecting'
      && workspace.draft.inputs?.length === requiredFrames.size ? workspace : undefined
  })
}

async function runJourney(browser: Browser, report: AttemptReport) {
  const context = await browser.newContext()
  await context.addCookies([
    { name: 'zjudss-e2e-subject', value: `release-${report.attempt}@example.invalid`, url: 'http://127.0.0.1:15183' },
    { name: 'zjudss-e2e-tenant', value: 'release-candidate', url: 'http://127.0.0.1:15183' },
  ])
  const page = await context.newPage()
  const api = context.request
  try {
    const created = await api.post('/api/sessions', { data: { title: `候选发布 ${report.attempt}` } })
    expect(created.status()).toBe(201)
    const sessionId = String((await created.json()).id)
    await page.goto('/')
    const initialWorkspace = await uploadDataframes(page, api, sessionId)
    let dataframeVersionId = String(initialWorkspace.draft.versionId)

    for (const [fileName, tableKind] of [
      ['bus.parquet', 'bus'],
      ['line.parquet', 'line'],
      ['linecode.parquet', 'linecode'],
      ['transformer.parquet', 'transformer'],
      ['voltage_source.parquet', 'voltage_source'],
    ]) {
      const current = await apiJson(api, `/api/sessions/${sessionId}/workspace`)
      const input = current.draft.inputs.find((candidate: any) => candidate.fileName === fileName)
      expect(input?.inputId).toMatch(/^\d+$/)
      const trace = await sendAndCollect(page, api, sessionId,
        `我已逐项审阅并接受 ${fileName} 当前证据中的候选类型歧义、最高候选差异、置信度差异、全部类型冲突以及空值列风险。我的最终人工裁决是 tableKind=${tableKind}，无需再次确认或复述风险。请检查工作区后直接调用 confirm_dataframe_mapping：inputId=${input.inputId}，tableKind=${tableKind}，expectedDraftVersionId=${dataframeVersionId}。`,
        ['confirm_dataframe_mapping'])
      expectToolTrace(trace, {
        requiredTools: ['inspect_workspace', 'confirm_dataframe_mapping'],
      })
      report.segments.push(summarizeTrace(`confirm_${tableKind}`, trace))
      const workspace = await apiJson(api, `/api/sessions/${sessionId}/workspace`)
      dataframeVersionId = String((workspace.draft ?? workspace.active).versionId)
    }

    const mappedWorkspace = await waitFor(async () => {
      const workspace = await apiJson(api, `/api/sessions/${sessionId}/workspace`)
      return (workspace.draft ?? workspace.active)?.status === 'ready' ? workspace : undefined
    })
    let trace: ToolTraceEntry[]
    if (mappedWorkspace.draft) {
      trace = await sendAndCollect(page, api, sessionId,
        `请激活已确认完成的 DataFrame 候选版本 ID ${dataframeVersionId}，expectedDraftVersionId 也是 ${dataframeVersionId}。请直接调用激活工具。`,
        ['activate_model_version'])
      expectToolTrace(trace, { requiredTools: ['activate_model_version'] })
      report.segments.push(summarizeTrace('activate_dataframes', trace))
      await waitFor(async () => {
        const workspace = await apiJson(api, `/api/sessions/${sessionId}/workspace`)
        return String(workspace.active?.versionId) === dataframeVersionId ? workspace : undefined
      })
    } else {
      expect(String(mappedWorkspace.active?.versionId)).toBe(dataframeVersionId)
    }

    trace = await sendAndCollect(page, api, sessionId,
      `当前 active DataFrame 模型版本 ID 是 ${dataframeVersionId}。请检查工作区，加载 study-planning 和 rss-generation 技能，必要时查询内核文档，然后直接执行径向 power_flow。请使用兼容的六参数 ds_radial_pf(bus, line, linecode, load, transformer, voltage_source)，把默认 bus_voltage 结果命名为 voltage，不要只解释。`,
      ['run_study'])
    expectToolTrace(trace, {
      requiredTools: ['inspect_workspace', 'load_agent_skill', 'run_study'],
      gatedTools: { run_study: ['study-planning', 'rss-generation'] },
    })
    report.segments.push(summarizeTrace('dataframe_power_flow', trace))
    const baseline = await waitFor(async () => {
      const tasks = await apiJson(api, `/api/sessions/${sessionId}/tasks`)
      return tasks.find((task: any) => task.modelVersionId === dataframeVersionId && task.studyType === 'power_flow' && task.status === 'succeeded')
    })
    const baselineDatasets = await apiJson(api, `/api/tasks/${baseline.id}/datasets`)
    const voltage = baselineDatasets.find((dataset: any) => dataset.name === 'voltage')
    expect(voltage?.id).toMatch(/^\d+$/)

    trace = await sendAndCollect(page, api, sessionId,
      `请回答任务 ID ${baseline.id} 的电压结果问题：读取数据集 ID ${voltage.id} 的 schema，加载 result-analysis 技能，生成 derived DataFrame，仅保留 bus、v_mag、v_ang 三列。请直接运行分析，不要只给说明。`,
      ['run_result_analysis'])
    expectToolTrace(trace, {
      requiredTools: ['list_result_datasets', 'load_agent_skill', 'run_result_analysis'],
      gatedTools: { run_result_analysis: ['result-analysis'] },
    })
    report.segments.push(summarizeTrace('result_analysis', trace))

    const examples = await apiJson(api, '/api/examples?query=ieee13_short_circuit&sourceKind=rss&limit=5')
    const shortExample = examples.examples.find((example: any) => example.studyType === 'short_circuit' && example.sourceKind === 'rss')
    expect(shortExample?.id).toMatch(/^ex_/)
    trace = await sendAndCollect(page, api, sessionId,
      `请导入并激活 IEEE13 短路案例。先检查工作区，再用 search_examples 按 query=ieee13_short_circuit、sourceKind=rss 搜索，并用 inspect_example 检查搜索返回的稳定 exampleId。导入调用的 expectedDraftVersionId 是当前版本 ${dataframeVersionId}。导入成功后，激活调用的 versionId 和 expectedDraftVersionId 都必须填写 import_example 返回的新版本 ID，不要沿用旧基线 ${dataframeVersionId}。`,
      ['activate_model_version'])
    expectToolTrace(trace, {
      requiredTools: ['inspect_workspace', 'search_examples', 'inspect_example', 'import_example', 'activate_model_version'],
    })
    report.segments.push(summarizeTrace('import_short_circuit', trace))
    let workspace = await apiJson(api, `/api/sessions/${sessionId}/workspace`)
    const shortVersionId = String(workspace.active.versionId)
    expect(shortVersionId).not.toBe(dataframeVersionId)

    trace = await sendAndCollect(page, api, sessionId,
      `请对 active RSS 模型版本 ID ${shortVersionId} 直接执行 short_circuit。先加载 study-planning 和 rss-generation 技能；该模型本身就是可执行 RSS，使用版本入口脚本并只运行一次。`,
      ['run_study'])
    expectToolTrace(trace, {
      requiredTools: ['load_agent_skill', 'run_study'],
      gatedTools: { run_study: ['study-planning', 'rss-generation'] },
    })
    report.segments.push(summarizeTrace('short_circuit_baseline', trace))
    const shortBaseline = await waitFor(async () => {
      const tasks = await apiJson(api, `/api/sessions/${sessionId}/tasks`)
      return tasks.find((task: any) => task.modelVersionId === shortVersionId && task.studyType === 'short_circuit' && task.status === 'succeeded')
    })

    workspace = await apiJson(api, `/api/sessions/${sessionId}/workspace`)
    const dssInput = workspace.active.inputs.find((input: any) => input.fileName === 'IEEE13Nodeckt.dss')
    expect(dssInput?.inputId).toMatch(/^\d+$/)
    trace = await sendAndCollect(page, api, sessionId,
      `请修改当前模型 inputId ${dssInput.inputId} 中唯一对象 circuit.IEEE13Nodeckt 的 angle 参数：从 0 改为 1 deg。先加载 rss-generation 技能；dss_property 是 modify_model patch 的保留 tableKind，不是引擎函数，无需搜索文档。直接使用 patch={tableKind:dss_property,rowKey:{object:circuit.IEEE13Nodeckt},changes:{angle:{from:0,to:1,unit:deg}}}，expectedDraftVersionId=${shortVersionId}。修改成功后，激活调用的 versionId 和 expectedDraftVersionId 都填写 modify_model 返回的新版本 ID。`,
      ['activate_model_version'])
    expectToolTrace(trace, {
      requiredTools: ['load_agent_skill', 'modify_model', 'activate_model_version'],
      gatedTools: { modify_model: ['rss-generation'] },
    })
    report.segments.push(summarizeTrace('modify_and_activate', trace))
    workspace = await apiJson(api, `/api/sessions/${sessionId}/workspace`)
    const changedVersionId = String(workspace.active.versionId)
    expect(changedVersionId).not.toBe(shortVersionId)

    trace = await sendAndCollect(page, api, sessionId,
      `请对修改后的 active RSS 模型版本 ID ${changedVersionId} 重跑 short_circuit。加载 study-planning 和 rss-generation 技能并直接执行版本入口脚本。`,
      ['run_study'])
    expectToolTrace(trace, {
      requiredTools: ['load_agent_skill', 'run_study'],
      gatedTools: { run_study: ['study-planning', 'rss-generation'] },
    })
    report.segments.push(summarizeTrace('short_circuit_candidate', trace))
    const shortCandidate = await waitFor(async () => {
      const tasks = await apiJson(api, `/api/sessions/${sessionId}/tasks`)
      return tasks.find((task: any) => task.modelVersionId === changedVersionId && task.studyType === 'short_circuit' && task.status === 'succeeded')
    })

    trace = await sendAndCollect(page, api, sessionId,
      `请比较短路基准任务 ID ${shortBaseline.id} 和修改后任务 ID ${shortCandidate.id}。先加载 comparison 技能，再直接调用 compare_simulations。`,
      ['compare_simulations'])
    expectToolTrace(trace, {
      requiredTools: ['load_agent_skill', 'compare_simulations'],
      gatedTools: { compare_simulations: ['comparison'] },
    })
    report.segments.push(summarizeTrace('comparison', trace))

    return expectDomainState(api, {
      sessionId,
      minimumVersions: 3,
      taskStudyTypes: ['power_flow', 'short_circuit', 'short_circuit'],
      minimumAnalysisRuns: 1,
      minimumComparisons: 1,
      activeVersionId: changedVersionId,
    })
  } finally {
    await context.close()
  }
}

test('real DeepSeek and tensoreval complete the semantic release journey', async ({ browser }) => {
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
        } else {
          attemptReport.reason = error instanceof Error ? error.name : 'semantic_failure'
        }
        throw error
      }
    }
    expect(success, 'provider retries exhausted').toBe(true)
  } finally {
    await mkdir(dirname(reportPath), { recursive: true })
    const engineReportPath = process.env.RELEASE_ENGINE_REPORT
    const engineReport = engineReportPath ? JSON.parse(await readFile(engineReportPath, 'utf8')) : null
    await writeFile(reportPath, JSON.stringify({
      schemaVersion: 1,
      provider: 'deepseek',
      model: process.env.LLM_MODEL ?? 'deepseek-chat',
      engineContract: engineReport?.engineContract ?? null,
      engineBinarySha256: engineReport?.binarySha256 ?? null,
      attempts,
      finalState,
      success,
    }, null, 2))
  }
})
