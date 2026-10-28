interface ApiResponse {
  ok(): boolean
  status(): number
  json(): Promise<any>
}

export interface SemanticApiContext {
  get(path: string): Promise<ApiResponse>
}

export interface ToolTraceEntry {
  callId: string
  toolName: string
  args: Record<string, unknown>
  status: 'running' | 'success' | 'failed'
}

export interface ExpectedToolDependencies {
  requiredTools: string[]
  allowedTools?: string[]
  gatedTools?: Record<string, string[]>
}

export interface ExpectedDomainState {
  sessionId: string
  expectedVersions: number
  expectedRuns: number
  expectedDatasets: number
  expectedAnalysisRuns: number
  expectedComparisons: number
  activeVersionId?: string
}

export function expectEngineReleaseEvidence(report: any) {
  invariant(report?.engineContract === 'tensoreval-parquet-v3', 'engine contract is missing or unsupported')
  invariant(/^ers_[a-f0-9]{32}$/.test(report.engineResourceSnapshot ?? ''), 'engine resource snapshot is invalid')
  invariant(/^[a-f0-9]{64}$/.test(report.binarySha256 ?? ''), 'engine binary hash is invalid')
  invariant(report.bundleExamples > 0, 'release engine has no bundle examples')
  invariant(report.importableExamples > 0, 'release engine has no importable examples')
  invariant(report.facadeCatalogEntries > 0, 'release engine has no facade catalog entries')
  invariant(report.enabledFacades === report.facadeCatalogEntries, 'release engine did not enable every facade')
  invariant(Array.isArray(report.facadeDiagnostics) && report.facadeDiagnostics.length === 0,
    'release engine has facade diagnostics')
  invariant(report.contractChecks?.staticModelImport && report.contractChecks?.staticImportDataframes === 21,
    'release engine static import contract is incomplete')
  invariant(
    report.bundleExamples === report.importableExamples + report.nonImportableExamples
      && report.nonImportableDetails?.length === report.nonImportableExamples,
    'release engine example counts are inconsistent',
  )
}

interface AgentTurnMessage {
  id: string | number
  type: string
  callId?: string
  toolName?: string
  status?: string
}

const allowedTools = new Set([
  'inspect_workspace', 'read_model_dataframe', 'search_study_facades', 'prepare_study',
  'search_examples', 'inspect_example', 'modify_model',
  'list_result_datasets',
  'run_result_analysis', 'create_dataset_view', 'compare_simulations',
  'list_simulation_history', 'search_chat_history', 'read_chat_context', 'get_storage_usage',
  'load_agent_skill', 'search_engine_docs', 'read_engine_doc_section',
  'list_data_resources', 'read_data_resource', 'update_data_resource_alias', 'promote_result_dataset',
])
const sideEffectTools = new Set([
  'prepare_study', 'modify_model', 'update_data_resource_alias', 'promote_result_dataset',
  'run_result_analysis', 'create_dataset_view', 'compare_simulations',
])

