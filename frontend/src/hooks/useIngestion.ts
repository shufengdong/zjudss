import { useState } from 'react'
import { getIngestion, uploadFiles } from '../api/client'
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
      const accepted=await uploadFiles(sessionId,batchId,entries.map(e=>({id:e.id,file:e.file,relativePath:e.relativePath})))
      setQueue(q=>q.map(e=>({...e,status:'accepted'})))
      const follow=async()=>{const value=await getIngestion(accepted.ingestionId)
        if(value.status==='failed'){setQueue(q=>q.map(e=>({...e,status:'failed',error:value.errorMessage??'处理失败'})));return}
        if(value.status==='succeeded'||value.status==='completed_with_errors')return
        window.setTimeout(()=>void follow(),200)}
      void follow()
    }catch(error){
      setQueue(q=>q.map(e=>({...e,status:'failed',error:error instanceof Error?error.message:'上传失败'})))
    }
  }
  return{queue,upload}
}
