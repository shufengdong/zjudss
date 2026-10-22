import type { ModelStatusMessage, Workspace } from '../types'
export type WorkspaceState=Record<string,Workspace|undefined>
export type WorkspaceAction={type:'snapshot';sessionId:string;workspace:Workspace}|{type:'model-status';sessionId:string;message:ModelStatusMessage}
export function workspaceReducer(state:WorkspaceState,action:WorkspaceAction):WorkspaceState{
  if(action.type==='snapshot')return{...state,[action.sessionId]:action.workspace}
  const current=state[action.sessionId]
  if(!current||current.id!==action.message.workspaceId)return state
  return{...state,[action.sessionId]:{...current,[action.message.scope]:action.message.snapshot}}
}
