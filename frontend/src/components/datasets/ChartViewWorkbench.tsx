import { useEffect,useRef,useState } from 'react'
import { deleteChartView,getChartView } from '../../api/client'
import { useChartView } from '../../hooks/useChartView'
import type { ChartViewMetadata } from '../../types'
import ChartBuilder from './ChartBuilder'
import ChartRenderer from './ChartRenderer'

export default function ChartViewWorkbench({viewId,onClose,onDeleted,onOpenChart,notify}:{viewId:string;onClose:()=>void;onDeleted:(viewId:string)=>void;onOpenChart:(viewId:string)=>void;notify:(text:string)=>void}){
  const[metadata,setMetadata]=useState<ChartViewMetadata|null>(null),[error,setError]=useState<string|null>(null),[copying,setCopying]=useState(false),generation=useRef(0)
  const data=useChartView(viewId)
  useEffect(()=>{const request=++generation.current,controller=new AbortController();setMetadata(null);setError(null);setCopying(false)
    void getChartView(viewId,controller.signal).then(value=>{if(generation.current===request)setMetadata(value)}).catch(value=>{if(generation.current===request&&!controller.signal.aborted)setError(value instanceof Error?value.message:'图表加载失败')});return()=>controller.abort()
  },[viewId])
  const current=metadata?.id===viewId?metadata:null
  const remove=async()=>{if(!current||!window.confirm(`删除图表“${current.name}”？`))return;try{await deleteChartView(viewId);notify('图表已删除');onDeleted(viewId);onClose()}catch(value){notify(value instanceof Error?value.message:'图表删除失败')}}
  return <div className="flex h-full w-full min-w-0 flex-col overflow-hidden bg-white sm:min-w-[560px] sm:rounded-xl"><header className="flex min-h-12 max-w-full shrink-0 flex-wrap items-center gap-2 border-b px-3 py-2 sm:flex-nowrap sm:px-4"><h2 className="min-w-0 flex-1 truncate text-sm font-semibold">{current?.name??'已保存图表'}</h2>{current?.sourceDeleted&&<span className="rounded bg-amber-50 px-2 py-1 text-xs text-amber-700">来源已删除</span>}<button disabled={!current||current.sourceDeleted} onClick={()=>setCopying(true)} className="rounded border px-2 py-1 text-xs disabled:opacity-50">基于此图表新建</button><button aria-label="删除图表" disabled={!current} onClick={()=>void remove()} className="rounded border border-red-200 px-2 py-1 text-xs text-red-600">删除图表</button><button aria-label="关闭图表" onClick={onClose}>×</button></header>
    <div className="min-w-0 flex-1 overflow-auto">{error||data.error?<div role="alert" className="m-4 rounded bg-red-50 p-3 text-sm text-red-600">{error??data.error}</div>:copying&&current?<ChartBuilder datasetId={current.sourceDatasetId} sourceName={current.sourceName} initialConfig={current.config} initialName={`${current.name} 副本`} notify={notify} onSaved={onOpenChart}/>:data.data?<ChartRenderer model={data.data}/>:<p className="p-4 text-sm text-slate-500">正在加载图表…</p>}</div>
  </div>
}
