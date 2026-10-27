import { useEffect, useState } from 'react'
import { searchHistory } from '../../api/client'
import type { HistoryHit, Session, SessionDeletionPreview, User } from '../../types'
import Sidebar from '../Sidebar'
import UserMenu from '../auth/UserMenu'
export default function HistorySidebar(props:{
  sessions:Session[];selectedId:string|null;collapsed:boolean;onSelect:(id:string)=>void
  onNew:()=>void;onRename:(id:string,title:string)=>void
  onPreviewDelete:(id:string)=>Promise<SessionDeletionPreview>;onDelete:(id:string,impactToken:string)=>Promise<void>;onToggleCollapse:()=>void
  onLocate:(sessionId:string,messageId:string|null)=>void;user:User;onLogout:()=>void
}){
  const [query,setQuery]=useState('');const [hits,setHits]=useState<HistoryHit[]>([])
  useEffect(()=>{let live=true;if(!query.trim()){setHits([]);return}
    const timer=setTimeout(()=>void searchHistory(query.trim()).then(v=>{if(live)setHits(v)}).catch(()=>{if(live)setHits([])}),250)
    return()=>{live=false;clearTimeout(timer)}
  },[query])
  const search = props.collapsed ? null : <>
    <input aria-label="搜索历史" value={query} onChange={e=>setQuery(e.target.value)} placeholder="搜索历史对话"
      className="w-full rounded-lg border border-slate-200 bg-white px-3 py-2 text-xs shadow-sm outline-none focus:border-brand-400"/>
    {hits.length>0&&<div className="absolute left-3 right-3 top-full z-30 mt-1 max-h-64 overflow-auto rounded-lg border border-slate-200 bg-white p-1 shadow-xl">
      {hits.map((hit,index)=><button key={`${hit.sessionId}:${hit.messageId??index}`} onClick={()=>{props.onLocate(hit.sessionId,hit.messageId??null);setQuery('');setHits([])}}
        aria-label={`打开历史会话“${hit.title.trim() || '新会话'}”`}
        className="block w-full rounded-md px-2 py-2 text-left hover:bg-slate-50">
        <div className="truncate text-xs font-medium text-slate-700">{hit.title}</div>
        <div className="mt-1 line-clamp-2 text-[11px] text-slate-400">{hit.snippet}</div>
      </button>)}
    </div>}
  </>
  return <div className="flex h-full shrink-0">
    <Sidebar {...props} search={search} footer={<UserMenu user={props.user} onLogout={props.onLogout}/>}/>
  </div>
}
