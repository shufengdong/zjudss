import type { EChartsOption } from 'echarts'
import type { DatasetViewData, ResultColumn } from '../../../types'
import { base,indexOf,unitOf } from './shared'
export function heatmapOption(data:DatasetViewData,schema:ResultColumn[]):EChartsOption{
  const x=String(data.spec.x),category=String(data.spec.category),value=String(data.spec.value)
  const xi=indexOf(data,x),ci=indexOf(data,category),vi=indexOf(data,value)
  const xs=[...new Set(data.rows.map(r=>String(r[xi])))],categories=[...new Set(data.rows.map(r=>String(r[ci])))]
  const values=data.rows.map(r=>[xs.indexOf(String(r[xi])),categories.indexOf(String(r[ci])),r[vi] as number])
  const numeric=values.map(v=>Number(v[2])).filter(Number.isFinite)
  return{...base(data.spec.title),tooltip:{trigger:'item'},xAxis:{type:'category',name:x,data:xs},yAxis:{type:'category',name:category,data:categories},
    visualMap:{min:Math.min(...numeric),max:Math.max(...numeric),calculable:true,orient:'horizontal',left:'center',bottom:0,text:[unitOf(schema,value),'']},
    series:[{type:'heatmap',data:values}]}
}
