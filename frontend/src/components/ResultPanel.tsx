import{useState}from'react'
import type{MouseEvent as ReactMouseEvent}from'react'
import type{ModelImport,OpenDataFrame,Workspace}from'../types'
import type{RunHistoryState}from'../hooks/useRunHistory'
import WorkspacePanel from'./workspace/WorkspacePanel'
import RunHistory from'./runs/RunHistory'
export type WorkbenchTab='workspace'|'runs'
export default function ResultPanel({open,overlay,tab,onTab,onClose,sessionId,workspace,imports,runHistory,datasetRevision,onReload,notify,onOpenDataset,onOpenDataFrame,onInvalidateDataFrame}:{
  open:boolean;overlay:boolean;tab:WorkbenchTab;onTab:(tab:WorkbenchTab)=>void;onClose:()=>void
  sessionId:string|null;workspace?:Workspace;imports:ModelImport[];runHistory:RunHistoryState;datasetRevision:number;onReload:()=>Promise<Workspace|undefined>;notify:(text:string)=>void;onOpenDataset:(id:string)=>void;onOpenDataFrame:(source:OpenDataFrame)=>void;onInvalidateDataFrame:(target:{kind:'model-draft';draftId:string}|{kind:'model-version';versionId:string})=>void
}){
  const[width,setWidth]=useState(400)
  if(!open)return null
  const startResize=(event:ReactMouseEvent)=>{const start=event.clientX,initial=width
    const move=(e:MouseEvent)=>setWidth(Math.min(720,Math.max(340,initial+start-e.clientX)))
    const stop=()=>{window.removeEventListener('mousemove',move);window.removeEventListener('mouseup',stop)}
    window.addEventListener('mousemove',move);window.addEventListener('mouseup',stop)}
  const panel=<aside style={overlay?undefined:{width}} className={`${overlay?'fixed inset-y-0 right-0 z-40 w-[min(92vw,430px)] shadow-2xl':'relative'} flex shrink-0 flex-col border-l bg-slate-50`}>
    {!overlay&&<button aria-label="调整工作台宽度" onMouseDown={startResize} className="absolute inset-y-0 -left-1 z-10 w-2 cursor-col-resize"/>}
    <header className="flex h-12 items-center border-b bg-white px-2"><button onClick={()=>onTab('workspace')} className={`px-3 py-2 text-sm ${tab==='workspace'?'text-brand-600':''}`}>模型工作区</button>
      <button onClick={()=>onTab('runs')} className={`px-3 py-2 text-sm ${tab==='runs'?'text-brand-600':''}`}>运行记录</button><button aria-label="关闭结果面板" onClick={onClose} className="ml-auto px-3">×</button></header>
    <div className="flex-1 overflow-auto">{tab==='workspace'?<WorkspacePanel sessionId={sessionId} workspace={workspace} imports={imports} onReload={onReload} notify={notify} onOpenDataFrame={onOpenDataFrame} onInvalidateDataFrame={onInvalidateDataFrame}/>:<RunHistory key={sessionId} history={runHistory} datasetRevision={datasetRevision} onOpenDataset={onOpenDataset}/>}</div>
  </aside>
  return <>{overlay&&<button className="fixed inset-0 z-30 bg-black/30" onClick={onClose} aria-label="关闭工作台"/>}{panel}</>
}
