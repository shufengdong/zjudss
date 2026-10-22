import { useCallback, useEffect, useReducer } from 'react'
import { getWorkspace } from '../api/client'
import type { ModelStatusMessage } from '../types'
import { workspaceReducer } from '../state/workspace'
export function useWorkspace(sessionId:string|null){
  const [state,dispatch]=useReducer(workspaceReducer,{})
  const reload=useCallback(async(id:string)=>dispatch({type:'snapshot',sessionId:id,workspace:await getWorkspace(id)}),[])
  useEffect(()=>{if(sessionId)void reload(sessionId)},[sessionId,reload])
  const accept=(message:ModelStatusMessage)=>{if(sessionId)dispatch({type:'model-status',sessionId,message})}
  return{workspace:sessionId?state[sessionId]:undefined,reload,accept}
}
