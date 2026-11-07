import { useCallback, useEffect, useState } from 'react'
import { ApiError, getStudyConfiguration } from '../api/client'
import type { StudyConfigurationView } from '../types'

const statusRank = (status: StudyConfigurationView['status']) => ({
  choosing_facade: 0, collecting_inputs: 1, ready_to_confirm: 2,
  confirmed: 3, invalidated: 3, cancelled: 3,
}[status])

export function useStudyConfiguration(initial:StudyConfigurationView){
  const[configuration,setConfiguration]=useState(initial),[busy,setBusy]=useState(false),[error,setError]=useState<string|null>(null)
  useEffect(()=>setConfiguration((current)=>{
    if(initial.id!==current.id||initial.revision>current.revision)return initial
    if(initial.revision<current.revision)return current
    if(initial.status===current.status||statusRank(initial.status)>statusRank(current.status))return initial
    return current
  }),[initial])
  const refresh=useCallback(async()=>{const value=await getStudyConfiguration(String(initial.id));setConfiguration(value);return value},[initial.id])
  const mutate=useCallback(async(action:(current:StudyConfigurationView)=>Promise<StudyConfigurationView>)=>{setBusy(true);setError(null);try{const value=await action(configuration);setConfiguration(value);return value}catch(cause){if(cause instanceof ApiError&&cause.code==='configuration_stale'){await refresh();setError('配置已更新，请确认最新内容后重试')}else setError(cause instanceof Error?cause.message:'配置更新失败');return null}finally{setBusy(false)}},[configuration,refresh])
  return{configuration,busy,error,refresh,mutate}
}
