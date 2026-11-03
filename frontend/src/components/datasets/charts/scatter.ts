import type { EChartsOption } from 'echarts'
import type { ScatterChartModel } from '../../../types'
import { base, type ChartCompileContext, valueAxis } from './shared'

export function scatterOption(model: ScatterChartModel, context: ChartCompileContext = {}): EChartsOption {
  return {
    ...base(model, context), tooltip: { trigger: 'item' },
    xAxis: valueAxis(model.xAxis.name, model.xAxis.unit, model.presentation.axis.showGrid),
    yAxis: valueAxis(model.yAxis.name, model.yAxis.unit, model.presentation.axis.showGrid),
    series: model.series.map(series => ({ name: series.name, type: 'scatter', data: series.points.map(point => [point.x, point.y]) })),
  }
}
