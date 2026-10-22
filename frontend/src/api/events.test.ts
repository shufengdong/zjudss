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

afterEach(() => vi.unstubAllGlobals())

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
})
