// SSE 订阅：把后端的结构化事件流（api-design §6）转成前端 Message 回调。

import type { Message } from '../types'

/**
 * 订阅某会话的事件流。EventSource 会自动重连；但后端 broadcast 不回放历史，
 * 断线期间的事件可能丢失，完整性由 `GET /messages` 回放兜底（见 App.tsx）。
 *
 * 返回一个取消函数（关闭 EventSource）。
 */
export function subscribeEvents(
  sessionId: string,
  onMessage: (message: Message) => void,
): () => void {
  const es = new EventSource(`/api/sessions/${sessionId}/events`)
  es.onmessage = (e) => {
    try {
      onMessage(JSON.parse(e.data) as Message)
    } catch {
      // 忽略无法解析的事件
    }
  }
  return () => es.close()
}
