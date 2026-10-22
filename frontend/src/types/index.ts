// 会话状态
export type SessionStatus = 'completed' | 'running' | 'failed'

export interface Session {
  id: string
  title: string
  status: SessionStatus
  updatedAt: string // ISO 时间戳
}

// ---- v2 消息（discriminated union，共 8 种类型）----
export type MessageType =
  | 'user'
  | 'assistant'
  | 'tool-call'
  | 'progress'
  | 'result'
  | 'attachment'
  | 'model-status'
  | 'comparison'

export interface BaseMessage {
  id: string
  type: MessageType
  timestamp: string // ISO 时间戳
}

export interface UserMessage extends BaseMessage {
  type: 'user'
  content: string
}

export interface AssistantMessage extends BaseMessage {
  type: 'assistant'
  content: string // Markdown
}

export type ToolStatus = 'running' | 'success' | 'failed'

export interface ToolCallMessage extends BaseMessage {
  type: 'tool-call'
  callId: string
  toolName: string
  args: Record<string, unknown>
  status: ToolStatus
  summary?: string
}

export type StageStatus = 'pending' | 'running' | 'done' | 'failed'

export interface ProgressStage {
  key: string
  label: string
  status: StageStatus
}

export interface ProgressMessage extends BaseMessage {
  type: 'progress'
  operation: OperationRef
  stages: ProgressStage[]
}

export interface OperationRef {
  kind: 'ingestion' | 'simulation' | 'analysis'
  id: string
}

export type VisualizationKind = 'line' | 'bar' | 'scatter' | 'heatmap' | 'boxplot' | 'histogram'

export interface ResultColumn {
  name: string
  dtype: string
  nullable: boolean
  unit?: string
}

export interface DatasetViewRef {
  id: string
  name: string
  kind: VisualizationKind
  isDefault: boolean
}

export interface ResultOutputRef {
  datasetId: string
  name: string
  rowCount: number
  columns: ResultColumn[]
  previewRows?: unknown[][]
  previewTruncated: boolean
  defaultViewId: string | null
  views: DatasetViewRef[]
}

export interface ResultMessage extends BaseMessage {
  type: 'result'
  taskId?: string
  analysisRunId?: string
  title: string
  outputs: ResultOutputRef[]
}

export interface Attachment {
  fileName: string
  fileSize: string
  fileType: string
}

export interface AttachmentMessage extends BaseMessage {
  type: 'attachment'
  file: Attachment
  clientBatchId: string
  clientFileId: string
  ingestionId: string
  uploadStatus: 'accepted' | 'ingesting' | 'ready' | 'failed'
}

export interface ModelVersionSnapshot {
  versionId: string
  status: 'collecting' | 'ready' | 'invalid'
  recognizedTables: {
    inputId: string
    artifactId: string
    tableKind: string
    confidence: number
  }[]
  missing: string[]
  ambiguities: { inputId: string; candidates: string[] }[]
}

export interface ModelStatusMessage extends BaseMessage {
  type: 'model-status'
  workspaceId: string
  scope: 'active' | 'draft'
  snapshot: ModelVersionSnapshot | null
}

export interface ComparisonMessage extends BaseMessage {
  type: 'comparison'
  comparisonId: string
  baselineTaskId: string
  candidateTaskId: string
  analysisRunId: string
  title: string
  outputs: ResultOutputRef[]
}

export type Message =
  | UserMessage
  | AssistantMessage
  | ToolCallMessage
  | ProgressMessage
  | ResultMessage
  | AttachmentMessage
  | ModelStatusMessage
  | ComparisonMessage

// ---- 曲线 ----
export interface CurveSeries {
  name: string // 馈线名
  values: number[]
}

export interface CurveData {
  variable: string // 变量名，如「电压」
  unit: string // 单位，如「p.u.」
  timeAxis: string[] // 时间轴
  series: CurveSeries[]
}

// ---- 结果面板标签页 ----
export type ResultTab = 'curves' | 'report' | 'logs'

// ---- 消息组件需要的外部动作（查看曲线 / 下载报告 / 轻提示）----
export interface MessageHandlers {
  onViewCurves: (taskId: string) => void
  onDownloadReport: (taskId: string) => void
  notify: (text: string) => void
}
