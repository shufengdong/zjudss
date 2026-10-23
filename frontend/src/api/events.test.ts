import { afterEach, describe, expect, it, vi } from 'vitest'
import { subscribeEvents } from './events'

class FakeEventSource {
  static latest: FakeEventSource
  onopen: (() => void) | null = null
  onmessage: ((event: MessageEvent<string>) => void) | null = null
  onerror: (() => void) | null = null
  listeners = new Map<string, (event: Event) => void>()
  closed = false

  constructor(readonly url: string) {
    FakeEventSource.latest = this
  }

  addEventListener(type: string, listener: EventListenerOrEventListenerObject) {
    this.listeners.set(type, listener as (event: Event) => void)
  }

  close() {
    this.closed = true
  }
}

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('subscribeEvents', () => {
  it('exposes readiness, messages, control events and close', async () => {
    vi.stubGlobal('EventSource', FakeEventSource)
    const messages: string[] = []
    const controls: string[] = []
    const subscription = subscribeEvents('7', {
      onMessage: (message) => messages.push(message.id),
      onControl: (event) => controls.push(event.reason),
    })
    const source = FakeEventSource.latest
    expect(source.url).toBe('/api/sessions/7/events')

    source.onopen?.()
    await subscription.opened
    source.onmessage?.(
      new MessageEvent('message', {
        data: JSON.stringify({
          id: '12',
          type: 'assistant',
          timestamp: '2026-10-22T00:00:00Z',
          content: 'done',
        }),
      }),
    )
    source.listeners.get('resync-required')?.(
      new MessageEvent('resync-required', { data: JSON.stringify({ reason: 'cursor_unavailable' }) }),
    )

    expect(messages).toEqual(['12'])
    expect(controls).toEqual(['cursor_unavailable'])
    subscription.close()
    expect(source.closed).toBe(true)
  })

  it('closes permanently when an error probe reports missing authentication', async () => {
    vi.stubGlobal('EventSource', FakeEventSource)
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(
      JSON.stringify({ error: '请先登录', code: 'authentication_required' }),
      { status: 401, headers: { 'content-type': 'application/json' } },
    )))
    const listener = vi.fn()
    window.addEventListener('zjudss:authentication-required', listener)
    const subscription = subscribeEvents('7', { onMessage: () => undefined, onControl: () => undefined })
    void subscription.opened.catch(() => undefined)

    FakeEventSource.latest.onerror?.()
    await vi.waitFor(() => expect(FakeEventSource.latest.closed).toBe(true))
    expect(listener).toHaveBeenCalledTimes(1)
    window.removeEventListener('zjudss:authentication-required', listener)
  })
})
