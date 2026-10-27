import { MESSAGE_TYPES, type MessageType } from './message'
import { VISUALIZATION_KINDS, type VisualizationKind } from './dataset'
import type { DatasetPage, ModelDataFramePage, OpenDataFrame } from './index'

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
