import { useCallback,useEffect,useRef,useState } from 'react'
import { getDataset,getDatasetRows } from '../api/client'
import type { DatasetMetadata,DatasetPage } from '../types'
export function mergePages(current:DatasetPage|null,next:DatasetPage):DatasetPage{
  if(!current)return next
  return{...next,rows:[...current.rows,...next.rows]}
}
export function useDataset(id:string|null){
  const [metadata,setMetadata]=useState<DatasetMetadata|null>(null),[page,setPage]=useState<DatasetPage|null>(null)
  const [loading,setLoading]=useState(false),[error,setError]=useState<string|null>(null)
  const inFlight=useRef(false)
  const load=useCallback(async(reset=false)=>{if(!id||inFlight.current)return;inFlight.current=true;setLoading(true);setError(null);try{
    const meta=metadata??await getDataset(id);setMetadata(meta)
    const next=await getDatasetRows(id,{limit:100,cursor:reset?undefined:page?.nextCursor??undefined})
    setPage(p=>reset?next:mergePages(p,next))
  }catch(e){setError(e instanceof Error?e.message:'数据集加载失败')}finally{inFlight.current=false;setLoading(false)}},[id,metadata,page?.nextCursor])
  useEffect(()=>{setMetadata(null);setPage(null);if(id)void Promise.all([getDataset(id).then(setMetadata),getDatasetRows(id,{limit:100}).then(setPage)]).catch(e=>setError(e instanceof Error?e.message:'数据集加载失败'))},[id])
  return{metadata,page,loading,error,loadMore:()=>load(false)}
}
