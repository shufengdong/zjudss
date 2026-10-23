import type { Workspace } from '../types'
export type WorkspaceState=Record<string,Workspace|undefined>
export type WorkspaceAction={type:'snapshot';sessionId:string;workspace:Workspace}
export function workspaceReducer(state:WorkspaceState,action:WorkspaceAction):WorkspaceState{
  return{...state,[action.sessionId]:action.workspace}
}
