import{useCallback,useEffect,useMemo,useReducer,useRef,useState}from'react'
import type{Message,MessageHandlers,OpenDataFrame,Session,User}from'./types'
import{ApiError,applyExample,createRun,createScriptText,createSession,deleteSession as deleteSessionApi,getCurrentUser,listRunDatasets,listSessions,logout,postMessage,renameSession as renameSessionApi}from'./api/client'
import{useSessionFeed}from'./hooks/useSessionFeed';import{useWorkspace}from'./hooks/useWorkspace';import{useModelImports}from'./hooks/useModelImports'
import{useRunHistory}from'./hooks/useRunHistory';import{useStorageUsage}from'./hooks/useStorageUsage';import{useMediaQuery}from'./hooks/useMediaQuery'
import HistorySidebar from'./components/history/HistorySidebar';import Composer from'./components/Composer';import UploadQueue from'./components/uploads/UploadQueue'
import ResultPanel,{type WorkbenchTab}from'./components/ResultPanel';import DatasetWorkbench from'./components/datasets/DatasetWorkbench'
import ModelDataFrameWorkbench from'./components/model/ModelDataFrameWorkbench'
import ComparisonView from'./components/comparisons/ComparisonView';import ExampleLibrary from'./components/examples/ExampleLibrary';import MessageItem from'./components/messages/MessageItem'
import{PanelLeftIcon,PanelRightIcon}from'./components/icons'
import LoginPage from'./components/auth/LoginPage'
import{AUTHENTICATION_REQUIRED_EVENT,authReducer,initialAuthState,preferenceKey}from'./state/auth'
const localId=()=>`local-user-${Date.now()}-${Math.random()}`
function AuthenticatedApp({user,onLogout}:{user:User;onLogout:()=>void}){
  const[sessions,setSessions]=useState<Session[]>([]),[selectedId,setSelectedId]=useState<string|null>(null)
  const[sessionLoadState,setSessionLoadState]=useState<'loading'|'ready'|'error'>('loading'),[sessionLoadAttempt,setSessionLoadAttempt]=useState(0)
  const[draft,setDraft]=useState(''),[sidebarCollapsed,setSidebarCollapsed]=useState(false),[panelOpen,setPanelOpen]=useState(true)
  const[tab,setTab]=useState<WorkbenchTab>('workspace'),[openDataFrame,setOpenDataFrame]=useState<OpenDataFrame|null>(null),[comparisonId,setComparisonId]=useState<string|null>(null)
  const[examplesOpen,setExamplesOpen]=useState(false),[toast,setToast]=useState<string|null>(null)
  const isNarrow=useMediaQuery('(max-width:1279px)'),timer=useRef<number|undefined>(undefined)
  const{feed,older,add,remove}=useSessionFeed(selectedId),workspaceState=useWorkspace(selectedId)
  const notify=useCallback((text:string)=>{setToast(text);if(timer.current)clearTimeout(timer.current);timer.current=window.setTimeout(()=>setToast(null),2500)},[])
  const reloadWorkspace=useCallback(async()=>selectedId?workspaceState.reload(selectedId):undefined,[selectedId,workspaceState.reload])
  const imports=useModelImports(selectedId,workspaceState.workspace?.draft,reloadWorkspace)
  const runs=useRunHistory(selectedId,feed?.messages.length??0),usage=useStorageUsage(feed?.messages.length??0)
  const select=useCallback((id:string)=>{setSelectedId(id);setOpenDataFrame(null);setComparisonId(null);if(isNarrow)setSidebarCollapsed(true);localStorage.setItem(preferenceKey(user.id),id)},[user.id,isNarrow])
  useEffect(()=>{if(isNarrow){setSidebarCollapsed(true);setPanelOpen(false)}},[isNarrow])
  useEffect(()=>{let live=true;setSessionLoadState('loading');void listSessions().then(async list=>{if(!live)return
    const available=list.length===0?[await createSession()]:list;if(!live)return
    setSessions(available);const remembered=localStorage.getItem(preferenceKey(user.id)),next=available.some(s=>s.id===remembered)?remembered!:available[0].id
    setSelectedId(next);localStorage.setItem(preferenceKey(user.id),next);setSessionLoadState('ready')
  }).catch(e=>{if(live){setSessionLoadState('error');notify(e instanceof Error?e.message:'会话初始化失败')}});return()=>{live=false}},[notify,sessionLoadAttempt,user.id])
  useEffect(()=>{const messages=feed?.messages??[];if(messages.some(message=>message.type==='model-status')&&selectedId)void workspaceState.reload(selectedId)},[feed?.messages,selectedId,workspaceState.reload])
  const newSession=useCallback(async()=>{try{const value=await createSession();setSessions(v=>[value,...v]);select(value.id)}catch(e){notify(e instanceof Error?e.message:'新建失败')}},[notify,select])
  const rename=async(id:string,title:string)=>{try{const value=await renameSessionApi(id,title);setSessions(v=>v.map(s=>s.id===id?value:s))}catch(e){notify(e instanceof Error?e.message:'重命名失败')}}
  const removeSession=async(id:string)=>{try{await deleteSessionApi(id);const rest=sessions.filter(s=>s.id!==id)
    if(rest.length===0){setSessions([]);setSelectedId(null);setSessionLoadState('loading');try{const replacement=await createSession();setSessions([replacement]);select(replacement.id);setSessionLoadState('ready')}catch(e){setSessionLoadState('error');notify(e instanceof Error?e.message:'会话初始化失败')}return}
    setSessions(rest);if(selectedId===id)select(rest[0].id)
  }catch(e){notify(e instanceof Error?e.message:'删除失败')}}
  const sendText=useCallback((text:string)=>{
    const content=text.trim();if(!content||!selectedId)return
    const optimistic:Message={id:localId(),type:'user',timestamp:new Date().toISOString(),content};add(selectedId,optimistic);setDraft('')
    void postMessage(selectedId,content).then(({sessionTitle})=>{
      setSessions(values=>values.map(session=>session.id===selectedId?{...session,title:sessionTitle}:session))
    }).catch(e=>{remove(selectedId,optimistic.id);setDraft(content);notify(e instanceof Error?e.message:'发送失败')})
  },[selectedId,add,remove,notify])
  const runScript=useCallback(async(script:Extract<Message,{type:'script'}>['script'])=>{const active=workspaceState.workspace?.active;if(!active){notify('请先生成或激活模型版本');return}try{await createRun(script.id,active.id,crypto.randomUUID?.()??`${Date.now()}-${Math.random()}`);notify('运行已提交')}catch(error){if(error instanceof ApiError&&error.code==='active_model_changed')await reloadWorkspace();throw error}},[workspaceState.workspace?.active?.id,notify,reloadWorkspace])
  const createScript=useCallback(async(content:string)=>{if(!selectedId)return;await createScriptText(selectedId,{name:`manual-${Date.now()}.rss`,content});notify('脚本已创建')},[selectedId,notify])
  const handlers:MessageHandlers=useMemo(()=>({onOpenDataset:id=>setOpenDataFrame({kind:'result',datasetId:id}),onOpenComparison:setComparisonId,onRunScript:runScript,activeVersionId:workspaceState.workspace?.active?.id??null,notify}),[notify,runScript,workspaceState.workspace?.active?.id])
  const invalidateDataFrame=useCallback((target:{kind:'model-draft';draftId:string}|{kind:'model-version';versionId:string})=>setOpenDataFrame(current=>{if(!current||current.kind==='result')return current;if(target.kind==='model-draft'&&current.kind==='model-draft'&&current.draftId===target.draftId)return null;if(target.kind==='model-version'&&current.kind==='model-version'&&current.versionId===target.versionId)return null;return current}),[])
  const openRun=async(id:string)=>{try{const datasets=await listRunDatasets(id);if(datasets[0])setOpenDataFrame({kind:'result',datasetId:datasets[0].id});else notify('该运行暂无结果数据集')}catch(e){notify(e instanceof Error?e.message:'结果加载失败')}}
  if(sessionLoadState!=='ready')return <main className="flex min-h-screen items-center justify-center bg-slate-50 px-4 text-center text-sm text-slate-600">
    {sessionLoadState==='loading'?<p>正在准备会话…</p>:<div role="alert" className="rounded-xl border border-red-200 bg-white p-6 shadow-sm"><p className="text-red-700">会话初始化失败，请重试</p><div className="mt-4 flex justify-center gap-2"><button type="button" onClick={()=>setSessionLoadAttempt(value=>value+1)} className="rounded-lg bg-brand-500 px-3 py-1.5 text-white">重试</button><button type="button" onClick={onLogout} className="rounded-lg border px-3 py-1.5">退出登录</button></div></div>}
  </main>
  const title=sessions.find(s=>s.id===selectedId)?.title.trim()||'新会话'
  return <div className="flex h-screen w-full overflow-hidden">
    {isNarrow&&!sidebarCollapsed&&<button className="fixed inset-0 z-30 bg-black/30" aria-label="关闭历史侧栏" onClick={()=>setSidebarCollapsed(true)}/>}
    {(!isNarrow||!sidebarCollapsed)&&<div className={isNarrow?'fixed inset-y-0 left-0 z-40 shadow-2xl':'h-full'}><HistorySidebar sessions={sessions} selectedId={selectedId} collapsed={sidebarCollapsed} onSelect={select} onNew={()=>void newSession()} onRename={(id,title)=>void rename(id,title)} onDelete={id=>void removeSession(id)} onToggleCollapse={()=>setSidebarCollapsed(v=>!v)} user={user} onLogout={onLogout}/></div>}
    <main inert={isNarrow&&!sidebarCollapsed?true:undefined} aria-hidden={isNarrow&&!sidebarCollapsed?true:undefined} className="flex min-w-0 flex-1 flex-col bg-slate-50"><header className="flex h-14 items-center gap-2 border-b bg-white px-4">
      <button aria-label={sidebarCollapsed?'展开历史侧栏':'收起历史侧栏'} onClick={()=>setSidebarCollapsed(v=>!v)}><PanelLeftIcon className="h-5 w-5"/></button><h1 className="truncate text-sm font-semibold">{title}</h1>
      {usage?.isWarning&&<span className="ml-2 rounded bg-amber-50 px-2 py-1 text-xs text-amber-700">存储已使用 {Math.round(usage.logicalBytes/usage.byteLimit*100)}%</span>}
      <button onClick={()=>setExamplesOpen(true)} className="ml-auto rounded-lg border px-3 py-1.5 text-xs">案例库</button>
      <button aria-label={panelOpen?'收起结果面板':'展开结果面板'} onClick={()=>setPanelOpen(v=>!v)}><PanelRightIcon className="h-5 w-5"/></button>
    </header>
    <div className="flex-1 overflow-y-auto">{feed?.hasMoreBefore&&<div className="py-2 text-center"><button onClick={()=>void older()} className="rounded border bg-white px-3 py-1 text-xs">加载更早消息</button></div>}
      {!feed?.messages.length?<div className="flex h-full items-center justify-center text-center"><div><div className="text-3xl">⚡</div><p className="mt-2 text-sm text-slate-500">描述要做的仿真，或上传 RSS、文件夹、CSV、Excel、Parquet</p></div></div>:
        <div className="py-4">{feed.messages.map(message=><div key={message.id} data-testid={`message-${message.type}`} data-message-id={message.id}><MessageItem message={message} handlers={handlers}/></div>)}</div>}</div>
    <UploadQueue entries={imports.queue}/><Composer value={draft} onChange={setDraft} onSend={()=>sendText(draft)} uploading={imports.queue.some(e=>e.status==='uploading')} onUploadFiles={files=>void imports.upload(files)} onCreateScript={createScript}/>
    </main>
    <ResultPanel open={panelOpen} overlay={isNarrow} tab={tab} onTab={setTab} onClose={()=>setPanelOpen(false)} sessionId={selectedId} workspace={workspaceState.workspace} imports={imports.imports} runs={runs} onReload={reloadWorkspace} notify={notify} onRun={id=>void openRun(id)} onOpenDataFrame={setOpenDataFrame} onInvalidateDataFrame={invalidateDataFrame}/>
    {openDataFrame&&<div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-0 sm:p-8"><div className="h-full w-full max-w-full overflow-hidden shadow-2xl sm:w-auto sm:rounded-xl">{openDataFrame.kind==='result'?<DatasetWorkbench datasetId={openDataFrame.datasetId} onClose={()=>setOpenDataFrame(null)} onDeleted={()=>{setOpenDataFrame(null);notify('结果已删除')}}/>:<ModelDataFrameWorkbench source={openDataFrame} activeVersionId={workspaceState.workspace?.active?.id} onClose={()=>setOpenDataFrame(null)} onSourceChange={setOpenDataFrame} reloadWorkspace={reloadWorkspace} notify={notify}/>}</div></div>}
    {comparisonId&&<div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4"><div className="w-full max-w-2xl rounded-xl bg-white"><button aria-label="关闭比较" onClick={()=>setComparisonId(null)} className="float-right p-4">×</button><ComparisonView id={comparisonId} onDataset={id=>{setComparisonId(null);setOpenDataFrame({kind:'result',datasetId:id})}}/></div></div>}
    {examplesOpen&&<ExampleLibrary onClose={()=>setExamplesOpen(false)} onUse={(id,name,snapshot)=>{if(!selectedId)return;setExamplesOpen(false);void applyExample(selectedId,id,snapshot,crypto.randomUUID?.()??`${Date.now()}-${Math.random()}`,workspaceState.workspace?.draft).then(value=>{imports.setImports(items=>[value.modelImport,...items]);imports.watch(value.modelImport.id);notify(`案例“${name}”已进入导入确认`) }).catch(error=>notify(error instanceof Error?error.message:'案例导入失败'))}}/>}
    {toast&&<div className="fixed bottom-6 left-1/2 z-[70] -translate-x-1/2 rounded-lg bg-slate-900 px-4 py-2 text-sm text-white">{toast}</div>}
  </div>
}
function App(){
  const[auth,dispatch]=useReducer(authReducer,initialAuthState)
  const channel=useRef<BroadcastChannel|null>(null)
  useEffect(()=>{let live=true
    void getCurrentUser().then(user=>{if(live)dispatch({type:'authenticated',user})}).catch(()=>{if(live)dispatch({type:'anonymous'})})
    const unauthenticated=()=>dispatch({type:'anonymous'})
    window.addEventListener(AUTHENTICATION_REQUIRED_EVENT,unauthenticated)
    if(typeof BroadcastChannel!=='undefined'){
      channel.current=new BroadcastChannel('zjudss-auth')
      channel.current.onmessage=event=>{if(event.data==='logout')unauthenticated()}
    }
    return()=>{live=false;window.removeEventListener(AUTHENTICATION_REQUIRED_EVENT,unauthenticated);channel.current?.close();channel.current=null}
  },[])
  const onLogout=useCallback(()=>{void logout().catch(()=>undefined).finally(()=>{dispatch({type:'anonymous'});channel.current?.postMessage('logout')})},[])
  if(auth.status==='loading')return <main className="flex min-h-screen items-center justify-center bg-slate-50 text-sm text-slate-500">正在连接…</main>
  if(auth.status==='anonymous')return <LoginPage onAuthenticated={user=>dispatch({type:'authenticated',user})}/>
  return <AuthenticatedApp key={`${auth.user.id}:${auth.epoch}`} user={auth.user} onLogout={onLogout}/>
}
export default App
