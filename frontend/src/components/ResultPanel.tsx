import type{SimulationTask,Workspace}from'../types'
import WorkspacePanel from'./workspace/WorkspacePanel'
import TaskHistory from'./tasks/TaskHistory'
export type WorkbenchTab='workspace'|'tasks'
export default function ResultPanel({open,overlay,tab,onTab,onClose,workspace,tasks,onSuggest,onTask,onCompare}:{
  open:boolean;overlay:boolean;tab:WorkbenchTab;onTab:(tab:WorkbenchTab)=>void;onClose:()=>void
  workspace?:Workspace;tasks:SimulationTask[];onSuggest:(text:string)=>void;onTask:(id:string)=>void;onCompare:(ids:string[])=>void
}){
  if(!open)return null
  const panel=<aside className={`${overlay?'fixed inset-y-0 right-0 z-40 w-[min(92vw,430px)] shadow-2xl':'w-[400px]'} flex shrink-0 flex-col border-l bg-slate-50`}>
    <header className="flex h-12 items-center border-b bg-white px-2"><button onClick={()=>onTab('workspace')} className={`px-3 py-2 text-sm ${tab==='workspace'?'text-brand-600':''}`}>模型工作区</button>
      <button onClick={()=>onTab('tasks')} className={`px-3 py-2 text-sm ${tab==='tasks'?'text-brand-600':''}`}>仿真历史</button><button onClick={onClose} className="ml-auto px-3">×</button></header>
    <div className="flex-1 overflow-auto">{tab==='workspace'?<WorkspacePanel workspace={workspace} onSuggest={onSuggest}/>:<TaskHistory tasks={tasks} onOpen={onTask} onCompare={onCompare}/>}</div>
  </aside>
  return <>{overlay&&<button className="fixed inset-0 z-30 bg-black/30" onClick={onClose} aria-label="关闭工作台"/>}{panel}</>
}
