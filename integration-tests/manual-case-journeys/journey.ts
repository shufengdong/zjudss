import { expect, type APIRequestContext, type Page } from '@playwright/test'
import {
  agentTurnCompleted,
  expectToolTrace,
  type ToolTraceEntry,
} from '../release/semantic-assertions'
import { promptForFacade, promptSha256 } from './prompts'

export const JOURNEY_PHASES = [
  'discover',
  'apply_example',
  'confirm_candidates',
  'publish_by_agent',
  'prepare_study_by_agent',
  'complete_configuration',
  'confirm_workflow',
  'wait_workflow',
  'verify_result',
  'cleanup_session',
] as const

export type JourneyPhase = typeof JOURNEY_PHASES[number]

interface ExampleSummary {
  id: string
  title: string
  bundlePath: string
  importable: boolean
}

interface ExampleTreeNode {
  type: 'directory' | 'example'
  children?: ExampleTreeNode[]
  example?: ExampleSummary
}

interface ExampleDetail extends ExampleSummary {
  manifest: { projection: { profile: string; targetFunction: string } } | null
}

export interface DiscoveredExample {
  exampleId: string
  title: string
  bundlePath: string
  importable: boolean
  projectionProfile: string
  targetFunction: string
}

interface StudyInput {
  inputId: string
  alias: string
  kind: 'model' | 'struct' | 'data'
  required: boolean
  binding: { modelTableName?: string | null; resourceId?: string | null }
  resourceOptions: Array<{ resourceId: string }>
  reuseOffer?: { resourceId: string } | null
  structCard?: unknown | null
}

interface StudyConfiguration {
  id: number
  revision: number
  status: string
  inputs: StudyInput[]
  results: {
    available: Array<{ id: string; name: string; recommended: boolean }>
    selected: string[]
  }
}

interface WorkflowView {
  id: number
  runId: number
  status: string
  finalScriptResourceId?: number | null
  outputDatasetIds?: string[]
  error?: { code?: string } | null
}

interface AgentTurn {
  trace: ToolTraceEntry[]
  messages: any[]
}

export interface JourneyEvidence {
  sessionId: string
  facade: string
  runId: string
  datasetIds: string[]
  promptSha256: string
  phaseDurationsMs: Record<string, number>
}

export class JourneyPhaseError extends Error {
  sessionId?: string
  phaseDurationsMs?: Record<string, number>

  constructor(
    readonly phase: JourneyPhase,
    readonly code: string,
    options?: { cause?: unknown },
  ) {
    super(`案例长测阶段失败：${phase}`)
    this.name = 'JourneyPhaseError'
    if (options?.cause !== undefined) Object.defineProperty(this, 'cause', { value: options.cause })
  }
}

export async function discoverExamples(api: APIRequestContext): Promise<{
  engineResourceSnapshot: string
  examples: DiscoveredExample[]
}> {
  const listed = await apiJson(api, '/api/examples') as {
    engineResourceSnapshot: string
    tree: ExampleTreeNode[]
  }
  const summaries = flattenExamples(listed.tree).filter(example => example.importable)
  const examples: DiscoveredExample[] = []
  for (const summary of summaries) {
    const detail = await apiJson(api, `/api/examples/${summary.id}`) as ExampleDetail
    const targetFunction = detail.manifest?.projection.targetFunction
    const projectionProfile = detail.manifest?.projection.profile
    if (!targetFunction || !projectionProfile) {
      throw new JourneyPhaseError('discover', 'case_manifest_projection_missing')
    }
    examples.push({
      exampleId: summary.id,
      title: summary.title,
      bundlePath: summary.bundlePath,
      importable: summary.importable,
      projectionProfile,
      targetFunction,
    })
  }
  return { engineResourceSnapshot: listed.engineResourceSnapshot, examples }
}

