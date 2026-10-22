import{describe,expect,it}from'vitest';import type{Message}from'../types';import{sessionFeedReducer}from'./sessionFeed'
const base={timestamp:'2026-10-22T00:00:00Z'}
describe('session keyed state',()=>{
  it('keeps late events in their own session and merges producer snapshots',()=>{
    let state=sessionFeedReducer({},{type:'event',sessionId:'old',message:{...base,id:'1',type:'assistant',content:'old'}})
    state=sessionFeedReducer(state,{type:'event',sessionId:'new',message:{...base,id:'2',type:'result',taskId:'7',title:'a',outputs:[]}})
    state=sessionFeedReducer(state,{type:'event',sessionId:'new',message:{...base,id:'3',type:'result',taskId:'7',title:'b',outputs:[]} as Message})
    expect(state.old.messages).toHaveLength(1);expect(state.new.messages).toHaveLength(1);expect(state.new.messages[0].id).toBe('3')
  })
})
