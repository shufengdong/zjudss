import type {
  ChartAggregation,
  ChartCapabilities,
  ChartConfig,
  ChartFieldRef,
  ChartKind,
  ChartOrder,
  ChartPalette,
  ChartPresentation,
  LegendOrientation,
  LegendPosition,
} from '../../../types'
import type { ChartFieldOption } from '../ChartFieldSelect'
import { fieldKey, resolveField } from './fieldMapping'

type BuilderAggregation = ChartAggregation | 'count'

export interface ChartBuilderState {
  kind: ChartKind
  name: string
  title: string
  titleShow: boolean
  xKey: string
  numericXKey: string
  measureKeys: string[]
  secondCategoryKey: string
  xAxisMode: 'category' | 'value'
  sampling: boolean
  samplePoints: number
  order: ChartOrder
  aggregation: BuilderAggregation
  palette: ChartPalette
  legend: boolean
  legendPosition: LegendPosition
  legendOrientation: LegendOrientation
  grid: boolean
  lineWidth: 1 | 2 | 3 | 4
  lineStyle: 'solid' | 'dashed' | 'dotted'
  showSymbol: boolean
  bins: number
  showOutliers: boolean
  measureLabels: Record<string, string>
  seedConfig: ChartConfig | null
}

interface SeedChartBuilderStateArgs {
  capabilities: ChartCapabilities
  categories: ChartFieldOption[]
  measures: ChartFieldOption[]
  sourceName: string
  initialConfig?: ChartConfig
  initialName?: string
}

export const chartMeasureLimit = (capabilities: ChartCapabilities): number =>
  Math.min(16, capabilities.limits.maxSeries)

const configMeasures = (config: ChartConfig): ChartFieldRef[] => {
  if (config.kind === 'line' || config.kind === 'bar' || config.kind === 'scatter') {
    return config.measures
  }
  return [config.value]
}

const selectableKey = (field: ChartFieldRef, options: ChartFieldOption[]): string => {
  const key = fieldKey(field, options)
  return options.find(option => option.key === key && !option.disabled)?.key
    ?? options.find(option => !option.disabled)?.key
    ?? ''
}

const cloneConfig = (config: ChartConfig): ChartConfig =>
  JSON.parse(JSON.stringify(config)) as ChartConfig

