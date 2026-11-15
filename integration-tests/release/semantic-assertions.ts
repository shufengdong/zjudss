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
  invariant(report.nonImportableExamples === 0, 'release engine has cases without an enabled projection')
  invariant(report.facadeCatalogEntries > 0, 'release engine has no facade catalog entries')
  invariant(report.enabledFacades === report.facadeCatalogEntries, 'release engine did not enable every facade')
  invariant(Array.isArray(report.facadeDiagnostics) && report.facadeDiagnostics.length === 0,
    'release engine has facade diagnostics')
  invariant(report.projectionCatalogEntries > 0 && report.enabledProjectionProfiles > 0,
    'release engine has no projection catalog')
  invariant(
    report.enabledProjectionProfiles + (report.projectionDiagnostics?.length ?? -1) === report.projectionCatalogEntries,
    'release engine projection catalog is incomplete',
  )
  invariant(
    report.projectedExamples === report.importableExamples
      && report.projectedExamples === report.bundleExamples,
    'release engine did not project every bundle example',
  )
  invariant(report.projectionFailures === 0, 'release engine case projection failed')
  invariant(
    Array.isArray(report.caseProjections) && report.caseProjections.length === report.projectedExamples,
    'release engine case projection evidence is incomplete',
  )
  const projectedIds = new Set<string>()
  for (const projection of report.caseProjections) {
    invariant(
      typeof projection.exampleId === 'string' && /^ex_[a-f0-9]+$/.test(projection.exampleId),
      'release engine case projection example ID is invalid',
    )
    invariant(!projectedIds.has(projection.exampleId), 'release engine has duplicate case projection evidence')
    projectedIds.add(projection.exampleId)
    invariant(
      typeof projection.projectionProfile === 'string' && projection.projectionProfile.length > 0,
      'release engine case projection profile is missing',
    )
    invariant(projection.success === true, 'release engine case projection did not succeed')
    for (const field of ['modelOutputs', 'dataOutputs', 'structOutputs', 'diagnosticOutputs']) {
      invariant(
        Number.isInteger(projection[field]) && projection[field] >= 0,
        `release engine case projection ${field} is invalid`,
      )
    }
    invariant(projection.failureCode === undefined, 'successful case projection includes a failure code')
  }
  const projectedProfiles = new Set<string>(
    report.caseProjections.map((projection: any) => projection.projectionProfile),
  )
  invariant(
    projectedProfiles.size === report.referencedProjectionProfiles,
    'release engine projected profile coverage is inconsistent',
  )
  invariant(
    report.referencedProjectionProfiles > 0
      && Array.isArray(report.representativeRuns)
      && report.representativeRuns.length === report.referencedProjectionProfiles,
    'release engine did not project and run every referenced profile',
  )
  const representativeProfiles = new Set<string>()
  for (const run of report.representativeRuns) {
    invariant(
      typeof run.projectionProfile === 'string'
        && projectedProfiles.has(run.projectionProfile)
        && !representativeProfiles.has(run.projectionProfile),
      'release engine representative profiles are incomplete',
    )
    representativeProfiles.add(run.projectionProfile)
    invariant(
      run.success === true
        && typeof run.exampleId === 'string'
        && /^ex_[a-f0-9]+$/.test(run.exampleId)
        && report.caseProjections.some((projection: any) =>
          projection.success
          && projection.exampleId === run.exampleId
          && projection.projectionProfile === run.projectionProfile),
      'release engine representative does not match projected evidence',
    )
  }
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

const sideEffectTools = new Set([
  'prepare_study', 'update_study_results', 'modify_model', 'publish_model_version', 'update_data_resource_alias', 'promote_result_dataset',
  'run_result_analysis', 'compare_simulations',
])

