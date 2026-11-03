import type { EChartsOption } from 'echarts'
import type { HistogramChartModel } from '../../../types'
import { axisName, base, type ChartCompileContext, valueAxis } from './shared'

export function histogramOption(model: HistogramChartModel, context: ChartCompileContext = {}): EChartsOption {
  return {
    ...base(model, context),
    xAxis: { type: 'category', name: axisName(model.value.name, model.value.unit),
      data: model.bins.map(bin => `[${String(bin.start)}, ${String(bin.end)}${bin.includesEnd ? ']' : ')'}`) },
    yAxis: valueAxis('频数', null, model.presentation.axis.showGrid),
    series: [{ name: model.value.name, type: 'bar', data: model.bins.map(bin => bin.count) }],
  }
}
