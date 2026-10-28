import { useCallback, useEffect, useRef, useState } from 'react'
import {
  confirmUploadBatch, discardUploadBatch, getUploadBatch, updateUploadCandidates,
  updateUploadParseSettings, uploadBatchFiles, uploadScript,
} from '../api/client'
import type { CandidateDecision, ParseSettings, ReplacementAction, UploadBatchView, UploadCandidateView, WorkspaceDraft } from '../types'

export interface UploadEntry { id:string; name:string; relativePath?:string; status:'queued'|'uploading'|'accepted'|'failed'; error?:string }
const stableId=()=>crypto.randomUUID?.()??`${Date.now()}-${Math.random()}`
const tabularExtensions=new Set(['dss','dssbundle','rssbundle','zip','csv','xlsx','xls','parquet'])
const extension=(name:string)=>name.toLowerCase().split('.').pop()??''
const relativePath=(file:File)=>(file as File&{webkitRelativePath?:string}).webkitRelativePath??''
const terminal=new Set(['confirmed','failed','discarded','expired'])

export function classifyUploads(files:File[]){
  const directoryBatch=files.some(file=>relativePath(file)&&(tabularExtensions.has(extension(file.name))||relativePath(file).split('/').some(part=>part.toLowerCase().endsWith('.dssbundle')||part.toLowerCase().endsWith('.rssbundle'))))
  return{
    scripts:files.filter(file=>extension(file.name)==='rss'&&!(directoryBatch&&relativePath(file))),
    tabular:files.filter(file=>tabularExtensions.has(extension(file.name))||(directoryBatch&&Boolean(relativePath(file)))),
    unsupported:files.filter(file=>extension(file.name)!=='rss'&&!tabularExtensions.has(extension(file.name))&&!(directoryBatch&&relativePath(file))),
  }
}
export const selectableCandidateIds=(candidates:UploadCandidateView[])=>candidates.filter(candidate=>candidate.evidence?.hidden!==true).map(candidate=>candidate.id)

export function useUploadBatches(sessionId:string|null,draft:WorkspaceDraft|null|undefined,onChanged:()=>void,recoveryIds:string[]){
  const[queue,setQueue]=useState<UploadEntry[]>([])
  const[batches,setBatches]=useState<UploadBatchView[]>([])
  const generation=useRef(0),timers=useRef(new Map<string,number>()),operationIds=useRef(new Map<string,string>()),changed=useRef(onChanged)
  changed.current=onChanged
  const recoveryKey=recoveryIds.join('\u0000')
  const merge=useCallback((batch:UploadBatchView)=>setBatches(items=>[batch,...items.filter(item=>item.id!==batch.id)]),[])
  const stop=useCallback((id:string)=>{const timer=timers.current.get(id);if(timer!==undefined)window.clearTimeout(timer);timers.current.delete(id)},[])
  const watch=useCallback((id:string)=>{
    stop(id)
    const current=generation.current
    const poll=async()=>{try{const value=await getUploadBatch(id);if(generation.current!==current||value.sessionId!==sessionId)return;merge(value)
      if(!terminal.has(value.status))timers.current.set(id,window.setTimeout(()=>void poll(),500))
      else changed.current()
    }catch{/* durable message keeps the batch recoverable after refresh */}}
    void poll()
  },[merge,sessionId,stop])
  useEffect(()=>()=>timers.current.forEach(timer=>window.clearTimeout(timer)),[])
  useEffect(()=>{
    generation.current+=1;timers.current.forEach(timer=>window.clearTimeout(timer));timers.current.clear();operationIds.current.clear();setQueue([]);setBatches([])
  },[sessionId])
  useEffect(()=>{
    if(!sessionId)return
    const current=generation.current
    for(const id of Array.from(new Set(recoveryKey?recoveryKey.split('\u0000'):[]))){void getUploadBatch(id).then(value=>{if(generation.current===current&&value.sessionId===sessionId){merge(value);if(!terminal.has(value.status))watch(value.id)}}).catch(()=>undefined)}
  },[sessionId,recoveryKey,merge,watch])
  const upload=useCallback(async(files:File[])=>{
    if(!sessionId||files.length===0)return
    const classified=classifyUploads(files)
    const entries=files.map(file=>({id:stableId(),name:file.name,relativePath:relativePath(file)||undefined,status:'queued' as const,file}))
    setQueue(entries.map(({file:_,...entry})=>entry))
    const mark=(ids:string[],status:UploadEntry['status'],error?:string)=>setQueue(items=>items.map(item=>ids.includes(item.id)?{...item,status,error}:item))
    const unsupported=entries.filter(item=>classified.unsupported.includes(item.file));mark(unsupported.map(item=>item.id),'failed','不支持的文件类型')
    for(const entry of entries.filter(item=>classified.scripts.includes(item.file))){mark([entry.id],'uploading');try{await uploadScript(sessionId,entry.file);mark([entry.id],'accepted')}catch(cause){mark([entry.id],'failed',cause instanceof Error?cause.message:'上传失败')}}
    const tabular=entries.filter(item=>classified.tabular.includes(item.file))
    if(tabular.length){const ids=tabular.map(item=>item.id);mark(ids,'uploading');try{const accepted=await uploadBatchFiles(sessionId,stableId(),tabular.map(item=>({id:item.id,file:item.file,relativePath:item.relativePath})));merge(accepted);mark(ids,'accepted');watch(accepted.id)}catch(cause){mark(ids,'failed',cause instanceof Error?cause.message:'上传失败')}}
  },[sessionId,merge,watch])
  const updateParseSettings=useCallback(async(id:string,settings:ParseSettings)=>{const value=await updateUploadParseSettings(id,settings);merge(value);watch(id);return value},[merge,watch])
  const updateCandidates=useCallback(async(id:string,decisions:CandidateDecision[])=>{const value=await updateUploadCandidates(id,decisions);merge(value);return value},[merge])
  const confirm=useCallback(async(id:string,decisions:CandidateDecision[],replacementAction?:ReplacementAction)=>{const expectedDraft=draft?{id:Number(draft.id),revision:draft.revision}:null,payloadKey=`${id}:${JSON.stringify({decisions,replacementAction,expectedDraft})}`,clientOperationId=operationIds.current.get(payloadKey)??stableId();operationIds.current.set(payloadKey,clientOperationId);const result=await confirmUploadBatch(id,{decisions,replacementAction,expectedDraft,clientOperationId});operationIds.current.delete(payloadKey);watch(id);changed.current();return result},[draft?.id,draft?.revision,watch])
  const discard=useCallback(async(id:string)=>{await discardUploadBatch(id);stop(id);setBatches(items=>items.filter(item=>item.id!==id))},[stop])
  return{queue,batches,upload,addBatch:merge,watch,updateParseSettings,updateCandidates,confirm,discard}
}
