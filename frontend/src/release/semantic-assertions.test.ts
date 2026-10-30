import { describe, expect, test } from 'vitest'
import {
  agentTurnCompleted,
  expectEngineReleaseEvidence,
  expectToolTrace,
  readAllMessages,
  type ToolTraceEntry,
} from '../../../integration-tests/release/semantic-assertions'

const call = (toolName: string, status: ToolTraceEntry['status'], args: Record<string, unknown> = {}): ToolTraceEntry => ({
  callId: `${toolName}-${status}`, toolName, status, args,
})

describe('release semantic assertions', () => {
  test('accepts the bundle-only engine report and rejects inconsistent counts', () => {
    const caseProjections = Array.from({ length: 103 }, (_, index) => ({
      exampleId: `ex_${index.toString(16).padStart(8, '0')}`,
      projectionProfile: `profile_${index % 6}`,
      success: true,
      modelOutputs: index === 0 ? 0 : 20,
      dataOutputs: 1,
      structOutputs: index === 0 ? 1 : 0,
      diagnosticOutputs: 0,
    }))
    const report = {
      engineContract: 'tensoreval-parquet-v3',
      engineResourceSnapshot: `ers_${'a'.repeat(32)}`,
      binarySha256: 'b'.repeat(64),
      bundleExamples: 103,
      importableExamples: 103,
      nonImportableExamples: 0,
      nonImportableDetails: [],
      facadeCatalogEntries: 15,
      enabledFacades: 15,
      facadeDiagnostics: [],
      projectionCatalogEntries: 9,
      enabledProjectionProfiles: 7,
      referencedProjectionProfiles: 6,
      projectionDiagnostics: ["PSA.DSS.AUTOADD.UNSUPPORTED", "PSA.DSS.RELCALC.UNSUPPORTED"],
      projectedExamples: 103,
      projectionFailures: 0,
      caseProjections,
      representativeRuns: Array.from({ length: 6 }, (_, index) => ({
        exampleId: caseProjections[index].exampleId,
        success: true,
        projectionProfile: `profile_${index}`,
      })),
      contractChecks: { staticModelImport: true, staticImportDataframes: 21 },
    }
    expect(() => expectEngineReleaseEvidence(report)).not.toThrow()
    expect(() => expectEngineReleaseEvidence({
      ...report,
      nonImportableExamples: 3,
    })).toThrow(/enabled projection/)
    expect(() => expectEngineReleaseEvidence({
      ...report,
      projectedExamples: 102,
    })).toThrow(/project every bundle example/)
    expect(() => expectEngineReleaseEvidence({
      ...report,
      projectionFailures: 1,
    })).toThrow(/case projection failed/)
    expect(() => expectEngineReleaseEvidence({
      ...report,
      caseProjections: caseProjections.slice(1),
    })).toThrow(/case projection evidence is incomplete/)
    expect(() => expectEngineReleaseEvidence({
      ...report,
      caseProjections: [caseProjections[0], ...caseProjections.slice(0, -1)],
    })).toThrow(/duplicate case projection evidence/)
    expect(() => expectEngineReleaseEvidence({
      ...report,
      caseProjections: [
        { ...caseProjections[0], success: false, failureCode: 'model_import_failed' },
        ...caseProjections.slice(1),
      ],
    })).toThrow(/case projection did not succeed/)
    expect(() => expectEngineReleaseEvidence({
      ...report,
      representativeRuns: Array.from({ length: 6 }, () => report.representativeRuns[0]),
    })).toThrow(/representative profiles are incomplete/)
    expect(() => expectEngineReleaseEvidence({
      ...report,
      caseProjections: caseProjections.map((projection) => ({
        ...projection,
        projectionProfile: 'unknown_profile',
      })),
    })).toThrow(/projected profile coverage is inconsistent/)
    expect(() => expectEngineReleaseEvidence({
      ...report,
      representativeRuns: [
        { ...report.representativeRuns[0], exampleId: 'ex_ffffffff' },
        ...report.representativeRuns.slice(1),
      ],
    })).toThrow(/representative does not match projected evidence/)
  })

  test('waits for an assistant response after the final tool result', () => {
    expect(agentTurnCompleted([
      { id: 1, type: 'assistant' },
      { id: 2, type: 'tool-call', callId: 'inspect', toolName: 'inspect_workspace', status: 'running' },
      { id: 3, type: 'tool-call', callId: 'inspect', toolName: 'inspect_workspace', status: 'success' },
      { id: 4, type: 'assistant' },
    ], ['prepare_study'])).toBe(false)
    expect(agentTurnCompleted([
      { id: 1, type: 'assistant' },
      { id: 2, type: 'tool-call', callId: 'inspect', toolName: 'inspect_workspace', status: 'success' },
      { id: 3, type: 'tool-call', callId: 'prepare', toolName: 'prepare_study', status: 'success' },
      { id: 4, type: 'assistant' },
    ], ['prepare_study'])).toBe(true)
  })

  test('accepts stable IDs and skills loaded before gated tools', () => {
    expect(() => expectToolTrace([
      call('load_agent_skill', 'success', { skill_id: 'study-planning' }),
      call('search_study_facades', 'success', { explicitFunction: 'ds_newton_raphson_pf' }),
      call('prepare_study', 'running', { facadeId: 'static_power_flow_newton', derivedFromWorkflowId: null }),
      call('prepare_study', 'success', { facadeId: 'static_power_flow_newton', derivedFromWorkflowId: null }),
    ], {
      requiredTools: ['prepare_study'],
      gatedTools: { prepare_study: ['study-planning'] },
    })).not.toThrow()
    expect(() => expectToolTrace([
      call('load_agent_skill', 'success', { skill_id: 'study-planning' }),
      call('search_study_facades', 'success', { explicitFunction: 'ds_newton_raphson_pf' }),
      call('prepare_study', 'running', { facadeId: 'static_power_flow_newton' }),
      call('prepare_study', 'success', { facadeId: 'static_power_flow_newton' }),
    ], {
      requiredTools: ['search_study_facades', 'prepare_study'],
      gatedTools: { prepare_study: ['study-planning'] },
    })).not.toThrow()
  })

  test('rejects illegal tools, unstable IDs, failed tools, and missing skill ordering', () => {
    expect(() => expectToolTrace([call('shell', 'success')], { requiredTools: ['shell'] })).toThrow(/disallowed/)
    expect(() => expectToolTrace([call('inspect_example', 'success', { exampleId: '/tmp/case.rss' })], { requiredTools: ['inspect_example'] })).toThrow(/stable example ID/)
    expect(() => expectToolTrace([call('prepare_study', 'failed', { facadeId: 'test' })], { requiredTools: ['prepare_study'] })).toThrow(/did not succeed/)
    expect(() => expectToolTrace([
      call('prepare_study', 'running', { facadeId: 'test' }),
      call('load_agent_skill', 'success', { skill_id: 'study-planning' }),
      call('prepare_study', 'success', { facadeId: 'test' }),
    ], { requiredTools: ['prepare_study'], gatedTools: { prepare_study: ['study-planning'] } })).toThrow(/not loaded before/)
  })

  test('requires a failed side effect to recover with the same tool', () => {
    expect(() => expectToolTrace([
      call('run_result_analysis', 'running', { datasetId: 1 }),
      call('run_result_analysis', 'failed', { datasetId: 1 }),
      call('run_result_analysis', 'running', { datasetId: 1 }),
      call('run_result_analysis', 'success', { datasetId: 1 }),
    ], { requiredTools: ['run_result_analysis'] })).not.toThrow()
    expect(() => expectToolTrace([
      call('run_result_analysis', 'failed', { datasetId: 1 }),
    ], { requiredTools: [] })).toThrow(/failed without recovery/)
  })

  test('accepts a failed read when a later read supplies evidence and the side effect succeeds', () => {
    expect(() => expectToolTrace([
      call('read_engine_doc_section', 'failed'),
      call('search_engine_docs', 'success'),
      call('run_result_analysis', 'success', { datasetId: 1 }),
    ], {
      requiredTools: ['run_result_analysis'],
      allowedTools: ['read_engine_doc_section', 'search_engine_docs', 'run_result_analysis'],
    })).not.toThrow()
  })

  test('rejects unexpected successful tools and repeated side effects', () => {
    expect(() => expectToolTrace([
      call('inspect_workspace', 'success'),
      call('search_examples', 'success'),
    ], {
      requiredTools: ['inspect_workspace'],
      allowedTools: ['inspect_workspace'],
    })).toThrow(/unexpected successful tool search_examples/)

    expect(() => expectToolTrace([
      call('prepare_study', 'success', { facadeId: 'test' }),
      call('prepare_study', 'success', { facadeId: 'test' }),
    ], {
      requiredTools: ['prepare_study'],
      allowedTools: ['prepare_study'],
    })).toThrow(/prepare_study succeeded 2 times/)
  })

  test('paginates the complete message history with nextBeforeId', async () => {
    const paths: string[] = []
    const api = {
      async get(path: string) {
        paths.push(path)
        const second = path.includes('beforeId=101')
        return {
          ok: () => true,
          status: () => 200,
          json: async () => second
            ? { messages: [{ id: '1' }], hasMoreBefore: false, nextBeforeId: '1' }
            : { messages: [{ id: '101' }], hasMoreBefore: true, nextBeforeId: '101' },
        }
      },
    }

    await expect(readAllMessages(api, '7')).resolves.toEqual([{ id: '101' }, { id: '1' }])
    expect(paths).toEqual([
      '/api/sessions/7/messages?limit=100',
      '/api/sessions/7/messages?limit=100&beforeId=101',
    ])
  })
})
