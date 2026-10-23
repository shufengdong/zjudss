export type SessionStatus = 'completed' | 'running' | 'failed'
export interface Session { id: string; title: string; status: SessionStatus; updatedAt: string }
export interface HistoryHit { sessionId: string; title: string; updatedAtMs: number; score: number; snippet: string }
export interface MessagePage<T> { messages: T[]; nextBeforeId: string | null; hasMoreBefore: boolean }
