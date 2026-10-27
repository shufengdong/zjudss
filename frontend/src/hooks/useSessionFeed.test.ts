import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { listMessages } from '../api/client'
import { subscribeEvents, type EventCallbacks } from '../api/events'
import type { Message } from '../types'
import { useSessionFeed } from './useSessionFeed'

vi.mock('../api/client', () => ({ listMessages: vi.fn() }))
vi.mock('../api/events', () => ({ subscribeEvents: vi.fn() }))

const mockedListMessages = vi.mocked(listMessages)
const mockedSubscribe = vi.mocked(subscribeEvents)
const base = { timestamp: '2026-10-27T00:00:00Z', revision: 1 }
const message = (id:string, content:string):Message => ({ ...base, id, type:'assistant', content })

describe('useSessionFeed anchored history', () => {
  let callbacks: EventCallbacks
  beforeEach(() => {
    mockedListMessages.mockReset()
    mockedSubscribe.mockReset()
    mockedSubscribe.mockImplementation((_id, value) => {
      callbacks = value
      return { opened: Promise.resolve(), close: vi.fn() }
    })
  })

  it('loads a disjoint around window and returns to a live latest cache', async () => {
    mockedListMessages
      .mockResolvedValueOnce({ messages:[message('100','latest')], nextBeforeId:'100', hasMoreBefore:true })
      .mockResolvedValueOnce({ messages:[message('40','target')], nextBeforeId:'40', hasMoreBefore:true })
    const { result } = renderHook(() => useSessionFeed('7'))
    await waitFor(() => expect(result.current.feed?.messages[0]?.id).toBe('100'))

    await act(async () => result.current.loadAround('40'))
    expect(mockedListMessages).toHaveBeenLastCalledWith('7', { aroundId:'40', limit:50 })
    expect(result.current.feed?.messages.map(item=>item.id)).toEqual(['40'])

    act(() => callbacks.onMessage(message('101','new')))
    expect(result.current.feed?.hasNewerMessages).toBe(true)
    expect(result.current.feed?.messages.map(item=>item.id)).toEqual(['40'])

    act(() => result.current.returnLatest())
    expect(result.current.feed?.messages.map(item=>item.id)).toEqual(['100','101'])
    expect(result.current.feed?.hasNewerMessages).toBe(false)
  })
})
