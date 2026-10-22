import type { EChartsOption } from 'echarts'
import type { DatasetViewData, ResultColumn } from '../../../types'
import { base,indexOf,unitOf } from './shared'
export function histogramOption(data:DatasetViewData,schema:ResultColumn[]):EChartsOption{
  const value=String(data.spec.value),vi=indexOf(data,value),bins=Number(data.spec.bins??20)
  const numbers=data.rows.map(r=>Number(r[vi])).filter(Number.isFinite),min=Math.min(...numbers),max=Math.max(...numbers)
  const width=max===min?1:(max-min)/bins,counts=Array.from({length:bins},()=>0)
  numbers.forEach(n=>counts[Math.min(bins-1,Math.floor((n-min)/width))]++)
  const edges=counts.map((_,i)=>[min+i*width,min+(i+1)*width] as [number,number])
  return{...base(data.spec.title),xAxis:{type:'category',name:unitOf(schema,value),data:edges.map(([a,b])=>`${a.toPrecision(4)}–${b.toPrecision(4)}`)},
    yAxis:{type:'value',name:'频数'},series:[{type:'bar',data:counts,custom:{binEdges:edges}} as never]}
}
