import { useEffect, useMemo, useState } from 'react'
import { getDataResource, getDataResourceRows } from '../../api/client'
import type { DataResourceRows, DataResourceView, ResultColumn } from '../../types'
import DataFrameTable from '../dataframes/DataFrameTable'

const columnsFrom=(resource:DataResourceView|null):ResultColumn[]=>Array.isArray(resource?.schema)?resource.schema.map((value,index)=>{const item=(value&&typeof value==='object'?value:{}) as Record<string,unknown>;return{name:String(item.name??`column_${index+1}`),dtype:String(item.dtype??item.dataType??'unknown'),nullable:item.nullable!==false,unit:typeof item.unit==='string'?item.unit:null}}):[]
export default function DataResourceWorkbench({resourceId,onClose}:{resourceId:string;onClose:()=>void}){
  const[resource,setResource]=useState<DataResourceView|null>(null),[page,setPage]=useState<DataResourceRows|null>(null),[loading,setLoading]=useState(true),[error,setError]=useState<string|null>(null)
  useEffect(()=>{const controller=new AbortController();setResource(null);setPage(null);setLoading(true);setError(null)
    void Promise.all([getDataResource(resourceId,controller.signal),getDataResourceRows(resourceId,{limit:100},controller.signal)]).then(([metadata,rows])=>{setResource(metadata);setPage(rows)}).catch(cause=>{if(!controller.signal.aborted)setError(cause instanceof Error?cause.message:'读取失败')}).finally(()=>{if(!controller.signal.aborted)setLoading(false)})
    return()=>controller.abort()
  },[resourceId])
  const loadMore=async()=>{if(!page?.nextCursor||loading)return;setLoading(true);try{const rows=await getDataResourceRows(resourceId,{cursor:page.nextCursor,limit:100});if(rows.resourceId!==resourceId)throw new Error('数据文件分页已失效');setPage(previous=>previous?{...rows,rows:[...previous.rows,...rows.rows]}:rows)}catch(cause){setError(cause instanceof Error?cause.message:'读取失败')}finally{setLoading(false)}}
  const columns=useMemo(()=>columnsFrom(resource),[resource])
  return <div className="fixed inset-0 z-[70] flex items-center justify-center bg-black/30 p-0 sm:p-8"><section role="dialog" aria-modal="true" aria-label="数据文件预览" className="flex h-full w-full max-w-6xl flex-col overflow-hidden bg-white sm:rounded-xl">
    <header className="flex items-center border-b px-4 py-3"><div><h2 className="font-semibold">{resource?.displayName??'数据文件'}</h2><p className="text-xs text-slate-400">{resource?.alias}</p></div><button type="button" aria-label="关闭数据文件预览" onClick={onClose} className="ml-auto p-2">×</button></header>
    {error&&<p role="alert" className="border-b border-red-100 bg-red-50 p-3 text-sm text-red-600">{error}</p>}<div className="min-h-0 flex-1 overflow-auto"><DataFrameTable columns={columns} page={page} loading={loading} onMore={()=>void loadMore()}/>{loading&&!page&&<p className="p-4 text-sm text-slate-400">加载中…</p>}</div>
  </section></div>
}
