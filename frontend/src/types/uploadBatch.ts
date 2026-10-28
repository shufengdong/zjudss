export type UploadBatchStatus = 'processing' | 'awaiting_parse_settings' | 'awaiting_purpose' | 'ready_to_confirm' | 'partially_confirmed' | 'confirmed' | 'failed' | 'discarded' | 'expired'
export type ChildPublishStatus = 'not_applicable' | 'ready' | 'published' | 'failed'
export type CandidatePurpose = 'model_table' | 'data_resource' | 'skip'
export type ReplacementAction = 'replace' | 'save_as_new' | 'cancel'

export interface ParseSettings {
  delimiter?: number | null
  encoding: string
  headerRow: number
  decimal: string
  thousands?: string | null
  datetimeFormat?: string | null
  timezone?: string | null
  excelRange?: string | null
}

export interface CandidateDecision {
  candidateId: string
  purpose: CandidatePurpose
  selected: boolean
  tableName?: string | null
  displayName?: string | null
  alias?: string | null
}

export interface UploadCandidateView {
  id: string
  sheetName?: string | null
  displayName: string
  proposedAlias?: string | null
  purpose?: CandidatePurpose | null
  inferredSchemaRef?: string | null
  schema?: unknown
  rowCount?: number | null
  evidence: Record<string, unknown>
  selected: boolean
}

export interface UploadBatchView {
  id: string
  sessionId: string
  clientBatchId: string
  sourceKind: string
  replacementResourceId?: string | null
  status: UploadBatchStatus
  modelPublishStatus: ChildPublishStatus
  resourcePublishStatus: ChildPublishStatus
  parseSettings: Partial<ParseSettings>
  diagnostics: unknown
  reservedBytes: number
  expiresAtMs: number
  revision: number
  candidates: UploadCandidateView[]
}
