import { expect, it } from 'vitest'
import type { ChartConfig } from '../../../types'
import { chartCapabilities } from '../../../test/fixtures'
import { chartFieldOptions } from '../ChartFieldSelect'
import { buildChartConfig, seedChartBuilderState } from './state'

const capabilities = chartCapabilities()
const categories = chartFieldOptions(capabilities, 'category')
const measures = chartFieldOptions(capabilities, 'measure')
const shown = {
  legend: { show: true, position: 'right' as const, orientation: 'vertical' as const },
  palette: 'monochrome' as const,
  title: { show: true },
  axis: { showGrid: false },
}

const configs: ChartConfig[] = [
  {
    version: 1,
    kind: 'line',
    title: 'line',
    presentation: {
      ...shown,
      line: { width: 3, style: 'dashed', showSymbol: true },
    },
    x: { column: 'v_mag', path: [0], label: 'X' },
    xAxisMode: 'value',
    measures: [{ column: 'v_mag', path: [1], label: 'Y' }],
    order: 'xDesc',
    aggregation: 'mean',
    sampling: { mode: 'lttb', maxPointsPerSeries: 321 },
  },
  {
    version: 1,
    kind: 'bar',
    title: 'bar',
    presentation: shown,
    x: { column: 'bus', label: 'Bus' },
    measures: [{ column: 'v_mag', path: [0], label: 'A' }],
    order: 'xAsc',
    aggregation: 'sum',
  },
  {
    version: 1,
    kind: 'scatter',
    presentation: shown,
    x: { column: 'v_mag', path: [0], label: '横轴' },
    measures: [{ column: 'v_mag', path: [0], label: '纵轴' }],
  },
  {
    version: 1,
    kind: 'heatmap',
    presentation: shown,
    x: { column: 'bus' },
    y: { column: 'bus' },
    value: { column: 'v_mag', path: [0], label: '计数' },
    aggregation: 'count',
  },
  {
    version: 1,
    kind: 'boxplot',
    presentation: shown,
    category: { column: 'bus' },
    value: { column: 'v_mag', path: [2], label: 'C' },
    showOutliers: false,
  },
  {
    version: 1,
    kind: 'histogram',
    presentation: shown,
    value: { column: 'v_mag', path: [1], label: 'B' },
    bins: 12,
  },
]

it.each(configs)('round trips seeded $kind configs without losing labels or controlled settings', config => {
  const state = seedChartBuilderState({
    capabilities,
    categories,
    measures,
    sourceName: 'result',
    initialConfig: config,
    initialName: '副本',
  })
  expect(buildChartConfig(state, capabilities, categories, measures)).toEqual(config)
  expect(state.name).toBe('副本')
})

it('coerces invalid aggregations, limits measures, and disables unavailable kinds', () => {
  const limited = chartCapabilities({
    limits: { ...capabilities.limits, maxSeries: 2 },
    templates: capabilities.templates.map(template =>
      template.kind === 'line' ? { ...template, enabled: false, reason: 'no_category_field' } : template,
    ),
  })
  const limitedCategories = chartFieldOptions(limited, 'category')
  const limitedMeasures = chartFieldOptions(limited, 'measure')
  const state = seedChartBuilderState({
    capabilities: limited,
    categories: limitedCategories,
    measures: limitedMeasures,
    sourceName: 'result',
  })
  expect(state.kind).toBe('bar')
  expect(state.measureKeys).toHaveLength(2)

  const heatmap = { ...state, kind: 'heatmap' as const, aggregation: 'none' as const }
  expect(buildChartConfig(heatmap, limited, limitedCategories, limitedMeasures)).toMatchObject({
    kind: 'heatmap',
    aggregation: 'mean',
  })

  const disabled = { ...state, kind: 'line' as const }
  expect(buildChartConfig(disabled, limited, limitedCategories, limitedMeasures)).toBeNull()
})
