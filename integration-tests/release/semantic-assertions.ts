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
  gatedTools?: Record<string, string[]>
}

export interface ExpectedDomainState {
  sessionId: string
  minimumVersions: number
  taskStudyTypes: string[]
  minimumAnalysisRuns: number
  minimumComparisons: number
  activeVersionId?: string
}

const allowedTools = new Set([
  'inspect_workspace', 'search_examples', 'inspect_example', 'import_example',
  'remove_model_input', 'replace_model_input', 'confirm_dataframe_mapping', 'modify_model',
  'activate_model_version', 'rollback_model_version', 'run_study', 'list_result_datasets',
  'run_result_analysis', 'create_dataset_view', 'compare_simulations',
  'list_simulation_history', 'search_chat_history', 'read_chat_context', 'get_storage_usage',
  'load_agent_skill', 'search_engine_docs', 'read_engine_doc_section',
])

function invariant(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

function assertStableIds(call: ToolTraceEntry) {
  const visit = (value: unknown, key = '') => {
    if (key === 'exampleId' || key === 'example_id') {
      invariant(typeof value === 'string' && /^ex_[a-f0-9]+$/.test(value), `${call.toolName}.${key} is not a stable example ID`)
    } else if (key === 'engineResourceSnapshot') {
      invariant(typeof value === 'string' && /^ers_[a-f0-9]+$/.test(value), `${call.toolName}.${key} is not a stable snapshot ID`)
    } else if (/Id$/.test(key) && key !== 'callId') {
      invariant(Number.isInteger(value) && Number(value) > 0, `${call.toolName}.${key} is not a positive stable ID`)
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
  const failed = trace.find((call) => call.status === 'failed')
  invariant(!failed, `tool ${failed?.toolName ?? 'unknown'} failed`)
}

async function json(api: SemanticApiContext, path: string) {
  const response = await api.get(path)
  invariant(response.ok(), `${path} returned ${response.status()}`)
  return response.json()
}

export async function expectDomainState(api: SemanticApiContext, expected: ExpectedDomainState) {
  const workspace = await json(api, `/api/sessions/${expected.sessionId}/workspace`)
  const versions = await json(api, `/api/sessions/${expected.sessionId}/versions`)
  const tasks = await json(api, `/api/sessions/${expected.sessionId}/tasks`)
  const messages = (await json(api, `/api/sessions/${expected.sessionId}/messages?limit=100`)).messages as any[]
  invariant(versions.length >= expected.minimumVersions, `expected at least ${expected.minimumVersions} versions, got ${versions.length}`)
  if (expected.activeVersionId) {
    invariant(workspace.active?.versionId === expected.activeVersionId, 'active version does not match the expected stable ID')
  }
  const succeededStudies = tasks.filter((task: any) => task.status === 'succeeded').map((task: any) => task.studyType)
  for (const studyType of expected.taskStudyTypes) {
    const index = succeededStudies.indexOf(studyType)
    invariant(index >= 0, `missing succeeded ${studyType} task`)
    succeededStudies.splice(index, 1)
  }
  const analysisIds = [...new Set(messages.map((message) => message.analysisRunId).filter(Boolean))]
  const comparisonIds = [...new Set(messages.filter((message) => message.type === 'comparison').map((message) => message.comparisonId))]
  invariant(analysisIds.length >= expected.minimumAnalysisRuns, `expected at least ${expected.minimumAnalysisRuns} analysis runs`)
  invariant(comparisonIds.length >= expected.minimumComparisons, `expected at least ${expected.minimumComparisons} comparisons`)
  const datasets = []
  for (const task of tasks.filter((task: any) => task.status === 'succeeded')) {
    datasets.push(...await json(api, `/api/tasks/${task.id}/datasets`))
  }
  for (const id of analysisIds) {
    const analysis = await json(api, `/api/analysis-runs/${id}`)
    invariant(analysis.status === 'succeeded', `analysis ${id} did not succeed`)
  }
  for (const id of comparisonIds) {
    await json(api, `/api/task-comparisons/${id}`)
  }
  return {
    activeVersionId: workspace.active?.versionId ?? null,
    versionCount: versions.length,
    taskCount: tasks.length,
    datasetCount: datasets.length,
    analysisRunCount: analysisIds.length,
    comparisonCount: comparisonIds.length,
    studyTypes: tasks.map((task: any) => task.studyType),
  }
}
