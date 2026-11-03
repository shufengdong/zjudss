import type { EChartsOption } from 'echarts'
import type { BarChartModel } from '../../../types'
import { axisName, base, categoryLabel, type ChartCompileContext, valueAxis } from './shared'

export function barOption(model: BarChartModel, context: ChartCompileContext = {}): EChartsOption {
  return {
    ...base(model, context),
    xAxis: { type: 'category', name: axisName(model.xAxis.name, model.xAxis.unit), data: model.categories.map(categoryLabel) },
    yAxis: valueAxis(model.yAxis.name, model.yAxis.unit, model.presentation.axis.showGrid),
    series: model.series.map(series => ({ name: series.name, type: 'bar', data: series.values })),
  }
}
