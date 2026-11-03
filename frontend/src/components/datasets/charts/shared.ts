import type { EChartsOption, LegendComponentOption } from 'echarts'
import type { CategoryValue, ChartModel, ChartPalette, ChartPresentation } from '../../../types'

export const CHART_PALETTES: Readonly<Record<ChartPalette, readonly string[]>> = Object.freeze({
  default: Object.freeze(['#5470C6','#91CC75','#FAC858','#EE6666','#73C0DE','#3BA272','#FC8452','#9A60B4','#EA7CCC']),
  colorblind: Object.freeze(['#0072B2','#E69F00','#009E73','#D55E00','#CC79A7','#56B4E9','#F0E442','#000000']),
  highContrast: Object.freeze(['#003F5C','#BC5090','#FFA600','#2F4B7C','#FF6361','#58508D']),
  monochrome: Object.freeze(['#0F172A','#334155','#475569','#64748B','#94A3B8','#CBD5E1']),
})

export interface ChartCompileContext { narrow?: boolean }

export const categoryLabel = (value: CategoryValue): string => value.kind === 'boolean'
  ? (value.value ? 'true' : 'false')
  : String(value.value)

export const axisName = (name: string | null, unit: string | null): string =>
  [name, unit ? `(${unit})` : null].filter(Boolean).join(' ')

export const seriesColor = (presentation: ChartPresentation, index: number): string => {
  const colors = CHART_PALETTES[presentation.palette]
  return colors[index % colors.length]
}

export function legendOption(presentation: ChartPresentation, narrow = false): LegendComponentOption {
  const intent = presentation.legend
  if (!intent.show) return { show: false }
  const position = narrow && (intent.position === 'auto' || intent.position === 'right')
    ? 'bottom' : intent.position === 'auto' ? 'top' : intent.position
  const orient = narrow ? 'horizontal' : intent.orientation === 'auto'
    ? (position === 'right' ? 'vertical' : 'horizontal') : intent.orientation
  if (position === 'right') return { show: true, right: 16, top: 'middle', orient }
  if (position === 'bottom') return { show: true, left: 'center', bottom: 0, orient }
  return { show: true, left: 'center', top: 0, orient }
}

export function base(model: ChartModel, context: ChartCompileContext = {}): EChartsOption {
  const title = model.presentation.title.show ? model.title : null
  const titleVisible = title !== null
  const legend = legendOption(model.presentation, Boolean(context.narrow))
  const topLegend = model.presentation.legend.show && legend.top === 0
  if (titleVisible && topLegend) legend.top = 40
  const rightLegend = model.presentation.legend.show && !context.narrow && model.presentation.legend.position === 'right'
  const bottomLegend = model.presentation.legend.show && (context.narrow || model.presentation.legend.position === 'bottom')
  return {
    animation: false,
    aria: { enabled: true },
    color: [...CHART_PALETTES[model.presentation.palette]],
    ...(title !== null ? { title: { show: true, text: title, left: 16, top: 8 } } : {}),
    legend,
    tooltip: { trigger: 'axis' },
    grid: { left: 56, right: rightLegend ? 144 : 24, top: titleVisible ? (topLegend ? 80 : 64) : (topLegend ? 48 : 40),
      bottom: bottomLegend ? 88 : 48, containLabel: true },
  }
}

export const valueAxis = (name: string | null, unit: string | null, showGrid: boolean) => ({
  type: 'value' as const, name: axisName(name, unit), splitLine: { show: showGrid },
})
