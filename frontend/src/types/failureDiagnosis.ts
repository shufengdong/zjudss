export interface FailureAnalysisEvidence {
  statement: string
  evidence: string
}

export interface FailureAnalysis {
  summary: string
  facts: FailureAnalysisEvidence[]
  possibleCauses: FailureAnalysisEvidence[]
  unknowns: string[]
  recommendations: string[]
}

export type FailureAnalysisStatus = 'pending' | 'complete' | 'unavailable'
export type EngineLogStream = 'stdout' | 'stderr'

export interface EngineLog {
  stream: EngineLogStream
  text: string
  totalBytes: number | null
  sentBytes: number | null
  truncated: boolean
}

export interface FailureDiagnosis {
  code?: string
  message: string
  stage?: string
  exitCode: number | null
  durationMs: number | null
  engineLogs: EngineLog[]
  analysisStatus: FailureAnalysisStatus
  analysis: FailureAnalysis | null
}
