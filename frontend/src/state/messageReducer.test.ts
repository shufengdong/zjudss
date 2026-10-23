import { describe, expect, it } from 'vitest'
import type { Message } from '../types'
import { initialMessageState, messageReducer } from './messageReducer'

const base = { timestamp: '2026-10-22T00:00:00Z' }

describe('messageReducer', () => {
  it('merges progress by operation and tool calls by stable callId', () => {
    let state = messageReducer(initialMessageState, { type: 'select-session', sessionId: 's1' })
    const events: Message[] = [
      {
        ...base,
        id: '1',
        type: 'progress',
        operation: { kind: 'run', id: 'run-7' },
        stages: [{ key: 'prepare', label: '准备', status: 'running' }],
      },
      {
        ...base,
        id: '2',
        type: 'progress',
        operation: { kind: 'run', id: 'run-7' },
        stages: [{ key: 'prepare', label: '准备', status: 'done' }],
      },
      {
        ...base,
        id: '3',
        type: 'tool-call',
        callId: 'call-1',
        toolName: 'run_simulation',
        args: {},
        status: 'running',
      },
      {
        ...base,
        id: '4',
        type: 'tool-call',
        callId: 'call-1',
        toolName: 'run_simulation',
        args: {},
        status: 'success',
        summary: '完成',
      },
    ]
    for (const message of events) {
      state = messageReducer(state, { type: 'event', sessionId: 's1', message })
    }

    expect(state.messages).toHaveLength(2)
    expect(state.messages[0]).toMatchObject({ id: '2', type: 'progress' })
    expect(state.messages[1]).toMatchObject({ id: '4', type: 'tool-call', status: 'success' })
  })

  it('merges a snapshot with buffered events and rejects stale sessions', () => {
    let state = messageReducer(initialMessageState, { type: 'select-session', sessionId: 's1' })
    state = messageReducer(state, {
      type: 'event',
      sessionId: 's1',
      message: { ...base, id: '2', type: 'assistant', content: 'live' },
    })
    state = messageReducer(state, {
      type: 'snapshot',
      sessionId: 's1',
      messages: [{ ...base, id: '1', type: 'user', content: 'history' }],
    })
    state = messageReducer(state, {
      type: 'event',
      sessionId: 'stale',
      message: { ...base, id: '3', type: 'assistant', content: 'wrong session' },
    })

    expect(state.messages.map((message) => message.id)).toEqual(['1', '2'])
  })
})
