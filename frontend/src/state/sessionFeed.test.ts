import{describe,expect,it}from'vitest';import type{Message}from'../types';import{sessionFeedReducer}from'./sessionFeed'
const base={timestamp:'2026-10-22T00:00:00Z'}
describe('session keyed state',()=>{
  it('keeps late events in their own session and merges producer snapshots',()=>{
    let state=sessionFeedReducer({},{type:'event',sessionId:'old',message:{...base,id:'1',type:'assistant',content:'old'}})
    state=sessionFeedReducer(state,{type:'event',sessionId:'new',message:{...base,id:'2',type:'result',runId:'7',title:'a',outputs:[]}})
    state=sessionFeedReducer(state,{type:'event',sessionId:'new',message:{...base,id:'3',type:'result',runId:'7',title:'b',outputs:[]} as Message})
    expect(state.old.messages).toHaveLength(1);expect(state.new.messages).toHaveLength(1);expect(state.new.messages[0].id).toBe('3')
  })

  it('advances the feed revision when an aggregated run progress event is replaced',()=>{
    const running:Message={...base,id:'10',type:'progress',operation:{kind:'run',id:'8'},stages:[{key:'run',label:'执行脚本',status:'running'}]}
    const failed:Message={...base,id:'11',type:'progress',operation:{kind:'run',id:'8'},stages:[{key:'run',label:'执行脚本',status:'failed'}]}
    let state=sessionFeedReducer({},{type:'event',sessionId:'s1',message:running})
    const runningRevision=state.s1.revision
    state=sessionFeedReducer(state,{type:'event',sessionId:'s1',message:failed})

    expect(state.s1.messages).toHaveLength(1)
    expect(state.s1.messages[0]).toMatchObject({id:'11',type:'progress'})
    expect(state.s1.revision).toBeGreaterThan(runningRevision)
  })
})