export async function runCaseJourney(
  page: Page,
  api: APIRequestContext,
  engineResourceSnapshot: string,
  example: DiscoveredExample,
): Promise<JourneyEvidence> {
  const phaseDurationsMs: Record<string, number> = {}
  let sessionId = ''
  const phase = async <T>(name: JourneyPhase, action: () => Promise<T>): Promise<T> => {
    const started = Date.now()
    try {
      return await action()
    } catch (cause) {
      const failure = cause instanceof JourneyPhaseError
        ? cause
        : new JourneyPhaseError(name, errorCode(cause, `manual_case_${name}_failed`), { cause })
      phaseDurationsMs[name] = Date.now() - started
      failure.sessionId ??= sessionId || undefined
      failure.phaseDurationsMs = { ...phaseDurationsMs }
      throw failure
    } finally {
      phaseDurationsMs[name] ??= Date.now() - started
    }
  }

  sessionId = await phase('apply_example', async () => {
    const created = await createFreshSession(page, api)
    const dialog = await openExample(page, example)
    const applyResponse = page.waitForResponse(response => response.request().method() === 'POST'
      && new URL(response.url()).pathname === `/api/sessions/${created}/draft/example`, { timeout: 5 * 60_000 })
    await dialog.getByRole('button', { name: `使用案例“${example.title}”` }).click()
    await checkedJson(await applyResponse)
    return created
  })

  await phase('confirm_candidates', async () => {
    const confirm = page.getByRole('button', { name: '确认用途并发布' }).last()
    await expect(confirm).toBeVisible({ timeout: 5 * 60_000 })
    const responsePromise = page.waitForResponse(response => response.request().method() === 'POST'
      && /\/api\/upload-batches\/[^/]+\/confirm$/.test(new URL(response.url()).pathname), { timeout: 5 * 60_000 })
    await confirm.click()
    await checkedJson(await responsePromise)
    await waitFor(async () => {
      const workspace = await apiJson(api, `/api/sessions/${sessionId}/workspace`) as any
      return workspace.draft?.tables?.length > 0 ? workspace : undefined
    }, 5 * 60_000)
  })

  await phase('publish_by_agent', async () => {
    const turn = await sendAndCollect(
      page,
      api,
      sessionId,
      `请发布当前模型草稿并自动激活，版本描述为案例 ${example.exampleId} 的全链路长测。`,
      ['publish_model_version'],
    )
    expectToolTrace(turn.trace, {
      requiredTools: ['load_agent_skill', 'inspect_workspace', 'publish_model_version'],
      allowedTools: ['load_agent_skill', 'inspect_workspace', 'publish_model_version'],
      gatedTools: { publish_model_version: ['model-intake'] },
    })
    await waitFor(async () => {
      const workspace = await apiJson(api, `/api/sessions/${sessionId}/workspace`) as any
      return workspace.active && !workspace.draft ? workspace : undefined
    }, 3 * 60_000)
  })

  let prompt = ''
  let configurationId = ''
  await phase('prepare_study_by_agent', async () => {
    prompt = promptForFacade(example.targetFunction)
    const turn = await sendAndCollect(page, api, sessionId, prompt, ['prepare_study'])
    expectToolTrace(turn.trace, {
      requiredTools: ['load_agent_skill', 'inspect_workspace', 'search_study_facades', 'prepare_study'],
      gatedTools: { prepare_study: ['study-planning'] },
    })
    const explicitSearch = turn.trace.some(call => call.toolName === 'search_study_facades'
      && call.status === 'success' && call.args.explicitFunction === example.targetFunction)
    if (!explicitSearch) throw coded('study_facade_mismatch')
    const card = [...turn.messages].reverse().find(message => message.type === 'study-config'
      && message.card?.configurationId)
    configurationId = String(card?.card?.configurationId ?? '')
    if (!configurationId) throw coded('study_configuration_missing')
    await expect(page.getByRole('article', { name: '研究配置卡' }).last()).toBeVisible()
  })

  await phase('complete_configuration', async () => {
    await completeConfiguration(page, api, configurationId, engineResourceSnapshot)
  })

  let workflowId = ''
  await phase('confirm_workflow', async () => {
    const card = page.getByRole('article', { name: '研究配置卡' }).last()
    const confirm = card.getByRole('button', { name: '确认并生成脚本' })
    await expect(confirm).toBeVisible({ timeout: 120_000 })
    const responsePromise = page.waitForResponse(response => response.request().method() === 'POST'
      && new URL(response.url()).pathname === `/api/study-configurations/${configurationId}/confirm`)
    await confirm.click()
    const workflow = await checkedJson(await responsePromise) as WorkflowView
    workflowId = String(workflow.id)
  })

  const workflow = await phase('wait_workflow', async () => waitFor(async () => {
    const current = await apiJson(api, `/api/workflows/${workflowId}`) as WorkflowView
    if (current.status === 'failed' || current.status === 'cancelled') {
      throw coded(current.error?.code ?? `managed_workflow_${current.status}`)
    }
    return current.status === 'succeeded' ? current : undefined
  }, 15 * 60_000))

  const datasets = await phase('verify_result', async () => {
    const scripts = (await apiJson(api, `/api/sessions/${sessionId}/scripts`) as any).scripts as any[]
    const finalScript = scripts.find(script => Number(script.id) === workflow.finalScriptResourceId)
    if (finalScript?.sourceKind !== 'agent' || finalScript.validationStatus !== 'valid') {
      throw coded('agent_script_missing')
    }
    const run = await apiJson(api, `/api/runs/${workflow.runId}`) as any
    if (run.status !== 'succeeded') throw coded('managed_run_not_succeeded')
    const values = (await apiJson(api, `/api/runs/${workflow.runId}/datasets`) as any).datasets as any[]
    if (!values.length) throw coded('managed_result_missing')
    const configuration = await apiJson(api, `/api/study-configurations/${configurationId}`) as StudyConfiguration
    const expectedNames = configuration.results.available
      .filter(result => configuration.results.selected.includes(result.id))
      .map(result => result.name)
      .sort()
    const actualNames = values.map(dataset => String(dataset.name)).sort()
    if (JSON.stringify(actualNames) !== JSON.stringify(expectedNames)) {
      throw coded('managed_result_contract_mismatch')
    }
    const outputDatasetIds = (workflow.outputDatasetIds ?? []).map(String).sort()
    const actualDatasetIds = values.map(dataset => String(dataset.id)).sort()
    if (JSON.stringify(actualDatasetIds) !== JSON.stringify(outputDatasetIds)) {
      throw coded('managed_result_lineage_mismatch')
    }
    const first = values[0]
    if (!(first.rowCount > 0) || !Array.isArray(first.columns) || first.columns.length === 0) {
      throw coded('managed_result_empty')
    }
    const datasetPage = await apiJson(api, `/api/datasets/${first.id}/rows?limit=1`) as any
    if (!Array.isArray(datasetPage.rows) || datasetPage.rows.length === 0) {
      throw coded('managed_result_rows_empty')
    }
    return values
  })

  return {
    sessionId,
    facade: example.targetFunction,
    runId: String(workflow.runId),
    datasetIds: datasets.map(dataset => String(dataset.id)),
    promptSha256: promptSha256(prompt),
    phaseDurationsMs,
  }
}

