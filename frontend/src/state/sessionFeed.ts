import type { Message, MessagePage } from '../types'
import { mergeMessage } from './messageReducer'
export interface FeedEntry { messages:Message[]; nextBeforeId:string|null; hasMoreBefore:boolean; loading:boolean }
export type FeedState=Record<string,FeedEntry>
export type FeedAction=
  |{type:'loading';sessionId:string}
  |{type:'snapshot';sessionId:string;page:MessagePage<Message>}
  |{type:'older';sessionId:string;page:MessagePage<Message>}
  |{type:'event';sessionId:string;message:Message}
  |{type:'remove';sessionId:string;messageId:string}
export const emptyFeed=():FeedEntry=>({messages:[],nextBeforeId:null,hasMoreBefore:false,loading:false})
export function sessionFeedReducer(state:FeedState,action:FeedAction):FeedState{
  const current=state[action.sessionId]??emptyFeed()
  if(action.type==='loading')return{...state,[action.sessionId]:{...current,loading:true}}
  if(action.type==='remove')return{...state,[action.sessionId]:{...current,messages:current.messages.filter(m=>m.id!==action.messageId)}}
  if(action.type==='event')return{...state,[action.sessionId]:{...current,messages:mergeMessage(current.messages,action.message)}}
  const merged=(action.type==='older'?[...action.page.messages,...current.messages]:[...action.page.messages,...current.messages])
    .reduce<Message[]>(mergeMessage,[])
  return{...state,[action.sessionId]:{messages:merged,nextBeforeId:action.page.nextBeforeId,hasMoreBefore:action.page.hasMoreBefore,loading:false}}
}
