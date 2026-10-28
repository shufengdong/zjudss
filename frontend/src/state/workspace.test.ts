import{describe,expect,it}from'vitest';import{workspaceReducer}from'./workspace'
describe('workspace reducer',()=>{it('replaces the whole V3 workspace snapshot',()=>{
  let state=workspaceReducer({},{type:'snapshot',sessionId:'s',workspace:{id:'w',sessionId:'s',active:null,draft:null}})
  state=workspaceReducer(state,{type:'snapshot',sessionId:'s',workspace:{id:'w',sessionId:'s',active:null,draft:{id:'d',workspaceId:'w',baseVersionId:null,revision:2,diagnostics:{},tables:[],createdAt:'x',updatedAt:'x'}}})
  expect(state.s?.draft?.id).toBe('d');expect(state.s?.draft?.revision).toBe(2)
  state=workspaceReducer(state,{type:'snapshot',sessionId:'s',workspace:{id:'w',sessionId:'s',active:null,draft:null}})
  expect(state.s?.draft).toBeNull()
})})
