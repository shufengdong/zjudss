import { useCallback, useEffect, useRef, useState } from 'react'
import { getModelImport, uploadModelFiles, uploadScript } from '../api/client'
import type { ModelImport, WorkspaceDraft } from '../types'

export interface UploadEntry{id:string;name:string;relativePath?:string;status:'queued'|'uploading'|'accepted'|'failed';error?:string}
const stableId=()=>crypto.randomUUID?.()??`${Date.now()}-${Math.random()}`
const modelExtensions=new Set(['dss','dssbundle','rssbundle','zip','csv','xlsx','xls','parquet'])
const extension=(name:string)=>name.toLowerCase().split('.').pop()??''

export function classifyUploads(files:File[]){
  return {
    scripts:files.filter(file=>extension(file.name)==='rss'),
    models:files.filter(file=>modelExtensions.has(extension(file.name))),
    unsupported:files.filter(file=>extension(file.name)!=='rss'&&!modelExtensions.has(extension(file.name))),
  }
}

export function useModelImports(sessionId:string|null,draft:WorkspaceDraft|null|undefined,onChanged:()=>void){
  const[queue,setQueue]=useState<UploadEntry[]>([])
  const[imports,setImports]=useState<ModelImport[]>([])
  const timers=useRef<number[]>([])
  useEffect(()=>()=>timers.current.forEach(window.clearTimeout),[])
  useEffect(()=>{setQueue([]);setImports([])},[sessionId])
  const watch=useCallback((id:string)=>{const poll=async()=>{try{const value=await getModelImport(id);setImports(items=>[value,...items.filter(item=>item.id!==id)])
      if(['accepted','processing'].includes(value.status)){timers.current.push(window.setTimeout(()=>void poll(),250))}
      else onChanged()
    }catch{/* the accepted record remains visible and can be retried after reload */}};void poll()},[onChanged])
  const upload=useCallback(async(files:File[])=>{
    if(!sessionId||files.length===0)return
    const classified=classifyUploads(files),entries=files.map(file=>({id:stableId(),name:file.name,relativePath:(file as File&{webkitRelativePath?:string}).webkitRelativePath||undefined,status:'queued' as const,file}))
    setQueue(entries.map(({file:_,...entry})=>entry))
    for(const entry of entries.filter(item=>classified.unsupported.includes(item.file))){setQueue(items=>items.map(item=>item.id===entry.id?{...item,status:'failed',error:'不支持的文件类型'}:item))}
    for(const entry of entries.filter(item=>classified.scripts.includes(item.file))){setQueue(items=>items.map(item=>item.id===entry.id?{...item,status:'uploading'}:item));try{await uploadScript(sessionId,entry.file);setQueue(items=>items.map(item=>item.id===entry.id?{...item,status:'accepted'}:item))}catch(error){setQueue(items=>items.map(item=>item.id===entry.id?{...item,status:'failed',error:error instanceof Error?error.message:'上传失败'}:item))}}
    const models=entries.filter(item=>classified.models.includes(item.file));if(models.length){setQueue(items=>items.map(item=>models.some(model=>model.id===item.id)?{...item,status:'uploading'}:item));try{const accepted=await uploadModelFiles(sessionId,stableId(),models.map(item=>({id:item.id,file:item.file,relativePath:item.relativePath})),draft);setImports(items=>[accepted.modelImport,...items.filter(item=>item.id!==accepted.modelImport.id)]);setQueue(items=>items.map(item=>models.some(model=>model.id===item.id)?{...item,status:'accepted'}:item));watch(accepted.modelImport.id)}catch(error){setQueue(items=>items.map(item=>models.some(model=>model.id===item.id)?{...item,status:'failed',error:error instanceof Error?error.message:'上传失败'}:item))}}
  },[sessionId,draft?.id,draft?.revision,watch])
  return{queue,imports,upload,setImports,watch}
}