export function seedChartBuilderState({
  capabilities,
  categories,
  measures,
  sourceName,
  initialConfig,
  initialName,
}: SeedChartBuilderStateArgs): ChartBuilderState {
  const categoryKey = categories.find(option => !option.disabled)?.key ?? ''
  const enabledMeasures = measures.filter(option => !option.disabled)
  const firstMeasure = enabledMeasures[0]
  const unit = firstMeasure
    ? capabilities.fields.find(field => field.column === firstMeasure.field.column)?.unit
    : null
  const preferredMeasures = firstMeasure
    ? enabledMeasures.filter(option => option.field.column === firstMeasure.field.column
      || capabilities.fields.find(field => field.column === option.field.column)?.unit === unit)
    : []
  const enabledKind = capabilities.templates.find(template => template.enabled)?.kind ?? 'line'
  const defaultKind = capabilities.templates.find(template => template.kind === 'line')?.enabled
    ? 'line'
    : enabledKind
  const state: ChartBuilderState = {
    kind: defaultKind,
    name: initialName ?? `${sourceName} 图表`,
    title: '',
    titleShow: false,
    xKey: categoryKey || firstMeasure?.key || '',
    numericXKey: firstMeasure?.key ?? '',
    measureKeys: preferredMeasures
      .slice(0, chartMeasureLimit(capabilities))
      .map(option => option.key),
    secondCategoryKey: categoryKey,
    xAxisMode: categoryKey ? 'category' : 'value',
    sampling: false,
    samplePoints: 1000,
    order: 'source',
    aggregation: 'none',
    palette: 'colorblind',
    legend: true,
    legendPosition: 'auto',
    legendOrientation: 'auto',
    grid: true,
    lineWidth: 2,
    lineStyle: 'solid',
    showSymbol: false,
    bins: 20,
    showOutliers: true,
    measureLabels: {},
    seedConfig: null,
  }
  if (!initialConfig) return state

  const seedConfig = cloneConfig(initialConfig)
  state.seedConfig = seedConfig
  state.kind = seedConfig.kind
  state.name = initialName ?? `${sourceName} 图表副本`
  state.title = seedConfig.title ?? ''
  state.titleShow = seedConfig.presentation.title.show
  state.palette = seedConfig.presentation.palette
  state.legend = seedConfig.presentation.legend.show
  state.legendPosition = seedConfig.presentation.legend.position
  state.legendOrientation = seedConfig.presentation.legend.orientation
  state.grid = seedConfig.presentation.axis.showGrid

  if (seedConfig.kind === 'line') {
    state.xAxisMode = seedConfig.xAxisMode
    state.xKey = selectableKey(
      seedConfig.x,
      seedConfig.xAxisMode === 'value' ? measures : categories,
    )
    state.measureKeys = seedConfig.measures
      .map(field => selectableKey(field, measures))
      .filter(Boolean)
      .slice(0, chartMeasureLimit(capabilities))
    state.order = seedConfig.order
    state.aggregation = seedConfig.aggregation
    state.sampling = seedConfig.sampling.mode === 'lttb'
    if (seedConfig.sampling.mode === 'lttb') {
      state.samplePoints = seedConfig.sampling.maxPointsPerSeries
    }
    state.lineWidth = seedConfig.presentation.line.width
    state.lineStyle = seedConfig.presentation.line.style
    state.showSymbol = seedConfig.presentation.line.showSymbol
  } else if (seedConfig.kind === 'bar') {
    state.xKey = selectableKey(seedConfig.x, categories)
    state.measureKeys = seedConfig.measures
      .map(field => selectableKey(field, measures))
      .filter(Boolean)
      .slice(0, chartMeasureLimit(capabilities))
    state.order = seedConfig.order
    state.aggregation = seedConfig.aggregation
  } else if (seedConfig.kind === 'scatter') {
    state.numericXKey = selectableKey(seedConfig.x, measures)
    state.measureKeys = seedConfig.measures
      .map(field => selectableKey(field, measures))
      .filter(Boolean)
      .slice(0, chartMeasureLimit(capabilities))
  } else if (seedConfig.kind === 'heatmap') {
    state.xKey = selectableKey(seedConfig.x, categories)
    state.secondCategoryKey = selectableKey(seedConfig.y, categories)
    state.measureKeys = [selectableKey(seedConfig.value, measures)].filter(Boolean)
    state.aggregation = seedConfig.aggregation
  } else if (seedConfig.kind === 'boxplot') {
    state.xKey = selectableKey(seedConfig.category, categories)
    state.measureKeys = [selectableKey(seedConfig.value, measures)].filter(Boolean)
    state.showOutliers = seedConfig.showOutliers
  } else {
    state.measureKeys = [selectableKey(seedConfig.value, measures)].filter(Boolean)
    state.bins = seedConfig.bins
  }
  state.measureLabels = Object.fromEntries(
    configMeasures(seedConfig)
      .map(field => [selectableKey(field, measures), field.label ?? ''])
      .filter(([key]) => Boolean(key)),
  )
  return state
}

const presentation = (state: ChartBuilderState): ChartPresentation => ({
  legend: {
    show: state.legend,
    position: state.legendPosition,
    orientation: state.legendOrientation,
  },
  palette: state.palette,
  title: { show: state.titleShow },
  axis: { showGrid: state.grid },
})

