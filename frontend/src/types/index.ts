// 会话状态
export type SessionStatus = 'completed' | 'running' | 'failed'

export interface Session {
  id: string
  title: string
  status: SessionStatus
  updatedAt: string // ISO 时间戳
}

// ---- 消息（discriminated union，共 6 种类型）----
export type MessageType =
  | 'user'
  | 'assistant'
  | 'tool-call'
  | 'progress'
  | 'result'
  | 'attachment'

interface BaseMessage {
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
  taskId: string
  stages: ProgressStage[]
}

export interface SimulationMetrics {
  maxVoltage: number
  minVoltage: number
  violationNodes: number
  violationDuration: string
}

export interface ResultMessage extends BaseMessage {
  type: 'result'
  taskId: string
  title: string
  metrics: SimulationMetrics
}

export interface Attachment {
  fileName: string
  fileSize: string
  fileType: string
}

export interface AttachmentMessage extends BaseMessage {
  type: 'attachment'
  file: Attachment
}

export type Message =
  | UserMessage
  | AssistantMessage
  | ToolCallMessage
  | ProgressMessage
  | ResultMessage
  | AttachmentMessage

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
  onViewCurves: () => void
  onDownloadReport: (taskId: string) => void
  notify: (text: string) => void
}
