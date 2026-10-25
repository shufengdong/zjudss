import { useCallback, useEffect, useReducer } from 'react'
import { getWorkspace } from '../api/client'
import { workspaceReducer } from '../state/workspace'
export function useWorkspace(sessionId:string|null){
  const [state,dispatch]=useReducer(workspaceReducer,{})
  const reload=useCallback(async(id:string)=>{const workspace=await getWorkspace(id);dispatch({type:'snapshot',sessionId:id,workspace});return workspace},[])
  useEffect(()=>{if(sessionId)void reload(sessionId)},[sessionId,reload])
  return{workspace:sessionId?state[sessionId]:undefined,reload}
}
