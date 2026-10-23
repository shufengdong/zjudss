import { useCallback, useEffect, useReducer, useRef } from 'react'
import type { Message } from '../types'
import { listMessages } from '../api/client'
import { subscribeEvents } from '../api/events'
import { sessionFeedReducer } from '../state/sessionFeed'
export function useSessionFeed(sessionId:string|null){
  const [feeds,dispatch]=useReducer(sessionFeedReducer,{})
  const active=useRef(sessionId); active.current=sessionId
  const reload=useCallback(async(id:string)=>{
    dispatch({type:'loading',sessionId:id})
    const page=await listMessages(id,{limit:50})
    if(active.current===id)dispatch({type:'snapshot',sessionId:id,page})
  },[])
  useEffect(()=>{
    if(!sessionId)return
    const id=sessionId
    const sub=subscribeEvents(id,{
      onMessage:(message)=>dispatch({type:'event',sessionId:id,message}),
      onControl:()=>void reload(id),
    })
    const loadSnapshot=()=>{void reload(id).catch(()=>undefined)}
    void sub.opened.then(loadSnapshot,loadSnapshot)
    return sub.close
  },[sessionId,reload])
  const older=useCallback(async()=>{
    if(!sessionId)return
    const entry=feeds[sessionId]
    if(!entry?.hasMoreBefore||!entry.nextBeforeId)return
    const page=await listMessages(sessionId,{beforeId:entry.nextBeforeId,limit:50})
    if(active.current===sessionId)dispatch({type:'older',sessionId,page})
  },[feeds,sessionId])
  const add=(id:string,message:Message)=>dispatch({type:'event',sessionId:id,message})
  const remove=(id:string,messageId:string)=>dispatch({type:'remove',sessionId:id,messageId})
  return{feed:sessionId?feeds[sessionId]:undefined,older,add,remove,reload}
}
