import type { EChartsOption } from 'echarts'
import type { DatasetViewData, ResultColumn } from '../../../types'
import { base,indexOf,unitOf } from './shared'
const quantile=(v:number[],p:number)=>{const i=(v.length-1)*p,l=Math.floor(i),d=i-l;return v[l]+(v[Math.min(l+1,v.length-1)]-v[l])*d}
export function boxplotOption(data:DatasetViewData,schema:ResultColumn[]):EChartsOption{
  const category=String(data.spec.category),value=String(data.spec.value),ci=indexOf(data,category),vi=indexOf(data,value)
  const groups=new Map<string,number[]>();data.rows.forEach(r=>{const key=String(r[ci]),n=Number(r[vi]);if(Number.isFinite(n))groups.set(key,[...(groups.get(key)??[]),n])})
  const values=[...groups.values()].map(v=>{v.sort((a,b)=>a-b);return[v[0],quantile(v,.25),quantile(v,.5),quantile(v,.75),v[v.length-1]]})
  return{...base(data.spec.title),tooltip:{trigger:'item'},xAxis:{type:'category',name:category,data:[...groups.keys()]},yAxis:{type:'value',name:unitOf(schema,value)},series:[{type:'boxplot',data:values}]}
}
