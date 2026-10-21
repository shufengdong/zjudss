// HTTP 客户端：封装后端 REST 端点（契约见 docs/api-design.md）。
//
// 字段命名与 frontend/src/types/index.ts 完全一致（后端按 api-design §8 对齐），
// 因此这里的返回值直接复用前端类型，不做任何转换。

import type { Message, Session } from '../types'

/** 非 2xx 响应统一抛出的错误；`error` 字段来自后端统一错误体 `{ "error": "..." }`。 */
export class ApiError extends Error {
  status: number

  constructor(status: number, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
  }
}

async function request<T>(url: string, init: RequestInit = {}): Promise<T> {
  const res = await fetch(url, {
    ...init,
    headers: { 'Content-Type': 'application/json' },
  })
  if (!res.ok) {
    let message = `请求失败（${res.status}）`
    try {
      const body = (await res.json()) as { error?: string }
      if (typeof body.error === 'string') message = body.error
    } catch {
      // 忽略非 JSON 响应体
    }
    throw new ApiError(res.status, message)
  }
  return (await res.json()) as T
}

interface SessionListResponse {
  sessions: Session[]
}

interface MessagesResponse {
  messages: Message[]
}

/** GET /api/sessions：会话列表（按最近活跃排序）。 */
export async function listSessions(): Promise<Session[]> {
  const data = await request<SessionListResponse>('/api/sessions')
  return data.sessions
}

/** POST /api/sessions：新建会话。 */
export async function createSession(title: string): Promise<Session> {
  return request<Session>('/api/sessions', {
    method: 'POST',
    body: JSON.stringify({ title }),
  })
}

/** GET /api/sessions/{id}/messages：回放历史消息（与 SSE 同构）。 */
export async function listMessages(sessionId: string): Promise<Message[]> {
  const data = await request<MessagesResponse>(`/api/sessions/${sessionId}/messages`)
  return data.messages
}

/** POST /api/sessions/{id}/messages：发用户消息 → 触发 agent，返回 202。 */
export async function postMessage(sessionId: string, message: string): Promise<void> {
  await request<{ ok: boolean }>(`/api/sessions/${sessionId}/messages`, {
    method: 'POST',
    body: JSON.stringify({ message }),
  })
}
