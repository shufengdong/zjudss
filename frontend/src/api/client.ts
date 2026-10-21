// HTTP 客户端：封装后端 REST 端点（契约见 docs/api-design.md）。
//
// 字段命名与 frontend/src/types/index.ts 完全一致（后端按 api-design §8 对齐），
// 因此这里的返回值直接复用前端类型，不做任何转换。

import type { CurveData, Message, Session } from '../types'

/** 非 2xx 响应统一抛出的错误；`error` 字段来自后端统一错误体 `{ "error": "..." }`。 */
export class ApiError extends Error {
  status: number

  constructor(status: number, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
  }
}

/** 从失败响应里尽力解析后端统一错误体 `{ "error": "..." }`，构造 ApiError。 */
async function errorFrom(res: Response): Promise<ApiError> {
  let message = `请求失败（${res.status}）`
  try {
    const body = (await res.json()) as { error?: string }
    if (typeof body.error === 'string') message = body.error
  } catch {
    // 忽略非 JSON 响应体
  }
  return new ApiError(res.status, message)
}

/** 基础请求：默认按 JSON 解析；multipart 时不手写 Content-Type；非 2xx 抛 ApiError。 */
async function request<T>(
  url: string,
  init: RequestInit = {},
  parse: (res: Response) => Promise<T> = (res) => res.json() as Promise<T>,
): Promise<T> {
  const headers = new Headers(init.headers)
  if (init.body !== undefined && !(init.body instanceof FormData) && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json')
  }
  const res = await fetch(url, { ...init, headers })
  if (!res.ok) throw await errorFrom(res)
  return parse(res)
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

/** PATCH /api/sessions/{id}：重命名，返回更新后的会话。 */
export async function renameSession(sessionId: string, title: string): Promise<Session> {
  return request<Session>(`/api/sessions/${sessionId}`, {
    method: 'PATCH',
    body: JSON.stringify({ title }),
  })
}

/** DELETE /api/sessions/{id}：删除会话（204，无响应体）。 */
export async function deleteSession(sessionId: string): Promise<void> {
  await request<void>(`/api/sessions/${sessionId}`, { method: 'DELETE' }, async () => undefined)
}

/** POST /api/sessions/{id}/attachments：multipart 上传（字段名 `file`），返回 attachment 消息。 */
export async function uploadAttachment(sessionId: string, file: File): Promise<Message> {
  const form = new FormData()
  form.append('file', file)
  return request<Message>(`/api/sessions/${sessionId}/attachments`, {
    method: 'POST',
    body: form,
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

/** GET /api/tasks/{id}/curves：任务结果曲线；无数据 → 404。 */
export async function fetchCurves(taskId: string): Promise<CurveData> {
  return request<CurveData>(`/api/tasks/${taskId}/curves`)
}

/** GET /api/tasks/{id}/report：下载报告文件（二进制），返回 Blob 与文件名。 */
export async function fetchReport(taskId: string): Promise<{ blob: Blob; filename: string }> {
  const res = await fetch(`/api/tasks/${taskId}/report`)
  if (!res.ok) throw await errorFrom(res)
  const blob = await res.blob()
  const cd = res.headers.get('Content-Disposition') ?? ''
  const match = /filename="?([^";]+)"?/.exec(cd)
  return { blob, filename: match?.[1] ?? `report-${taskId}.md` }
}
