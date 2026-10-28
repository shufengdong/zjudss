import type { ReplacementAction } from './uploadBatch'

export interface DataResourceView {
  id: string
  sessionId: string
  displayName: string
  alias: string
  status: string
  declaredSchemaRef?: string | null
  sourceKind: string
  schema: unknown
  schemaHash: string
  contentHash: string
  rowCount: number
  sizeBytes: number
  originalAvailable: boolean
  sourceBatchId?: string | null
  sourceModelVersionId?: string | null
  replacedByResourceId?: string | null
  lineage: unknown
  createdAtMs: number
}

export interface DataResourcePage { resources: DataResourceView[]; nextCursor: string | null }
export interface DataResourceRows {
  resourceId: string
  columns: string[]
  rows: unknown[][]
  nextCursor: string | null
  diagnostics: Record<string, number>
}
export interface DataResourceDeletionPreview {
  impactToken: string
  resourceId: string
  displayName: string
  alias: string
  sizeBytes: number
  pendingConfigurationCount: number
  frozenReferenceCount: number
  historicalReferenceCount: number
  expiresAtMs: number
}
export interface DeletedDataResource { resourceId: string; tombstone: unknown }
export interface StructuredReplacementPreview {
  previewToken: string
  resourceId: string
  expectedContentHash: string
  schemaRef: string
  differences: unknown[]
  expiresAtMs: number
}
export interface UploadConfirmation {
  uploadBatchId: string
  status: string
  modelStatus: string
  resourceStatus: string
  modelReceipt?: unknown | null
  resourceReceipt?: unknown | null
  resources: DataResourceView[]
  diagnostics: unknown[]
}
export interface StructuredReplacementRequest { expectedContentHash: string; schemaRef: string; values: unknown }
export interface StructuredReplacementConfirmation { action: ReplacementAction }
