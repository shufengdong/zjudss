import{useCallback,useEffect,useMemo,useRef,useState}from'react'
import type{Message,MessageHandlers,Principal,Session}from'./types'
import{createSession,deleteSession as deleteSessionApi,getPrincipal,listSessions,listTaskDatasets,postMessage,renameSession as renameSessionApi}from'./api/client'
import{useSessionFeed}from'./hooks/useSessionFeed';import{useWorkspace}from'./hooks/useWorkspace';import{useIngestion}from'./hooks/useIngestion'
import{useSimulationHistory}from'./hooks/useSimulationHistory';import{useStorageUsage}from'./hooks/useStorageUsage';import{useMediaQuery}from'./hooks/useMediaQuery'
import HistorySidebar from'./components/history/HistorySidebar';import Composer from'./components/Composer';import UploadQueue from'./components/uploads/UploadQueue'
import ResultPanel,{type WorkbenchTab}from'./components/ResultPanel';import DatasetWorkbench from'./components/datasets/DatasetWorkbench'
import ComparisonView from'./components/tasks/ComparisonView';import ExampleLibrary from'./components/examples/ExampleLibrary';import MessageItem from'./components/messages/MessageItem'
import{PanelLeftIcon,PanelRightIcon}from'./components/icons'
const localId=()=>`local-user-${Date.now()}-${Math.random()}`
function App(){
  const[sessions,setSessions]=useState<Session[]>([]),[selectedId,setSelectedId]=useState<string|null>(null),[principal,setPrincipal]=useState<Principal|null>(null)
  const[draft,setDraft]=useState(''),[sidebarCollapsed,setSidebarCollapsed]=useState(false),[panelOpen,setPanelOpen]=useState(true)
  const[tab,setTab]=useState<WorkbenchTab>('workspace'),[datasetId,setDatasetId]=useState<string|null>(null),[comparisonId,setComparisonId]=useState<string|null>(null)
  const[examplesOpen,setExamplesOpen]=useState(false),[toast,setToast]=useState<string|null>(null)
  const isNarrow=useMediaQuery('(max-width:1279px)'),timer=useRef<number|undefined>(undefined)
  const{feed,older,add,remove}=useSessionFeed(selectedId),workspaceState=useWorkspace(selectedId),ingestion=useIngestion(selectedId)
  const tasks=useSimulationHistory(selectedId,feed?.messages.length??0),usage=useStorageUsage(feed?.messages.length??0)
  const notify=useCallback((text:string)=>{setToast(text);if(timer.current)clearTimeout(timer.current);timer.current=window.setTimeout(()=>setToast(null),2500)},[])
  const select=useCallback((id:string)=>{setSelectedId(id);setDatasetId(null);setComparisonId(null);if(isNarrow)setSidebarCollapsed(true);if(principal)localStorage.setItem(`zjudss:last-session:${principal.scopeKey}`,id)},[principal,isNarrow])
  useEffect(()=>{if(isNarrow){setSidebarCollapsed(true);setPanelOpen(false)}},[isNarrow])
  useEffect(()=>{let live=true;void Promise.all([getPrincipal(),listSessions()]).then(([me,list])=>{if(!live)return;setPrincipal(me);setSessions(list)
    const remembered=localStorage.getItem(`zjudss:last-session:${me.scopeKey}`),next=list.some(s=>s.id===remembered)?remembered:list[0]?.id??null;if(next)setSelectedId(next)
  }).catch(e=>notify(e instanceof Error?e.message:'初始化失败'));return()=>{live=false}},[notify])
  useEffect(()=>{const messages=feed?.messages??[];const last=[...messages].reverse().find((m):m is Extract<Message,{type:'model-status'}>=>m.type==='model-status');if(last){workspaceState.accept(last);if(selectedId)void workspaceState.reload(selectedId)}},[feed?.messages,selectedId])
  const newSession=useCallback(async()=>{try{const value=await createSession();setSessions(v=>[value,...v]);select(value.id)}catch(e){notify(e instanceof Error?e.message:'新建失败')}},[notify,select])
  const rename=async(id:string,title:string)=>{try{const value=await renameSessionApi(id,title);setSessions(v=>v.map(s=>s.id===id?value:s))}catch(e){notify(e instanceof Error?e.message:'重命名失败')}}
  const removeSession=async(id:string)=>{try{await deleteSessionApi(id);const rest=sessions.filter(s=>s.id!==id);setSessions(rest);if(selectedId===id)setSelectedId(rest[0]?.id??null)}catch(e){notify(e instanceof Error?e.message:'删除失败')}}
  const sendText=useCallback((text:string)=>{
    const content=text.trim();if(!content||!selectedId)return
    const optimistic:Message={id:localId(),type:'user',timestamp:new Date().toISOString(),content};add(selectedId,optimistic);setDraft('')
    void postMessage(selectedId,content).catch(e=>{remove(selectedId,optimistic.id);setDraft(content);notify(e instanceof Error?e.message:'发送失败')})
  },[selectedId,add,remove,notify])
  const handlers:MessageHandlers=useMemo(()=>({onOpenDataset:setDatasetId,onOpenComparison:setComparisonId,notify}),[notify])
  const openTask=async(id:string)=>{try{const datasets=await listTaskDatasets(id);if(datasets[0])setDatasetId(datasets[0].id);else notify('该任务暂无结果数据集')}catch(e){notify(e instanceof Error?e.message:'结果加载失败')}}
  const title=sessions.find(s=>s.id===selectedId)?.title??'配电网仿真智能体'
  return <div className="flex h-screen w-full overflow-hidden">
    {isNarrow&&!sidebarCollapsed&&<button className="fixed inset-0 z-30 bg-black/30" aria-label="关闭历史侧栏" onClick={()=>setSidebarCollapsed(true)}/>}
    {(!isNarrow||!sidebarCollapsed)&&<div className={isNarrow?'fixed inset-y-0 left-0 z-40 shadow-2xl':''}><HistorySidebar sessions={sessions} selectedId={selectedId} collapsed={sidebarCollapsed} onSelect={select} onNew={()=>void newSession()} onRename={(id,title)=>void rename(id,title)} onDelete={id=>void removeSession(id)} onToggleCollapse={()=>setSidebarCollapsed(v=>!v)}/></div>}
    <main inert={isNarrow&&!sidebarCollapsed?true:undefined} aria-hidden={isNarrow&&!sidebarCollapsed?true:undefined} className="flex min-w-0 flex-1 flex-col bg-slate-50"><header className="flex h-14 items-center gap-2 border-b bg-white px-4">
      <button aria-label={sidebarCollapsed?'展开历史侧栏':'收起历史侧栏'} onClick={()=>setSidebarCollapsed(v=>!v)}><PanelLeftIcon className="h-5 w-5"/></button><h1 className="truncate text-sm font-semibold">{title}</h1>
      {usage?.isWarning&&<span className="ml-2 rounded bg-amber-50 px-2 py-1 text-xs text-amber-700">存储已使用 {Math.round(usage.logicalBytes/usage.byteLimit*100)}%</span>}
      <button onClick={()=>setExamplesOpen(true)} className="ml-auto rounded-lg border px-3 py-1.5 text-xs">案例库</button>
      <button aria-label={panelOpen?'收起结果面板':'展开结果面板'} onClick={()=>setPanelOpen(v=>!v)}><PanelRightIcon className="h-5 w-5"/></button>
    </header>
    <div className="flex-1 overflow-y-auto">{feed?.hasMoreBefore&&<div className="py-2 text-center"><button onClick={()=>void older()} className="rounded border bg-white px-3 py-1 text-xs">加载更早消息</button></div>}
      {!feed?.messages.length?<div className="flex h-full items-center justify-center text-center"><div><div className="text-3xl">⚡</div><p className="mt-2 text-sm text-slate-500">描述要做的仿真，或上传 RSS、文件夹、CSV、Excel、Parquet</p></div></div>:
        <div className="py-4">{feed.messages.map(message=><div key={message.id} data-testid={`message-${message.type}`} data-message-id={message.id}><MessageItem message={message} handlers={handlers}/></div>)}</div>}</div>
    <UploadQueue entries={ingestion.queue}/><Composer value={draft} onChange={setDraft} onSend={()=>sendText(draft)} uploading={ingestion.queue.some(e=>e.status==='uploading')} onUploadFiles={files=>void ingestion.upload(files)}/>
    </main>
    <ResultPanel open={panelOpen} overlay={isNarrow} tab={tab} onTab={setTab} onClose={()=>setPanelOpen(false)} workspace={workspaceState.workspace} tasks={tasks}
      onSuggest={sendText} onTask={id=>void openTask(id)} onCompare={ids=>sendText(`请比较仿真任务 ${ids[0]} 和 ${ids[1]}，给出差异表和合适的图表`)}/>
    {datasetId&&<div className="fixed inset-0 z-50 flex bg-black/30 p-2 sm:p-8"><div className="ml-auto h-full w-full max-w-full overflow-hidden rounded-xl shadow-2xl sm:w-auto"><DatasetWorkbench datasetId={datasetId} onClose={()=>setDatasetId(null)}/></div></div>}
    {comparisonId&&<div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4"><div className="w-full max-w-2xl rounded-xl bg-white"><button aria-label="关闭比较" onClick={()=>setComparisonId(null)} className="float-right p-4">×</button><ComparisonView id={comparisonId} onDataset={id=>{setComparisonId(null);setDatasetId(id)}}/></div></div>}
    {examplesOpen&&<ExampleLibrary onClose={()=>setExamplesOpen(false)} onUse={(id,name)=>{setExamplesOpen(false);sendText(`请导入内置案例“${name}”（exampleId: ${id}），导入后先不要运行，告诉我模型状态`)}}/>}
    {toast&&<div className="fixed bottom-6 left-1/2 z-[70] -translate-x-1/2 rounded-lg bg-slate-900 px-4 py-2 text-sm text-white">{toast}</div>}
  </div>
}
export default App