export function buildChartConfig(
  state: ChartBuilderState,
  capabilities: ChartCapabilities,
  categories: ChartFieldOption[],
  measures: ChartFieldOption[],
): ChartConfig | null {
  const measureKeys = state.measureKeys.slice(0, chartMeasureLimit(capabilities))
  if (measureKeys.length === 0) return null
  if (!capabilities.templates.find(template => template.kind === state.kind)?.enabled) return null
  if (state.kind === 'scatter' && !state.numericXKey) return null
  if (state.kind !== 'scatter' && state.kind !== 'histogram' && !state.xKey) return null

  const base = presentation(state)
  const title = state.title || undefined
  const seed = state.seedConfig?.kind === state.kind ? state.seedConfig : null
  const preservedMeasures = seed ? configMeasures(seed) : []
  const measure = (key: string): ChartFieldRef => {
    const field = resolveField(key, measures, preservedMeasures)
    if (!Object.prototype.hasOwnProperty.call(state.measureLabels, key)) return field
    const label = state.measureLabels[key].trim()
    return label
      ? { ...field, label }
      : { column: field.column, ...(field.path ? { path: field.path } : {}) }
  }
  const splitBy = seed && 'splitBy' in seed && seed.splitBy
    ? { splitBy: seed.splitBy }
    : {}
  const scalarAggregation: ChartAggregation = state.aggregation === 'count'
    ? 'mean'
    : state.aggregation

  if (state.kind === 'line') {
    const lineSeed = seed?.kind === 'line' ? seed : null
    return {
      version: 1,
      kind: 'line',
      title,
      presentation: {
        ...base,
        line: {
          width: state.lineWidth,
          style: state.lineStyle,
          showSymbol: state.showSymbol,
        },
      },
      x: state.xAxisMode === 'value'
        ? resolveField(state.xKey, measures, lineSeed ? [lineSeed.x] : [])
        : resolveField(state.xKey, categories, lineSeed ? [lineSeed.x] : []),
      xAxisMode: state.xAxisMode,
      measures: measureKeys.map(measure),
      ...splitBy,
      order: state.order,
      aggregation: scalarAggregation,
      sampling: state.sampling && state.xAxisMode === 'value'
        ? { mode: 'lttb', maxPointsPerSeries: state.samplePoints }
        : { mode: 'none' },
    }
  }
  if (state.kind === 'bar') {
    const barSeed = seed?.kind === 'bar' ? seed : null
    return {
      version: 1,
      kind: 'bar',
      title,
      presentation: base,
      x: resolveField(state.xKey, categories, barSeed ? [barSeed.x] : []),
      measures: measureKeys.map(measure),
      ...splitBy,
      order: state.order,
      aggregation: scalarAggregation,
    }
  }
  if (state.kind === 'scatter') {
    const scatterSeed = seed?.kind === 'scatter' ? seed : null
    return {
      version: 1,
      kind: 'scatter',
      title,
      presentation: base,
      x: resolveField(state.numericXKey, measures, scatterSeed ? [scatterSeed.x] : []),
      measures: measureKeys.map(measure),
      ...splitBy,
    }
  }
  if (state.kind === 'heatmap') {
    const heatmapSeed = seed?.kind === 'heatmap' ? seed : null
    return {
      version: 1,
      kind: 'heatmap',
      title,
      presentation: base,
      x: resolveField(state.xKey, categories, heatmapSeed ? [heatmapSeed.x] : []),
      y: resolveField(
        state.secondCategoryKey || state.xKey,
        categories,
        heatmapSeed ? [heatmapSeed.y] : [],
      ),
      value: measure(measureKeys[0]),
      aggregation: state.aggregation === 'none' ? 'mean' : state.aggregation,
    }
  }
  if (state.kind === 'boxplot') {
    const boxplotSeed = seed?.kind === 'boxplot' ? seed : null
    return {
      version: 1,
      kind: 'boxplot',
      title,
      presentation: base,
      category: resolveField(
        state.xKey,
        categories,
        boxplotSeed ? [boxplotSeed.category] : [],
      ),
      value: measure(measureKeys[0]),
      ...splitBy,
      showOutliers: state.showOutliers,
    }
  }
  return {
    version: 1,
    kind: 'histogram',
    title,
    presentation: base,
    value: measure(measureKeys[0]),
    bins: state.bins,
  }
}

export const displayedMeasureKeys = (state: ChartBuilderState): string[] =>
  state.kind === 'line' || state.kind === 'bar' || state.kind === 'scatter'
    ? state.measureKeys
    : state.measureKeys.slice(0, 1)
