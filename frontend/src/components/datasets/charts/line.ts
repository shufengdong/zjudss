import type { EChartsOption } from 'echarts'
import type { CategoryValue, LineChartModel } from '../../../types'
import { axisName, base, categoryLabel, type ChartCompileContext, seriesColor, valueAxis } from './shared'

export function lineOption(model: LineChartModel, context: ChartCompileContext = {}): EChartsOption {
  const category = model.xAxis.type === 'category'
  const slots = category ? categorySlots(model) : []
  return {
    ...base(model, context),
    xAxis: category ? {
      type: 'category', name: axisName(model.xAxis.name, model.xAxis.unit),
      data: slots.map(slot => slot.label),
    } : valueAxis(model.xAxis.name, model.xAxis.unit, model.presentation.axis.showGrid),
    yAxis: valueAxis(model.yAxis.name, model.yAxis.unit, model.presentation.axis.showGrid),
    series: model.series.map((series, index) => ({
      name: series.name, type: 'line', smooth: false, showSymbol: model.presentation.line.showSymbol,
      lineStyle: { width: model.presentation.line.width, type: model.presentation.line.style,
        color: seriesColor(model.presentation, index) },
      itemStyle: { color: seriesColor(model.presentation, index) },
      data: category
        ? categorySeriesData(series, slots, model.aggregation)
        : series.points.map(point => [point.x as number, point.y]),
    })),
  }
}

const categoryValueKey = (point: LineChartModel['series'][number]['points'][number]) => {
  if (typeof point.x === 'number') return `number:${numberKey(point.x)}`
  return `${point.x.kind}:${point.x.kind === 'number' ? numberKey(point.x.value) : String(point.x.value)}`
}

const categorySeriesEntries = (series: LineChartModel['series'][number], aggregation: LineChartModel['aggregation']) => {
  return series.points.map(point => {
    const valueKey = categoryValueKey(point)
    return { key: aggregation === 'none' ? `${valueKey}:ordinal:${point.ordinal}` : valueKey,
      label: typeof point.x === 'number' ? String(point.x) : categoryLabel(point.x),
      category: typeof point.x === 'number' ? { kind: 'number' as const, value: point.x } : point.x,
      ordinal: point.ordinal, value: point.y }
  })
}

const categorySlots = (model: LineChartModel) => {
  const slots = new Map<string, ReturnType<typeof categorySeriesEntries>[number]>()
  model.series.forEach(series => categorySeriesEntries(series, model.aggregation).forEach(entry => {
    const current = slots.get(entry.key)
    if (!current || entry.ordinal < current.ordinal) slots.set(entry.key, entry)
  }))
  return [...slots.values()].sort((left, right) => {
    if (model.order === 'source') return left.ordinal - right.ordinal
    const ordered = compareCategories(left.category, right.category)
    return (model.order === 'xDesc' ? -ordered : ordered) || left.ordinal - right.ordinal
  })
}

const categorySeriesData = (series: LineChartModel['series'][number], slots: ReturnType<typeof categorySlots>, aggregation: LineChartModel['aggregation']) => {
  const values = new Map(categorySeriesEntries(series, aggregation).map(entry => [entry.key, entry.value]))
  return slots.map(slot => values.get(slot.key) ?? null)
}

const categoryRank: Record<CategoryValue['kind'], number> = { string: 0, boolean: 1, integer: 2, number: 3 }

const compareCategories = (left: CategoryValue, right: CategoryValue): number => {
  if (left.kind !== right.kind) return categoryRank[left.kind] - categoryRank[right.kind]
  if (left.kind === 'string' && right.kind === 'string') return compareUtf8(left.value, right.value)
  if (left.kind === 'boolean' && right.kind === 'boolean') return Number(left.value) - Number(right.value)
  if (left.kind === 'integer' && right.kind === 'integer') return BigInt(left.value) < BigInt(right.value) ? -1 : BigInt(left.value) > BigInt(right.value) ? 1 : 0
  if (left.kind === 'number' && right.kind === 'number') {
    if (Object.is(left.value, right.value)) return 0
    if (Object.is(left.value, -0)) return -1
    if (Object.is(right.value, -0)) return 1
    return left.value - right.value
  }
  return 0
}

const numberKey = (value: number) => Object.is(value, -0) ? '-0' : String(value)

const compareUtf8 = (left: string, right: string): number => {
  const encoder = new TextEncoder()
  const a = encoder.encode(left), b = encoder.encode(right)
  for (let index = 0; index < Math.min(a.length, b.length); index += 1) {
    if (a[index] !== b[index]) return a[index] - b[index]
  }
  return a.length - b.length
}