export async function cleanupSession(page: Page, api: APIRequestContext, sessionId: string): Promise<void> {
  try {
    const preview = await apiJson(api, `/api/sessions/${sessionId}/deletion-preview`, 'POST') as { impactToken: string }
    const response = await api.delete(`/api/sessions/${sessionId}?impactToken=${encodeURIComponent(preview.impactToken)}`)
    if (!response.ok()) await checkedJson(response)
    await page.reload()
    await expect(page.getByRole('button', { name: '新建仿真' })).toBeVisible()
  } catch (cause) {
    throw new JourneyPhaseError('cleanup_session', errorCode(cause, 'manual_case_cleanup_failed'), { cause })
  }
}

async function completeConfiguration(
  page: Page,
  api: APIRequestContext,
  configurationId: string,
  engineResourceSnapshot: string,
) {
  for (let iteration = 0; iteration < 100; iteration += 1) {
    const configuration = await apiJson(api, `/api/study-configurations/${configurationId}`) as StudyConfiguration
    if ((configuration as StudyConfiguration & { engineSnapshotId?: string }).engineSnapshotId !== engineResourceSnapshot) {
      throw coded('study_engine_snapshot_mismatch')
    }
    if (configuration.status === 'ready_to_confirm') return
    if (configuration.status !== 'collecting_inputs') throw coded(`study_configuration_${configuration.status}`)
    const card = page.getByRole('article', { name: '研究配置卡' }).last()
    let acted = false
    for (const input of configuration.inputs) {
      if (input.kind === 'model' || input.binding.resourceId) continue
      if (input.kind === 'struct' && input.structCard) {
        const heading = card.getByRole('heading', { name: `参数：${input.alias}` })
        const section = heading.locator('xpath=ancestor::section[1]')
        const confirm = section.getByRole('button', { name: '确认参数' })
        if (!await confirm.isEnabled()) {
          await section.getByRole('button', { name: '恢复默认' }).click()
        }
        if (!await confirm.isEnabled()) throw coded('study_struct_values_incomplete')
        const response = page.waitForResponse(candidate => candidate.request().method() === 'PUT'
          && new URL(candidate.url()).pathname.includes(`/api/study-configurations/${configurationId}/structs/`))
        await confirm.click()
        await checkedJson(await response)
        acted = true
        break
      }
      if (input.resourceOptions.length === 1 || input.reuseOffer) {
        const heading = card.getByRole('heading', { name: `选择 ${input.alias}` })
        const section = heading.locator('xpath=ancestor::section[1]')
        const response = page.waitForResponse(candidate => candidate.request().method() === 'PUT'
          && new URL(candidate.url()).pathname.includes(`/api/study-configurations/${configurationId}/bindings/`))
        await section.getByRole('button', { name: '确认文件' }).click()
        await checkedJson(await response)
        acted = true
        break
      }
      if (input.required && input.resourceOptions.length > 1) throw coded('study_resource_ambiguous')
      if (input.required) throw coded('study_input_not_found')
    }
    if (acted) continue
    if (configuration.results.selected.length === 0) {
      const recommended = configuration.results.available.filter(result => result.recommended)
      if (recommended.length === 0) throw coded('recommended_result_missing')
      for (const result of recommended) {
        const checkbox = card.getByRole('checkbox', { name: new RegExp(escapeRegex(result.name)) })
        if (!await checkbox.isChecked()) await checkbox.check()
      }
      const response = page.waitForResponse(candidate => candidate.request().method() === 'PUT'
        && new URL(candidate.url()).pathname === `/api/study-configurations/${configurationId}/results`)
      await card.getByRole('button', { name: '确认结果' }).click()
      await checkedJson(await response)
      continue
    }
    throw coded('study_configuration_stalled')
  }
  throw coded('study_configuration_iteration_limit')
}

