import type { EChartsOption } from 'echarts'
import type { HeatmapChartModel } from '../../../types'
import { base, categoryLabel, CHART_PALETTES, type ChartCompileContext } from './shared'

export function heatmapOption(model: HeatmapChartModel, context: ChartCompileContext = {}): EChartsOption {
  return {
    ...base(model, context), tooltip: { trigger: 'item' },
    xAxis: { type: 'category', name: model.xAxis.name, data: model.xCategories.map(categoryLabel) },
    yAxis: { type: 'category', name: model.yAxis.name, data: model.yCategories.map(categoryLabel) },
    visualMap: { min: model.value.min, max: model.value.max, calculable: true, orient: 'horizontal', left: 'center', bottom: 24,
      inRange: { color: [...CHART_PALETTES[model.presentation.palette]] } },
    series: [{ name: model.value.name, type: 'heatmap', data: model.cells.map(cell => [cell.xIndex, cell.yIndex, cell.value]) }],
  }
}
