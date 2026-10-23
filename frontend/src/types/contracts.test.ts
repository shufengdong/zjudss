import { MESSAGE_TYPES, type MessageType } from './message'
import { VISUALIZATION_KINDS, type VisualizationKind } from './dataset'

describe('frontend public enum contracts', () => {
  it('exports all message types from the union source', () => {
    const values: readonly MessageType[] = MESSAGE_TYPES
    expect(values).toEqual([
      'user', 'assistant', 'tool-call', 'progress',
      'result', 'attachment', 'model-status', 'comparison', 'script',
    ])
  })

  it('exports all visualization kinds from the union source', () => {
    const values: readonly VisualizationKind[] = VISUALIZATION_KINDS
    expect(values).toEqual(['line', 'bar', 'scatter', 'heatmap', 'boxplot', 'histogram'])
  })
})
