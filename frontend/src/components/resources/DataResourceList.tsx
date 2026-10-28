import { useRef } from 'react'
import type { DataResourceView } from '../../types'

const size=(value:number)=>value<1024?`${value} B`:value<1024*1024?`${(value/1024).toFixed(1)} KiB`:`${(value/1024/1024).toFixed(1)} MiB`
export default function DataResourceList({resources,busyId,onView,onDownload,onDelete,onReplace,onModifyParameters}:{resources:DataResourceView[];busyId:string|null;onView:(resource:DataResourceView)=>void;onDownload:(resource:DataResourceView,kind:'normalized'|'original')=>void;onDelete:(resource:DataResourceView)=>void;onReplace:(resource:DataResourceView,file:File)=>void;onModifyParameters?:(resource:DataResourceView)=>void}){
  const inputs=useRef<Record<string,HTMLInputElement|null>>({})
  return <div className="space-y-2">{resources.map(resource=><article key={resource.id} className="rounded-xl border bg-white p-3 text-xs">
    <div className="flex min-w-0 items-start gap-2"><div className="min-w-0 flex-1"><h3 className="truncate font-semibold" title={resource.displayName}>{resource.displayName}</h3><p className="mt-1 truncate text-slate-500">alias: {resource.alias}</p></div><span className="rounded bg-slate-100 px-2 py-1">{resource.status}</span></div>
    <p className="mt-2 text-slate-500">{resource.rowCount.toLocaleString()} 行 · {Array.isArray(resource.schema)?resource.schema.length:0} 列 · {size(resource.sizeBytes)}</p>
    <p className="mt-1 break-all text-slate-400">来源：{resource.sourceKind}{resource.declaredSchemaRef?` · ${resource.declaredSchemaRef}`:''}</p><p className="mt-1 text-slate-400">创建：{new Date(resource.createdAtMs).toLocaleString()}</p>
    <div className="mt-3 flex flex-wrap gap-2"><button type="button" onClick={()=>onView(resource)} className="rounded border px-2 py-1">查看</button><button type="button" onClick={()=>onDownload(resource,'normalized')} className="rounded border px-2 py-1">下载 Parquet</button>{resource.originalAvailable&&<button type="button" onClick={()=>onDownload(resource,'original')} className="rounded border px-2 py-1">下载原文件</button>}
      <input ref={element=>{inputs.current[resource.id]=element}} type="file" className="hidden" onChange={event=>{const file=event.target.files?.[0];if(file)onReplace(resource,file);event.target.value=''}}/>
      <button type="button" disabled={busyId===resource.id} onClick={()=>inputs.current[resource.id]?.click()} className="rounded border px-2 py-1">重新上传</button>
      {resource.declaredSchemaRef&&resource.rowCount<=1&&<button type="button" onClick={()=>onModifyParameters?.(resource)} className="rounded border px-2 py-1">修改参数</button>}
      <button type="button" disabled={busyId===resource.id} onClick={()=>onDelete(resource)} className="rounded border border-red-200 px-2 py-1 text-red-600">删除</button></div>
  </article>)}</div>
}
