import{useState}from'react'
import type{SimulationTask}from'../../types'
export default function TaskHistory({tasks,onOpen,onCompare}:{tasks:SimulationTask[];onOpen:(id:string)=>void;onCompare:(ids:string[])=>void}){
  const[selected,setSelected]=useState<string[]>([])
  return <div className="space-y-2 p-3">{tasks.map(t=><div key={t.id} className="rounded-lg border bg-white p-2 text-xs">
    <div className="flex"><button className="font-medium" onClick={()=>onOpen(t.id)}>{t.studyType||'仿真'} #{t.id}</button><span className="ml-auto">{t.status}</span></div>
    <label className="mt-1 flex gap-1 text-slate-400"><input type="checkbox" checked={selected.includes(t.id)} onChange={e=>setSelected(v=>e.target.checked?[...v,t.id]:v.filter(x=>x!==t.id))}/>加入比较</label>
  </div>)}{selected.length===2&&<button onClick={()=>onCompare(selected)} className="rounded bg-brand-500 px-3 py-1.5 text-xs text-white">在对话中比较</button>}</div>
}
