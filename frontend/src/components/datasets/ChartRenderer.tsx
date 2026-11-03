import { useEffect, useRef } from 'react'
import * as echarts from 'echarts'
import type { ChartModel } from '../../types'
import { lineOption } from './charts/line'
import { barOption } from './charts/bar'
import { scatterOption } from './charts/scatter'
import { heatmapOption } from './charts/heatmap'
import { boxplotOption } from './charts/boxplot'
import { histogramOption } from './charts/histogram'
import type { ChartCompileContext } from './charts/shared'

export function compileView(model: ChartModel, context: ChartCompileContext = {}) {
  switch (model.kind) {
    case 'line': return lineOption(model, context)
    case 'bar': return barOption(model, context)
    case 'scatter': return scatterOption(model, context)
    case 'heatmap': return heatmapOption(model, context)
    case 'boxplot': return boxplotOption(model, context)
    case 'histogram': return histogramOption(model, context)
  }
}

export default function ChartRenderer({ model }: { model: ChartModel }) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!ref.current) return
    const chart = echarts.init(ref.current)
    const media = window.matchMedia('(max-width: 639px)')
    const render = () => chart.setOption(compileView(model, { narrow: media.matches }), { notMerge: true })
    const resize = () => { chart.resize(); render() }
    render()
    media.addEventListener('change', render)
    window.addEventListener('resize', resize)
    return () => {
      media.removeEventListener('change', render)
      window.removeEventListener('resize', resize)
      chart.dispose()
    }
  }, [model])
  return <div ref={ref} className="h-[420px] w-full" aria-label={`${model.kind} 图表`}/>
}
