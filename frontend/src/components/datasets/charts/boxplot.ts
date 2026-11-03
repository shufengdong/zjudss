import type { EChartsOption } from 'echarts'
import type { BoxplotChartModel } from '../../../types'
import { base, categoryLabel, type ChartCompileContext, valueAxis } from './shared'

export function boxplotOption(model: BoxplotChartModel, context: ChartCompileContext = {}): EChartsOption {
  const boxplots = model.series.map(series => ({
    name: series.name, type: 'boxplot' as const,
    data: series.values.map(value => value ?? '-'),
  }))
  const outliers = model.series.filter(series => series.outliers.length > 0).map(series => ({
    name: `${series.name} 离群点`, type: 'scatter' as const,
    data: series.outliers.map(outlier => [outlier.categoryIndex, outlier.value]),
  }))
  return {
    ...base(model, context), tooltip: { trigger: 'item' },
    xAxis: { type: 'category', name: model.categoryAxis.name, data: model.categoryAxis.categories.map(categoryLabel) },
    yAxis: valueAxis(model.valueAxis.name, model.valueAxis.unit, model.presentation.axis.showGrid),
    // ECharts accepts '-' as an empty item, but its boxplot declaration omits that documented gap token.
    series: [...boxplots, ...outliers] as unknown as EChartsOption['series'],
  }
}
