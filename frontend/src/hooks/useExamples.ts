import { useCallback, useEffect, useState } from 'react'
import { listExamples } from '../api/client'
import type { ExampleTreeNode } from '../types'
export function useExamples(query:string){
  const [tree,setTree]=useState<ExampleTreeNode[]>([])
  const [engineResourceSnapshot,setEngineResourceSnapshot]=useState('')
  const [loading,setLoading]=useState(false)
  const [error,setError]=useState<string|null>(null)
  const [retryCounter,setRetryCounter]=useState(0)
  const retry=useCallback(()=>setRetryCounter(value=>value+1),[])
  useEffect(()=>{
    let live=true
    setLoading(true)
    setError(null)
    const timer=setTimeout(()=>void listExamples({query}).then(response=>{
      if(live){setTree(response.tree);setEngineResourceSnapshot(response.engineResourceSnapshot)}
    }).catch(reason=>{
      if(live){setTree([]);setError(reason instanceof Error?reason.message:'案例库加载失败')}
    }).finally(()=>{if(live)setLoading(false)}),200)
    return()=>{live=false;clearTimeout(timer)}
  },[query,retryCounter])
  return{tree,loading,error,engineResourceSnapshot,retry}
}
