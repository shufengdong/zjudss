export type SessionStatus = 'completed' | 'running' | 'failed'
export interface Session { id: string; title: string; status: SessionStatus; updatedAt: string }
export interface HistoryHit { sessionId: string; messageId?: string|null; title: string; updatedAtMs: number; score: number; snippet: string }
export interface MessagePage<T> { messages: T[]; nextBeforeId: string | null; hasMoreBefore: boolean }
export interface SessionDeletionPreview {impactToken:string;modelCount:number;dataResourceCount:number;scriptCount:number;runCount:number;resultCount:number;analysisArtifactCount:number;logicalBytes:number;expiresAt:string}
