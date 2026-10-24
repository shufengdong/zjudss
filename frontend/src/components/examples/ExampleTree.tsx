import { useLayoutEffect, useState } from 'react'
import type { ExampleTreeNode as TreeNode } from '../../types'

const directoryKey=(path:string[])=>path.map(part=>part.toLocaleLowerCase()).join('/')
const directoryKeys=(nodes:TreeNode[],all:boolean)=>{
  const keys:string[]=[]
  const visit=(items:TreeNode[],depth:number)=>items.forEach(node=>{
    if(node.type==='directory'){
      if(all||depth===0)keys.push(directoryKey(node.path))
      visit(node.children,depth+1)
    }
  })
  visit(nodes,0)
  return keys
}

export default function ExampleTree({nodes,selectedId,onSelect,searchActive}:{
  nodes:TreeNode[];selectedId:string|null;onSelect:(id:string)=>void;searchActive:boolean
}){
  const [expanded,setExpanded]=useState<Set<string>>(()=>new Set())
  useLayoutEffect(()=>{
    const required=directoryKeys(nodes,searchActive)
    setExpanded(current=>{
      const next=new Set(current)
      required.forEach(key=>next.add(key))
      return next
    })
  },[nodes,searchActive])

  const renderNodes=(items:TreeNode[],depth=0):React.ReactNode=>items.map(node=>{
    if(node.type==='directory'){
      const key=directoryKey(node.path)
      const open=expanded.has(key)
      return <li key={`directory:${key}`} role="treeitem" aria-expanded={open}>
        <button type="button" aria-expanded={open} onClick={()=>setExpanded(current=>{
          const next=new Set(current);if(next.has(key))next.delete(key);else next.add(key);return next
        })} className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm font-medium text-slate-700 hover:bg-slate-100" style={{paddingLeft:`${depth*.9+.5}rem`}}>
          <span aria-hidden="true" className="w-3 text-slate-400">{open?'▾':'▸'}</span>{node.name}
        </button>
        {open&&<ul role="group">{renderNodes(node.children,depth+1)}</ul>}
      </li>
    }
    const item=node.example
    return <li key={`example:${item.id}`} role="treeitem">
      <button type="button" aria-current={selectedId===item.id?'true':undefined} onClick={()=>onSelect(item.id)}
        className={`flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm hover:bg-slate-100 ${selectedId===item.id?'bg-brand-50 text-brand-700':'text-slate-600'}`}
        style={{paddingLeft:`${depth*.9+1.75}rem`}}>
        <span aria-hidden="true" className={item.importable?'text-emerald-500':'text-amber-500'}>●</span>{item.title}
      </button>
    </li>
  })
  return <ul role="tree" aria-label="案例目录" className="space-y-0.5">{renderNodes(nodes)}</ul>
}