async function openExample(page: Page, example: DiscoveredExample) {
  await page.getByRole('button', { name: '案例库' }).click()
  const dialog = page.getByRole('dialog', { name: '内置案例库' })
  await dialog.getByRole('textbox', { name: '搜索内置案例' }).fill(example.bundlePath)
  const leaf = dialog.getByRole('button', { name: example.title, exact: true })
  await expect(leaf).toHaveCount(1, { timeout: 120_000 })
  await leaf.click()
  await expect(dialog.getByLabel(`案例详情“${example.title}”`)).toContainText(example.bundlePath)
  await expect(dialog.getByRole('button', { name: `使用案例“${example.title}”` })).toBeVisible()
  return dialog
}

async function createFreshSession(page: Page, api: APIRequestContext): Promise<string> {
  const before = new Set(((await apiJson(api, '/api/sessions') as any).sessions as any[]).map(session => String(session.id)))
  await page.getByRole('button', { name: '新建仿真' }).click()
  return waitFor(async () => {
    const sessions = (await apiJson(api, '/api/sessions') as any).sessions as any[]
    return sessions.map(session => String(session.id)).find(id => !before.has(id))
  }, 30_000)
}

async function sendAndCollect(
  page: Page,
  api: APIRequestContext,
  sessionId: string,
  text: string,
  requiredTools: string[],
): Promise<AgentTurn> {
  const before = (await apiJson(api, `/api/sessions/${sessionId}/messages?limit=100`) as any).messages as any[]
  const lastId = Math.max(0, ...before.map(message => Number(message.id)))
  await page.getByRole('textbox', { name: '消息' }).fill(text)
  await page.getByRole('button', { name: '发送消息' }).click()
  const messages = await waitFor(async () => {
    const all = (await apiJson(api, `/api/sessions/${sessionId}/messages?limit=100`) as any).messages as any[]
    const current = all.filter(message => Number(message.id) > lastId)
    return agentTurnCompleted(current, requiredTools) ? current : undefined
  }, 5 * 60_000)
  return {
    messages,
    trace: messages.filter(message => message.type === 'tool-call').map(message => ({
      callId: message.callId,
      toolName: message.toolName,
      args: message.args ?? {},
      status: message.status,
    })),
  }
}

export async function waitFor<T>(read: () => Promise<T | undefined>, timeoutMs: number): Promise<T> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const value = await read()
    if (value !== undefined) return value
    await new Promise(resolve => setTimeout(resolve, 500))
  }
  throw coded('manual_case_timeout')
}

async function apiJson(api: APIRequestContext, path: string, method: 'GET' | 'POST' = 'GET') {
  const response = method === 'POST' ? await api.post(path) : await api.get(path)
  return checkedJson(response)
}

async function checkedJson(response: { ok(): boolean; status(): number; json(): Promise<any> }) {
  const body = await response.json().catch(() => ({}))
  if (!response.ok()) {
    const failure = coded(typeof body.code === 'string' ? body.code : `http_${response.status()}`)
    throw failure
  }
  return body
}

function flattenExamples(nodes: ExampleTreeNode[]): ExampleSummary[] {
  return nodes.flatMap(node => node.type === 'example' && node.example
    ? [node.example]
    : flattenExamples(node.children ?? []))
}

function coded(code: string) {
  return Object.assign(new Error(code), { code })
}

function errorCode(cause: unknown, fallback: string) {
  if (cause && typeof cause === 'object' && 'code' in cause) {
    const code = (cause as { code?: unknown }).code
    if (typeof code === 'string' && /^[a-z][a-z0-9_]*$/.test(code)) return code
  }
  return fallback
}

function escapeRegex(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}
