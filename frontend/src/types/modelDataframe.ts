import type { ResultColumn } from './dataset'

export type OpenDataFrame =
  | { kind: 'result'; datasetId: string }
  | { kind: 'model-version'; dataframeId: string; versionId: string; tableName: string }
  | { kind: 'model-draft'; dataframeId: string; draftId: string; revision: number; tableName: string }

export type ModelDataFrameSource = Exclude<OpenDataFrame, { kind: 'result' }>

export interface ModelDataFrameMetadata {
  id: string
  scope: 'model_version' | 'draft'
  ownerId: string
  revision?: number
  tableName: string
  rowCount: number
  sizeBytes: number
  columns: ResultColumn[]
  schemaHash: string
  contentHash: string
}

export interface ModelDataFramePage {
  resourceId: string
  columns: string[]
  rows: unknown[][]
  nextCursor: string | null
  diagnostics: Record<string, number>
}
