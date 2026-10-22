import { useEffect,useRef } from 'react'
import * as echarts from 'echarts'
import type { DatasetViewData, ResultColumn } from '../../types'
import { lineOption } from './charts/line';import { barOption } from './charts/bar';import { scatterOption } from './charts/scatter'
import { heatmapOption } from './charts/heatmap';import { boxplotOption } from './charts/boxplot';import { histogramOption } from './charts/histogram'
export function compileView(data:DatasetViewData,schema:ResultColumn[]){
  const builders={line:lineOption,bar:barOption,scatter:scatterOption,heatmap:heatmapOption,boxplot:boxplotOption,histogram:histogramOption}
  return builders[data.kind](data,schema)
}
export default function ChartRenderer({data,schema}:{data:DatasetViewData;schema:ResultColumn[]}){
  const ref=useRef<HTMLDivElement>(null)
  useEffect(()=>{if(!ref.current)return;const chart=echarts.init(ref.current);chart.setOption(compileView(data,schema));const resize=()=>chart.resize();window.addEventListener('resize',resize);return()=>{window.removeEventListener('resize',resize);chart.dispose()}},[data,schema])
  return <div ref={ref} className="h-[420px] w-full" aria-label={`${data.kind} 图表`}/>
}
