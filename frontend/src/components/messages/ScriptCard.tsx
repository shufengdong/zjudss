import{useEffect,useState}from'react'
import{getScript}from'../../api/client'
import type{ScriptSummary}from'../../types'
export const SCRIPT_PREVIEW_LINES=20
export default function ScriptCard({script,onRun,notify}:{script:ScriptSummary;onRun:(script:ScriptSummary)=>Promise<void>;notify:(text:string)=>void}){
  const[resolved,setResolved]=useState(script),[expanded,setExpanded]=useState(false),[content,setContent]=useState<string|null>(null),[loading,setLoading]=useState(false),[running,setRunning]=useState(false)
  const incomplete=typeof resolved.preview!=='string'||typeof resolved.canRun!=='boolean'||typeof resolved.validationStatus!=='string'
  useEffect(()=>{if(!incomplete)return;let live=true;setLoading(true);void getScript(script.id).then(detail=>{if(live){setResolved(detail);setContent(detail.content)}}).catch(error=>{if(live)notify(error instanceof Error?error.message:'脚本读取失败')}).finally(()=>{if(live)setLoading(false)});return()=>{live=false}},[incomplete,notify,script.id])
  const full=async()=>{if(content!==null)return content;setLoading(true);try{const detail=await getScript(resolved.id);setResolved(detail);setContent(detail.content);return detail.content}finally{setLoading(false)}}
  const expand=async()=>{if(expanded){setExpanded(false);return}try{await full();setExpanded(true)}catch(error){notify(error instanceof Error?error.message:'脚本读取失败')}}
  const copy=async()=>{try{await navigator.clipboard.writeText(await full());notify('脚本全文已复制')}catch{notify('复制失败，请展开后手动复制')}}
  const run=async()=>{setRunning(true);try{await onRun(resolved)}finally{setRunning(false)}}
  const shown=expanded&&content!==null?content:resolved.preview
  const disabled=incomplete||!resolved.canRun||running
  const reason=resolved.disabledReason
  return <article className="mx-auto my-3 max-w-3xl rounded-xl border bg-white p-3" aria-label={`脚本 ${resolved.name}`}><div className="flex items-center"><b className="text-sm">{resolved.name}</b><span className="ml-2 text-xs text-slate-400">{resolved.validationStatus}</span><button onClick={()=>void copy()} className="ml-auto rounded border px-2 py-1 text-xs">复制</button></div><pre className={`${!expanded&&resolved.previewTruncated?'max-h-80 overflow-hidden':''} mt-2 overflow-auto rounded-lg bg-slate-950 p-3 text-xs text-slate-100`}><code>{shown}</code></pre><div className="mt-2 flex items-center gap-2">{resolved.previewTruncated&&<button disabled={loading} onClick={()=>void expand()} className="text-xs text-brand-600">{loading?'加载中…':expanded?'收起':'展开全文'}</button>}<button disabled={disabled} title={disabled?reason??undefined:undefined} onClick={()=>void run()} className="ml-auto rounded bg-brand-500 px-3 py-1.5 text-xs text-white disabled:bg-slate-300">{running?'正在提交…':loading&&incomplete?'加载脚本…':'运行'}</button></div>{disabled&&reason&&<p className="mt-1 text-right text-[11px] text-slate-400">{reason}</p>}</article>
}
