import type { EChartsOption } from 'echarts'
import type { DatasetViewData, ResultColumn } from '../../../types'
export const indexOf=(data:DatasetViewData,name:unknown)=>{
  const index=data.columns.indexOf(String(name??'')); if(index<0)throw new Error(`视图列不存在：${String(name)}`);return index
}
export const unitOf=(schema:ResultColumn[],name:string)=>schema.find(c=>c.name===name)?.unit??''
export const base=(title:unknown):EChartsOption=>({
  animation:false,title:{text:typeof title==='string'?title:''},tooltip:{trigger:'axis'},
  grid:{left:56,right:24,top:52,bottom:48,containLabel:true},
})
