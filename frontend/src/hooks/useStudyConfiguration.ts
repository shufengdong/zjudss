import { useCallback, useEffect, useState } from 'react'
import { ApiError, getStudyConfiguration } from '../api/client'
import type { StudyConfigurationView } from '../types'

export function useStudyConfiguration(initial:StudyConfigurationView){
  const[configuration,setConfiguration]=useState(initial),[busy,setBusy]=useState(false),[error,setError]=useState<string|null>(null)
  useEffect(()=>{if(initial.revision>=configuration.revision)setConfiguration(initial)},[initial.id,initial.revision]) // eslint-disable-line react-hooks/exhaustive-deps
  const refresh=useCallback(async()=>{const value=await getStudyConfiguration(String(initial.id));setConfiguration(value);return value},[initial.id])
  const mutate=useCallback(async(action:(current:StudyConfigurationView)=>Promise<StudyConfigurationView>)=>{setBusy(true);setError(null);try{const value=await action(configuration);setConfiguration(value);return value}catch(cause){if(cause instanceof ApiError&&cause.code==='configuration_stale'){await refresh();setError('配置已更新，请确认最新内容后重试')}else setError(cause instanceof Error?cause.message:'配置更新失败');return null}finally{setBusy(false)}},[configuration,refresh])
  return{configuration,busy,error,refresh,mutate}
}
