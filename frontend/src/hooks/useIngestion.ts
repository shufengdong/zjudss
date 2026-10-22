import { useState } from 'react'
import { uploadFiles } from '../api/client'
export interface UploadEntry{id:string;name:string;relativePath?:string;status:'queued'|'uploading'|'accepted'|'failed';error?:string}
const stableId=()=>crypto.randomUUID?.()??`${Date.now()}-${Math.random()}`
export function useIngestion(sessionId:string|null){
  const [queue,setQueue]=useState<UploadEntry[]>([])
  const upload=async(files:File[])=>{
    if(!sessionId||files.length===0)return
    const batchId=stableId()
    const entries=files.map(file=>({id:stableId(),name:file.name,relativePath:(file as File&{webkitRelativePath?:string}).webkitRelativePath||undefined,status:'queued' as const,file}))
    setQueue(entries.map(({file:_,...entry})=>entry))
    setQueue(q=>q.map(e=>({...e,status:'uploading'})))
    try{
      await uploadFiles(sessionId,batchId,entries.map(e=>({id:e.id,file:e.file,relativePath:e.relativePath})))
      setQueue(q=>q.map(e=>({...e,status:'accepted'})))
    }catch(error){
      setQueue(q=>q.map(e=>({...e,status:'failed',error:error instanceof Error?error.message:'上传失败'})))
    }
  }
  return{queue,upload}
}
