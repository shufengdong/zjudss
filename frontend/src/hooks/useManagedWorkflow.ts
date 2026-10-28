import { useCallback, useEffect, useRef, useState } from 'react'
import { cancelManagedWorkflow, getManagedWorkflow } from '../api/client'
import type { ManagedWorkflowView } from '../types'

export function useManagedWorkflow(id:string,revision=0){
  const[workflow,setWorkflow]=useState<ManagedWorkflowView|null>(null),[loading,setLoading]=useState(true),[error,setError]=useState<string|null>(null),generation=useRef(0)
  const reload=useCallback(async()=>{const current=++generation.current;setLoading(true);try{const value=await getManagedWorkflow(id);if(generation.current===current){setWorkflow(value);setError(null)}return value}catch(cause){if(generation.current===current)setError(cause instanceof Error?cause.message:'工作流读取失败');return null}finally{if(generation.current===current)setLoading(false)}},[id])
  useEffect(()=>{void reload();return()=>{generation.current+=1}},[reload,revision])
  const cancel=useCallback(async()=>{const value=await cancelManagedWorkflow(id);setWorkflow(value);return value},[id])
  return{workflow,loading,error,reload,cancel}
}