function invariant(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

export function agentTurnCompleted(messages: AgentTurnMessage[], requiredTools: string[] = []) {
  const calls = messages.filter((message) => message.type === 'tool-call')
  const running = new Set(calls.filter((message) => message.status === 'running').map((message) => message.callId))
  const terminal = new Set(calls.filter((message) => message.status !== 'running').map((message) => message.callId))
  if (![...running].every((id) => terminal.has(id))) return false
  if (!requiredTools.every((tool) => calls.some((call) => call.toolName === tool && call.status === 'success'))) return false
  const assistantIds = messages
    .filter((message) => message.type === 'assistant')
    .map((message) => Number(message.id))
  if (assistantIds.length === 0) return false
  const lastAssistantId = Math.max(...assistantIds)
  const lastToolId = Math.max(0, ...calls.map((message) => Number(message.id)))
  return lastAssistantId > lastToolId
}

function assertStableIds(call: ToolTraceEntry) {
  const visit = (value: unknown, key = '') => {
    if (value === null || value === undefined) return
    if (key === 'exampleId' || key === 'example_id') {
      invariant(typeof value === 'string' && /^ex_[a-f0-9]+$/.test(value), `${call.toolName}.${key} is not a stable example ID`)
    } else if (key === 'engineResourceSnapshot') {
      invariant(typeof value === 'string' && /^ers_[a-f0-9]+$/.test(value), `${call.toolName}.${key} is not a stable snapshot ID`)
    } else if (/Id$/.test(key) && key !== 'callId') {
      invariant(
        (Number.isInteger(value) && Number(value) > 0)
          || (typeof value === 'string' && /^[A-Za-z][A-Za-z0-9:_-]*$/.test(value)),
        `${call.toolName}.${key} is not a stable ID`,
      )
    }
    if (typeof value === 'string') {
      invariant(!value.startsWith('/') && !value.includes('../') && !value.includes('://'), `${call.toolName} contains a path or URL argument`)
    } else if (Array.isArray(value)) {
      value.forEach((item) => visit(item))
    } else if (value && typeof value === 'object') {
      Object.entries(value).forEach(([childKey, child]) => visit(child, childKey))
    }
  }
  visit(call.args)
}

export function expectToolTrace(trace: ToolTraceEntry[], expectedDependencies: ExpectedToolDependencies) {
  invariant(trace.length > 0, 'tool trace is empty')
  for (const call of trace) {
    invariant(allowedTools.has(call.toolName), `tool trace contains disallowed tool ${call.toolName}`)
    assertStableIds(call)
  }
  for (const required of expectedDependencies.requiredTools) {
    invariant(
      trace.some((call) => call.toolName === required && call.status === 'success'),
      `required tool ${required} did not succeed`,
    )
  }
  const expectedAllowed = expectedDependencies.allowedTools
    ? new Set(expectedDependencies.allowedTools)
    : null
  if (expectedAllowed) {
    for (const call of trace.filter((candidate) => candidate.status === 'success')) {
      invariant(expectedAllowed.has(call.toolName), `unexpected successful tool ${call.toolName}`)
    }
  }
  for (const tool of sideEffectTools) {
    const expectedCount = expectedDependencies.requiredTools.filter((required) => required === tool).length
    const actualCount = trace.filter((call) => call.toolName === tool && call.status === 'success').length
    invariant(actualCount === expectedCount, `${tool} succeeded ${actualCount} times; expected ${expectedCount}`)
  }
  for (const [tool, skills] of Object.entries(expectedDependencies.gatedTools ?? {})) {
    const gatedIndex = trace.findIndex((call) => call.toolName === tool && call.status === 'running')
    invariant(gatedIndex >= 0, `gated tool ${tool} was not called`)
    for (const skill of skills) {
      const loadedIndex = trace.findIndex((call) =>
        call.toolName === 'load_agent_skill'
        && call.status === 'success'
        && call.args.skill_id === skill,
      )
      invariant(loadedIndex >= 0 && loadedIndex < gatedIndex, `${skill} was not loaded before ${tool}`)
    }
  }
  const unrecovered = trace.find((call, index) => call.status === 'failed'
    && !trace.slice(index + 1).some((later) => later.status === 'success'
      && (!sideEffectTools.has(call.toolName) || later.toolName === call.toolName)))
  invariant(!unrecovered, `tool ${unrecovered?.toolName ?? 'unknown'} failed without recovery`)
}

async function json(api: SemanticApiContext, path: string) {
  const response = await api.get(path)
  invariant(response.ok(), `${path} returned ${response.status()}`)
  return response.json()
}

export async function readAllMessages(api: SemanticApiContext, sessionId: string) {
  const messages: any[] = []
  let beforeId: string | undefined
  for (let page = 0; page < 20; page += 1) {
    const query = beforeId ? `?limit=100&beforeId=${beforeId}` : '?limit=100'
    const response = await json(api, `/api/sessions/${sessionId}/messages${query}`)
    messages.push(...response.messages)
    if (!response.hasMoreBefore || !response.nextBeforeId) break
    beforeId = String(response.nextBeforeId)
  }
  return [...new Map(messages.map((message) => [String(message.id), message])).values()]
}

export async function expectDomainState(api: SemanticApiContext, expected: ExpectedDomainState) {
  const workspace = await json(api, `/api/sessions/${expected.sessionId}/workspace`)
  const versions = (await json(api, `/api/sessions/${expected.sessionId}/versions`)).versions as any[]
  const runs = (await json(api, `/api/sessions/${expected.sessionId}/runs`)).runs as any[]
  const messages = await readAllMessages(api, expected.sessionId)
  invariant(versions.length === expected.expectedVersions, `expected exactly ${expected.expectedVersions} versions, got ${versions.length}`)
  if (expected.activeVersionId) {
    invariant(workspace.active?.id === expected.activeVersionId, 'active version does not match the expected stable ID')
  }
  invariant(runs.length === expected.expectedRuns, `expected exactly ${expected.expectedRuns} runs, got ${runs.length}`)
  const analysisIds = [...new Set(messages.map((message) => message.analysisRunId).filter(Boolean))]
  const comparisonIds = [...new Set(messages.filter((message) => message.type === 'comparison').map((message) => message.comparisonId))]
  invariant(analysisIds.length === expected.expectedAnalysisRuns, `expected exactly ${expected.expectedAnalysisRuns} analysis runs`)
  invariant(comparisonIds.length === expected.expectedComparisons, `expected exactly ${expected.expectedComparisons} comparisons`)
  const datasets = []
  for (const run of runs.filter((run: any) => run.status === 'succeeded')) {
    const response = await json(api, `/api/runs/${run.id}/datasets`)
    datasets.push(...(response.datasets ?? response))
  }
  const analyses = []
  for (const id of analysisIds) analyses.push(await json(api, `/api/analysis-runs/${id}`))
  const succeededAnalyses = analyses.filter((analysis) => analysis.status === 'succeeded')
  invariant(succeededAnalyses.length === expected.expectedAnalysisRuns,
    `expected exactly ${expected.expectedAnalysisRuns} succeeded analysis runs`)
  for (const id of comparisonIds) {
    await json(api, `/api/run-comparisons/${id}`)
  }
  invariant(datasets.length === expected.expectedDatasets,
    `expected exactly ${expected.expectedDatasets} datasets, got ${datasets.length}`)
  return {
    activeVersionId: workspace.active?.id ?? null,
    versionCount: versions.length,
    runCount: runs.length,
    datasetCount: datasets.length,
    analysisRunCount: succeededAnalyses.length,
    comparisonCount: comparisonIds.length,
    runStatuses: runs.map((run: any) => run.status),
  }
}
