import { useState } from 'react'
import { useExamples } from '../../hooks/useExamples'
import { getExample } from '../../api/client'
import type { ExampleDetail as Detail } from '../../types'
import ExampleDetail from './ExampleDetail'
export default function ExampleLibrary({onUse,onClose}:{onUse:(id:string,title:string,snapshot:string)=>void;onClose:()=>void}){
  const [query,setQuery]=useState('');const [detail,setDetail]=useState<Detail|null>(null);const {examples,loading,engineResourceSnapshot}=useExamples(query)
  return <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4" role="dialog" aria-label="内置案例库">
    <div className="flex max-h-[80vh] w-full max-w-2xl flex-col rounded-2xl bg-white shadow-2xl">
      <div className="flex items-center gap-2 border-b p-4"><h2 className="font-semibold">内置案例库</h2>
        <button className="ml-auto text-sm text-slate-500" onClick={onClose}>关闭</button></div>
      {detail?<div className="overflow-auto p-4"><ExampleDetail example={detail}/><button aria-label="返回案例列表" onClick={()=>setDetail(null)} className="mt-4 rounded-lg border px-3 py-1.5 text-xs">返回案例列表</button></div>:<><div className="p-4"><input aria-label="搜索内置案例" autoFocus value={query} onChange={e=>setQuery(e.target.value)} placeholder="按名称或分析类型搜索"
        className="w-full rounded-lg border px-3 py-2 text-sm outline-none focus:border-brand-400"/></div>
      <div className="grid flex-1 gap-3 overflow-auto px-4 pb-4 sm:grid-cols-2">
        {loading?<p className="text-sm text-slate-400">加载中…</p>:examples.map(item=><article key={item.id} className="rounded-xl border p-3">
          <div className="font-medium text-slate-800">{item.title}</div><p className="mt-1 text-xs text-slate-500">{item.description}</p>
          <div className="mt-2 text-[11px] text-slate-400">{item.sourceKind} · {item.studyType} · {item.fileCount} 文件</div>
          <button aria-label={`查看案例“${item.title}”详情`} onClick={()=>void getExample(item.id).then(setDetail)}
            className="mt-3 rounded-lg border px-3 py-1.5 text-xs">查看详情</button>
          <button aria-label={`使用案例“${item.title}”`} disabled={!item.importable} onClick={()=>onUse(item.id,item.title,engineResourceSnapshot)}
            className="ml-2 mt-3 rounded-lg bg-brand-500 px-3 py-1.5 text-xs text-white disabled:bg-slate-300">使用此案例</button>
        </article>)}
      </div></>}
    </div>
  </div>
}
