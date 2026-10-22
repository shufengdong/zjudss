import type { Message } from '../types'

export interface MessageState {
  sessionId: string | null
  messages: Message[]
}

export const initialMessageState: MessageState = {
  sessionId: null,
  messages: [],
}

export type MessageAction =
  | { type: 'select-session'; sessionId: string | null }
  | { type: 'event'; sessionId: string; message: Message }
  | { type: 'snapshot'; sessionId: string; messages: Message[] }
  | { type: 'optimistic'; sessionId: string; message: Message }
  | { type: 'remove-optimistic'; sessionId: string; id: string }

function sameAggregate(left: Message, right: Message): boolean {
  if (left.type !== right.type) return false
  switch (left.type) {
    case 'progress':
      return (
        right.type === 'progress' &&
        left.operation.kind === right.operation.kind &&
        left.operation.id === right.operation.id
      )
    case 'tool-call':
      return right.type === 'tool-call' && left.callId === right.callId
    case 'model-status':
      return (
        right.type === 'model-status' &&
        left.workspaceId === right.workspaceId &&
        left.scope === right.scope
      )
    case 'comparison':
      return right.type === 'comparison' && left.comparisonId === right.comparisonId
    case 'attachment':
      return (
        right.type === 'attachment' &&
        left.clientBatchId === right.clientBatchId &&
        left.clientFileId === right.clientFileId
      )
    case 'result':
      return (
        right.type === 'result' &&
        left.taskId === right.taskId &&
        left.analysisRunId === right.analysisRunId
      )
    default:
      return false
  }
}

function messageOrder(message: Message): number {
  const parsed = Number(message.id)
  return Number.isSafeInteger(parsed) ? parsed : Number.MAX_SAFE_INTEGER
}

export function mergeMessage(messages: Message[], incoming: Message): Message[] {
  if (incoming.type === 'user') {
    const optimistic = messages.findIndex(
      (message) => message.type === 'user' && message.id.startsWith('local-user-'),
    )
    if (optimistic >= 0) {
      const next = messages.slice()
      next[optimistic] = incoming
      return next.sort((left, right) => messageOrder(left) - messageOrder(right))
    }
  }
  if (messages.some((message) => message.id === incoming.id)) return messages
  const next = messages.filter((message) => !sameAggregate(message, incoming))
  next.push(incoming)
  return next.sort((left, right) => messageOrder(left) - messageOrder(right))
}

export function messageReducer(state: MessageState, action: MessageAction): MessageState {
  if (action.type === 'select-session') {
    return { sessionId: action.sessionId, messages: [] }
  }
  if (state.sessionId !== action.sessionId) return state

  switch (action.type) {
    case 'event':
    case 'optimistic':
      return { ...state, messages: mergeMessage(state.messages, action.message) }
    case 'snapshot': {
      const snapshot = action.messages.reduce<Message[]>(mergeMessage, [])
      const merged = state.messages.reduce(mergeMessage, snapshot)
      return { ...state, messages: merged }
    }
    case 'remove-optimistic':
      return {
        ...state,
        messages: state.messages.filter((message) => message.id !== action.id),
      }
  }
}
