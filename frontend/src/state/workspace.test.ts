import{describe,expect,it}from'vitest';import{workspaceReducer}from'./workspace'
describe('workspace reducer',()=>{it('updates active/draft and snapshot null clears draft',()=>{
  let state=workspaceReducer({},{type:'snapshot',sessionId:'s',workspace:{id:'w',sessionId:'s',active:null,draft:null,activeIngestions:[]}})
  state=workspaceReducer(state,{type:'model-status',sessionId:'s',message:{id:'1',timestamp:'x',type:'model-status',workspaceId:'w',scope:'draft',snapshot:{versionId:'v',status:'ready',missing:[]}}})
  expect(state.s?.draft?.versionId).toBe('v')
  state=workspaceReducer(state,{type:'model-status',sessionId:'s',message:{id:'2',timestamp:'x',type:'model-status',workspaceId:'w',scope:'draft',snapshot:null}})
  expect(state.s?.draft).toBeNull()
})})
