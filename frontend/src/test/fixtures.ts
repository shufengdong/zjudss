import type {
  ChartCapabilities,
  FailureDiagnosis,
  SimulationRun,
  UploadBatchView,
} from '../types'

export const simulationRun = (overrides: Partial<SimulationRun> = {}): SimulationRun => ({
  id: '9',
  sessionId: '1',
  modelVersionId: '3',
  modelContentHash: 'm'.repeat(64),
  scriptResourceId: '4',
  scriptName: 'manual-study.rss',
  scriptArtifactId: '5',
  scriptContentHash: 's'.repeat(64),
  scriptMessageId: null,
  managedWorkflowId: null,
  clientRunId: 'client-9',
  status: 'succeeded',
  stage: 'completed',
  engineContractVersion: 'tensoreval-parquet-v3',
  agentBundleVersion: null,
  agentSkillRefs: [],
  exitCode: 0,
  error: null,
  createdAt: '2026-01-01T00:00:00.000Z',
  startedAt: '2026-01-01T00:00:01.000Z',
  finishedAt: '2026-01-01T00:00:03.500Z',
  ...overrides,
})

export const failureDiagnosis = (
  overrides: Partial<FailureDiagnosis> = {},
): FailureDiagnosis => ({
  code: 'run_failed',
  message: '仿真引擎执行失败',
  stage: 'run',
  exitCode: 7,
  durationMs: 1_234,
  engineLogs: [
    { stream: 'stdout', text: 'engine banner', totalBytes: 13, sentBytes: 13, truncated: false },
    { stream: 'stderr', text: 'line one\nline two', totalBytes: 99, sentBytes: 17, truncated: true },
  ],
  analysisStatus: 'complete',
  analysis: {
    summary: '脚本解析失败',
    facts: [{ statement: '引擎报告解析失败', evidence: 'stderr: line one' }],
    possibleCauses: [{ statement: '当前脚本可能存在语法错误', evidence: 'stderr: line two' }],
    unknowns: ['具体语法位置'],
    recommendations: ['核对脚本语法'],
  },
  ...overrides,
})

export const uploadBatch = (overrides: Partial<UploadBatchView> = {}): UploadBatchView => ({
  id: '12',
  sessionId: '1',
  clientBatchId: 'client-12',
  sourceKind: 'upload',
  replacementResourceId: null,
  status: 'awaiting_purpose',
  modelPublishStatus: 'ready',
  resourcePublishStatus: 'ready',
  parseSettings: {},
  diagnostics: {},
  reservedBytes: 0,
  expiresAtMs: 10,
  revision: 1,
  candidates: [],
  ...overrides,
})

export const chartCapabilities = (
  overrides: Partial<ChartCapabilities> = {},
): ChartCapabilities => ({
  datasetId: '7',
  schemaHash: 'h',
  rowCount: 3,
  fields: [
    {
      column: 'bus', dtype: 'Utf8', nullable: false, unit: null,
      scalarType: 'string', roles: ['category'], shape: { kind: 'scalar' },
    },
    {
      column: 'v_mag', dtype: 'List(Float64)', nullable: false, unit: 'p.u.',
      scalarType: 'number', roles: ['measure'],
      shape: {
        kind: 'list', rank: 1, observedPaths: [[0], [1], [2]],
        complete: true, truncated: false, reason: null,
      },
    },
    {
      column: 'wide', dtype: 'List(Float64)', nullable: false, unit: null,
      scalarType: 'number', roles: ['measure'],
      shape: {
        kind: 'list', rank: 1, observedPaths: [[0]],
        complete: false, truncated: true, reason: 'too_many_components',
      },
    },
  ],
  templates: (['line', 'bar', 'scatter', 'heatmap', 'boxplot', 'histogram'] as const)
    .map((kind) => ({ kind, enabled: true, reason: null })),
  limits: {
    maxFields: 256,
    maxObservedPathsPerField: 64,
    maxSeries: 100,
    maxPoints: 50_000,
    maxCategories: 5_000,
    maxHeatmapCells: 100_000,
  },
  ...overrides,
})
