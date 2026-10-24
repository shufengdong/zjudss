import { useRef, useState } from 'react'
import { getExample } from '../../api/client'
import { useExamples } from '../../hooks/useExamples'
import type { ExampleDetail as Detail } from '../../types'
import ExampleDetail from './ExampleDetail'
import ExampleTree from './ExampleTree'

export default function ExampleLibrary({onUse,onClose}:{onUse:(id:string,title:string,snapshot:string)=>void;onClose:()=>void}){
  const [query,setQuery]=useState('')
  const [selectedId,setSelectedId]=useState<string|null>(null)
  const [detail,setDetail]=useState<Detail|null>(null)
  const [detailLoading,setDetailLoading]=useState(false)
  const [detailError,setDetailError]=useState<string|null>(null)
  const requestNumber=useRef(0)
  const {tree,loading,error,engineResourceSnapshot,retry}=useExamples(query)

  const selectExample=(id:string)=>{
    const request=++requestNumber.current
    setSelectedId(id);setDetail(null);setDetailError(null);setDetailLoading(true)
    void getExample(id).then(value=>{if(request===requestNumber.current)setDetail(value)}).catch(reason=>{
      if(request===requestNumber.current)setDetailError(reason instanceof Error?reason.message:'案例详情加载失败')
    }).finally(()=>{if(request===requestNumber.current)setDetailLoading(false)})
  }
  const back=()=>{
    requestNumber.current+=1;setSelectedId(null);setDetail(null);setDetailError(null);setDetailLoading(false)
  }

  return <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-2 sm:p-4" role="dialog" aria-modal="true" aria-label="内置案例库">
    <div className="flex max-h-[90vh] min-h-[32rem] w-full max-w-5xl flex-col overflow-hidden rounded-2xl bg-white shadow-2xl">
      <header className="flex items-center gap-2 border-b px-4 py-3"><div><h2 className="font-semibold">内置案例库</h2><p className="text-xs text-slate-400">按 eig-rs rssbundle 目录浏览</p></div>
        <button type="button" className="ml-auto rounded px-2 py-1 text-sm text-slate-500 hover:bg-slate-100" onClick={onClose}>关闭</button></header>
      <div className="grid min-h-0 flex-1 sm:grid-cols-[minmax(18rem,2fr)_minmax(20rem,3fr)]">
        <section aria-label="案例目录面板" className={`${selectedId?'hidden sm:flex':'flex'} min-h-0 flex-col border-r`}>
          <div className="border-b p-3"><input aria-label="搜索内置案例" autoFocus value={query} onChange={event=>{setQuery(event.target.value);back()}} placeholder="按名称、目录或分析元数据搜索"
            className="w-full rounded-lg border px-3 py-2 text-sm outline-none focus:border-brand-400"/></div>
          <div className="min-h-0 flex-1 overflow-auto p-3">
            {loading&&<p className="text-sm text-slate-400">加载中…</p>}
            {!loading&&error&&<div role="alert" className="rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-red-700"><b>案例库加载失败</b><p className="mt-1 text-xs">{error}</p><button type="button" aria-label="重试加载案例" onClick={retry} className="mt-3 rounded border border-red-300 px-3 py-1 text-xs">重试</button></div>}
            {!loading&&!error&&tree.length===0&&<p className="py-8 text-center text-sm text-slate-400">没有匹配案例</p>}
            {!loading&&!error&&tree.length>0&&<ExampleTree nodes={tree} selectedId={selectedId} onSelect={selectExample} searchActive={query.trim().length>0}/>}
          </div>
        </section>
        <section aria-label="案例详情面板" className={`${selectedId?'flex':'hidden sm:flex'} min-h-0 flex-col`}>
          {selectedId&&<div className="border-b p-3"><button type="button" aria-label="返回案例目录" onClick={back} className="rounded border px-3 py-1.5 text-xs">返回案例目录</button></div>}
          <div className="min-h-0 flex-1 overflow-auto p-4 sm:p-5">
            {!selectedId&&<div className="flex h-full items-center justify-center text-sm text-slate-400">从左侧目录选择一个案例</div>}
            {detailLoading&&<p className="text-sm text-slate-400">正在加载案例详情…</p>}
            {detailError&&<div role="alert" className="rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-red-700">{detailError}</div>}
            {detail&&<><ExampleDetail example={detail}/>
              {detail.importable&&<button type="button" aria-label={`使用案例“${detail.title}”`} onClick={()=>onUse(detail.id,detail.title,engineResourceSnapshot)} className="mt-5 rounded-lg bg-brand-500 px-4 py-2 text-sm text-white">使用此案例</button>}
            </>}
          </div>
        </section>
      </div>
    </div>
  </div>
}