function invariant(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

export interface ExpectedPhaseVoltageRow {
  time: number
  bus: string
  phase: string
  vPu: number
}

export function expectPhaseVoltageRows(
  rows: unknown[][],
  expected: ExpectedPhaseVoltageRow[],
  tolerance = 1e-9,
) {
  invariant(rows.length === expected.length, `phase-voltage row count mismatch: ${rows.length} != ${expected.length}`)
  const expectedByKey = new Map<string, number>()
  for (const row of expected) {
    invariant(Number.isFinite(row.time) && Number.isFinite(row.vPu), 'expected phase-voltage value is invalid')
    const key = JSON.stringify([row.time, row.bus, row.phase])
    invariant(!expectedByKey.has(key), `duplicate expected phase-voltage identity: ${key}`)
    expectedByKey.set(key, row.vPu)
  }
  for (const row of rows) {
    invariant(row.length === 4, 'phase-voltage row must contain four values')
    const [time, bus, phase, vPu] = row
    invariant(typeof time === 'number' && Number.isFinite(time), 'phase-voltage time must be a finite number')
    invariant(typeof bus === 'string' && bus.length > 0, 'phase-voltage bus must be a nonempty string')
    invariant(typeof phase === 'string' && phase.length > 0, 'phase-voltage phase must be a nonempty string')
    invariant(typeof vPu === 'number' && Number.isFinite(vPu), 'phase-voltage v_pu must be a finite number')
    const key = JSON.stringify([time, bus, phase])
    const expectedValue = expectedByKey.get(key)
    invariant(expectedValue !== undefined, `unexpected phase-voltage identity: ${key}`)
    const scale = Math.max(1, Math.abs(expectedValue))
    invariant(Math.abs(vPu - expectedValue) <= tolerance * scale, `phase-voltage value mismatch for ${key}`)
    expectedByKey.delete(key)
  }
  invariant(expectedByKey.size === 0, 'phase-voltage output is missing expected identities')
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
    if (key === 'configurationId' || key === 'configuration_id') {
      invariant(typeof value === 'string' && /^[1-9][0-9]*$/.test(value), `${call.toolName}.${key} is not a stable configuration ID`)
    } else if (key === 'exampleId' || key === 'example_id') {
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

export function expectToolTrace(
  trace: ToolTraceEntry[],
  expectedDependencies: ExpectedToolDependencies,
  runtimeAllowedTools: readonly string[],
) {
  invariant(trace.length > 0, 'tool trace is empty')
  const allowedTools = new Set(runtimeAllowedTools)
  invariant(allowedTools.size > 0 && allowedTools.size === runtimeAllowedTools.length,
    'runtime allowed tools are empty or duplicated')
  invariant(runtimeAllowedTools.every(tool => typeof tool === 'string' && tool.trim() === tool && tool.length > 0),
    'runtime allowed tools contain an invalid name')
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
    const successIndex = trace.findIndex((call) => call.toolName === tool && call.status === 'success')
    const successCall = trace[successIndex]
    let gatedIndex = -1
    for (let index = successIndex - 1; index >= 0; index -= 1) {
      const call = trace[index]
      if (call.toolName === tool && call.status === 'running' && call.callId === successCall?.callId) {
        gatedIndex = index
        break
      }
    }
    if (gatedIndex < 0) {
      for (let index = successIndex - 1; index >= 0; index -= 1) {
        const call = trace[index]
        if (call.toolName === tool && call.status === 'running') {
          gatedIndex = index
          break
        }
      }
    }
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
  const lastRequiredSuccessIndex = expectedDependencies.requiredTools.reduce((latest, tool) => {
    for (let index = trace.length - 1; index >= 0; index -= 1) {
      const call = trace[index]
      if (call.toolName === tool && call.status === 'success') {
        return Math.max(latest, index)
      }
    }
    return latest
  }, -1)
  const unrecovered = trace.find((call, index) => call.status === 'failed'
    && (sideEffectTools.has(call.toolName) || index < lastRequiredSuccessIndex)
    && !trace.slice(index + 1).some((later) => later.status === 'success'
      && (!sideEffectTools.has(call.toolName) || later.toolName === call.toolName)))
  invariant(!unrecovered, `tool ${unrecovered?.toolName ?? 'unknown'} failed without recovery`)
}

export function expectDirectAnalysisTrace(
  trace: ToolTraceEntry[],
  runtimeAllowedTools: readonly string[],
) {
  const directTools = [
    'load_agent_skill',
    'inspect_workspace',
    'read_model_dataframe',
    'read_result_dataset',
    'search_engine_docs',
    'read_engine_doc_section',
    'run_result_analysis',
  ]
  expectToolTrace(trace, {
    requiredTools: directTools,
    allowedTools: directTools,
    gatedTools: { run_result_analysis: ['result-analysis'] },
  }, runtimeAllowedTools)
  invariant(!trace.some(call => call.toolName === 'load_agent_skill'
    && call.args.skill_id !== 'result-analysis'), 'direct analysis loaded an unrelated skill')
  const forbidden = new Set([
    'modify_model', 'publish_model_version', 'prepare_study', 'update_study_results',
    'create_script', 'update_script', 'run_script', 'run_simulation',
  ])
  invariant(!trace.some(call => forbidden.has(call.toolName)),
    'direct analysis invoked a modification, publication, study-confirmation, script, or run tool')
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
