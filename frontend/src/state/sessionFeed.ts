import type { Message, MessagePage } from '../types'
import { mergeMessage } from './messageReducer'

export interface FeedEntry {
  messages: Message[]
  nextBeforeId: string | null
  hasMoreBefore: boolean
  loading: boolean
  revision: number
  mode: 'latest' | 'anchored'
  anchorMessageId: string | null
  latestMessages: Message[]
  latestNextBeforeId: string | null
  latestHasMoreBefore: boolean
  hasNewerMessages: boolean
}
export type FeedState = Record<string, FeedEntry>
export type FeedAction =
  | { type: 'loading'; sessionId: string }
  | { type: 'snapshot'; sessionId: string; page: MessagePage<Message> }
  | { type: 'older'; sessionId: string; page: MessagePage<Message> }
  | { type: 'anchored'; sessionId: string; targetId: string; page: MessagePage<Message> }
  | { type: 'return-latest'; sessionId: string }
  | { type: 'event'; sessionId: string; message: Message }
  | { type: 'remove'; sessionId: string; messageId: string }

export const emptyFeed = (): FeedEntry => ({
  messages: [], nextBeforeId: null, hasMoreBefore: false, loading: false, revision: 0,
  mode: 'latest', anchorMessageId: null, latestMessages: [], latestNextBeforeId: null,
  latestHasMoreBefore: false, hasNewerMessages: false,
})

const mergePage = (left:Message[], right:Message[]) => [...left, ...right].reduce<Message[]>(mergeMessage, [])

export function sessionFeedReducer(state:FeedState, action:FeedAction):FeedState {
  const current = state[action.sessionId] ?? emptyFeed()
  if (action.type === 'loading') return { ...state, [action.sessionId]: { ...current, loading:true } }
  if (action.type === 'remove') {
    const remove = (messages:Message[]) => messages.filter(message => message.id !== action.messageId)
    return { ...state, [action.sessionId]: { ...current, messages:remove(current.messages), latestMessages:remove(current.latestMessages) } }
  }
  if (action.type === 'event') {
    const latestMessages = mergeMessage(current.latestMessages, action.message)
    if (current.mode === 'anchored') {
      return { ...state, [action.sessionId]: { ...current, latestMessages, hasNewerMessages:true, revision:current.revision + 1 } }
    }
    return { ...state, [action.sessionId]: { ...current, messages:latestMessages, latestMessages, revision:current.revision + 1 } }
  }
  if (action.type === 'snapshot') {
    const latestMessages = mergePage(action.page.messages, current.latestMessages)
    const latest = {
      latestMessages,
      latestNextBeforeId: action.page.nextBeforeId,
      latestHasMoreBefore: action.page.hasMoreBefore,
      loading: false,
      revision: current.revision + 1,
    }
    if (current.mode === 'anchored') {
      return { ...state, [action.sessionId]: { ...current, ...latest, hasNewerMessages:true } }
    }
    return { ...state, [action.sessionId]: {
      ...current, ...latest, messages:latestMessages, nextBeforeId:action.page.nextBeforeId,
      hasMoreBefore:action.page.hasMoreBefore, hasNewerMessages:false,
    } }
  }
  if (action.type === 'anchored') {
    return { ...state, [action.sessionId]: {
      ...current, messages:action.page.messages.reduce<Message[]>(mergeMessage, []),
      nextBeforeId:action.page.nextBeforeId, hasMoreBefore:action.page.hasMoreBefore,
      loading:false, mode:'anchored', anchorMessageId:action.targetId, hasNewerMessages:false,
    } }
  }
  if (action.type === 'return-latest') {
    return { ...state, [action.sessionId]: {
      ...current, messages:current.latestMessages, nextBeforeId:current.latestNextBeforeId,
      hasMoreBefore:current.latestHasMoreBefore, mode:'latest', anchorMessageId:null,
      hasNewerMessages:false, loading:false,
    } }
  }
  const messages = mergePage(action.page.messages, current.messages)
  if (current.mode === 'anchored') {
    return { ...state, [action.sessionId]: { ...current, messages, nextBeforeId:action.page.nextBeforeId, hasMoreBefore:action.page.hasMoreBefore, loading:false } }
  }
  return { ...state, [action.sessionId]: {
    ...current, messages, latestMessages:messages, nextBeforeId:action.page.nextBeforeId,
    latestNextBeforeId:action.page.nextBeforeId, hasMoreBefore:action.page.hasMoreBefore,
    latestHasMoreBefore:action.page.hasMoreBefore, loading:false,
  } }
}
