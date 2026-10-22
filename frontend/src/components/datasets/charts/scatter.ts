import type { EChartsOption } from 'echarts'
import type { DatasetViewData, ResultColumn } from '../../../types'
import { base,indexOf,unitOf } from './shared'
export function scatterOption(data:DatasetViewData,schema:ResultColumn[]):EChartsOption{
  const x=String(data.spec.x),ys=(data.spec.y as string[])??[];const xi=indexOf(data,x)
  return{...base(data.spec.title),tooltip:{trigger:'item'},xAxis:{type:'value',name:`${x} ${unitOf(schema,x)}`.trim()},yAxis:{type:'value',name:ys.length===1?unitOf(schema,ys[0]):''},
    legend:{data:ys},series:ys.map(name=>({name,type:'scatter',data:data.rows.map(r=>[Number(r[xi]),Number(r[indexOf(data,name)])])}))}
}
