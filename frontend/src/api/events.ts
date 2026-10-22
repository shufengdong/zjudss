import type { Message } from '../types'

export interface SseControlEvent {
  type: 'resync-required'
  reason: string
}

export interface EventSubscription {
  opened: Promise<void>
  close: () => void
}

export interface EventCallbacks {
  onMessage: (message: Message) => void
  onControl: (event: SseControlEvent) => void
  onError?: () => void
}

/**
 * 建立可恢复 SSE。浏览器重连会自动携带服务端 message 帧的 Last-Event-ID；调用方应先等待
 * opened，再读取 REST 快照，期间到达的事件交给 reducer 缓存和去重。
 */
export function subscribeEvents(sessionId: string, callbacks: EventCallbacks): EventSubscription {
  const source = new EventSource(`/api/sessions/${sessionId}/events`)
  let opened = false
  let resolveOpened!: () => void
  let rejectOpened!: (reason: Error) => void
  const openedPromise = new Promise<void>((resolve, reject) => {
    resolveOpened = resolve
    rejectOpened = reject
  })

  source.onopen = () => {
    opened = true
    resolveOpened()
  }
  source.onmessage = (event) => {
    try {
      callbacks.onMessage(JSON.parse(event.data) as Message)
    } catch {
      // 单个损坏事件由下一次持久快照或重连恢复。
    }
  }
  source.addEventListener('resync-required', (rawEvent) => {
    try {
      const payload = JSON.parse((rawEvent as MessageEvent<string>).data) as { reason?: string }
      callbacks.onControl({
        type: 'resync-required',
        reason: typeof payload.reason === 'string' ? payload.reason : 'cursor_unavailable',
      })
    } catch {
      callbacks.onControl({ type: 'resync-required', reason: 'cursor_unavailable' })
    }
  })
  source.onerror = () => {
    if (!opened) rejectOpened(new Error('事件流连接失败'))
    callbacks.onError?.()
  }

  return {
    opened: openedPromise,
    close: () => source.close(),
  }
}
