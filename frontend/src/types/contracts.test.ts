import { afterEach, vi } from 'vitest'
import { MESSAGE_TYPES, type MessageType } from './message'
import { VISUALIZATION_KINDS, type VisualizationKind } from './dataset'
import type { DatasetPage, ModelDataFramePage, OpenDataFrame } from './index'
import type { ChartConfig, ChartModel, ChartViewSummary, CategoryValue } from './chart'
import { createChartView, deleteChartView, getChartCapabilities, getChartView, getChartViewData,
  listChartViews, previewChart } from '../api/client'
import { chartCapabilities } from '../test/fixtures'

afterEach(() => vi.unstubAllGlobals())

describe('frontend public enum contracts', () => {
  it('exports all message types from the union source', () => {
    const values: readonly MessageType[] = MESSAGE_TYPES
    expect(values).toEqual([
      'user', 'assistant', 'tool-call', 'progress',
      'result', 'attachment', 'model-status', 'comparison', 'script',
      'upload-batch', 'study-config', 'workflow-progress',
    ])
  })

  it('exports all visualization kinds from the union source', () => {
    const values: readonly VisualizationKind[] = VISUALIZATION_KINDS
    expect(values).toEqual(['line', 'bar', 'scatter', 'heatmap', 'boxplot', 'histogram'])
  })
})

describe('dataframe identity contracts', () => {
  it('keeps result and model resources as a discriminated union', () => {
    const sources: OpenDataFrame[] = [
      { kind: 'result', datasetId: 'result-1' },
      { kind: 'model-version', dataframeId: 'frame-1', versionId: 'version-1', tableName: 'bus' },
      { kind: 'model-draft', dataframeId: 'frame-2', draftId: 'draft-1', revision: 4, tableName: 'load' },
    ]
    const resultPage: DatasetPage = { datasetId: 'result-1', columns: [], rows: [], nextCursor: null, diagnostics: {} }
    const modelPage: ModelDataFramePage = { resourceId: 'frame-1', columns: [], rows: [], nextCursor: null, diagnostics: {} }
    expect(sources.map(source => source.kind)).toEqual(['result', 'model-version', 'model-draft'])
    expect(resultPage.datasetId).toBe('result-1')
    expect(modelPage.resourceId).toBe('frame-1')
  })
})

describe('chart contracts', () => {
  it('keeps configs and models as six-branch discriminated unions', () => {
    const configKinds: ChartConfig['kind'][] = ['line', 'bar', 'scatter', 'heatmap', 'boxplot', 'histogram']
    const modelKinds: ChartModel['kind'][] = ['line', 'bar', 'scatter', 'heatmap', 'boxplot', 'histogram']
    expect(configKinds).toEqual(modelKinds)
    expect(chartCapabilities().templates.map(template => template.kind)).toEqual(configKinds)
  })

  it('keeps large integer categories tagged as decimal strings', () => {
    const value: CategoryValue = { kind: 'integer', value: '9007199254740993' }
    expect(value.value).toBe('9007199254740993')
  })

  it('uses lightweight saved-view summaries', () => {
    const summary: ChartViewSummary = { id: '9', sessionId: '2', sourceDatasetId: '7', sourceName: 'result',
      sourceDeleted: false, name: 'load', kind: 'line', pointCount: 42, sizeBytes: 1000,
      createdAt: '2026-11-03T00:00:00Z' }
    expect(summary).not.toHaveProperty('config')
  })

  it('calls the seven controlled chart API routes with string resource ids', async () => {
    const calls: { url: string; init: RequestInit }[] = []
    vi.stubGlobal('fetch', vi.fn(async (url: string, init: RequestInit = {}) => {
      calls.push({ url, init })
      return new Response(JSON.stringify({ view: { id: '9' }, replayed: false }), {
        status: init.method === 'POST' && url.endsWith('/views') ? 201 : 200,
        headers: { 'Content-Type': 'application/json' },
      })
    }))
    const config = { version: 1, kind: 'histogram', presentation: {
      legend: { show: true, position: 'auto', orientation: 'auto' }, palette: 'colorblind',
      title: { show: false }, axis: { showGrid: true },
    }, value: { column: 'load' }, bins: 20 } satisfies ChartConfig

    await getChartCapabilities('7')
    await previewChart('7', config)
    await createChartView('7', { clientOperationId: 'operation-1', name: 'load', config })
    await getChartView('9')
    await getChartViewData('9')
    await listChartViews('2', { cursor: 'next', limit: 20 })
    await deleteChartView('9')

    expect(calls.map(call => [call.url, call.init.method ?? 'GET'])).toEqual([
      ['/api/datasets/7/chart-capabilities', 'GET'],
      ['/api/datasets/7/chart-previews', 'POST'],
      ['/api/datasets/7/views', 'POST'],
      ['/api/dataset-views/9', 'GET'],
      ['/api/dataset-views/9/data', 'GET'],
      ['/api/sessions/2/dataset-views?cursor=next&limit=20', 'GET'],
      ['/api/dataset-views/9', 'DELETE'],
    ])
    expect(JSON.parse(String(calls[2].init.body))).toEqual({ clientOperationId: 'operation-1', name: 'load', config })
  })
})
