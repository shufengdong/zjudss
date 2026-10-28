import type { ResultOutputRef } from './dataset'
import type { ScriptSummary } from './script'
export const MESSAGE_TYPES = ['user','assistant','tool-call','progress','result','attachment','model-status','comparison','script','upload-batch','study-config','workflow-progress'] as const
export type MessageType=(typeof MESSAGE_TYPES)[number]
export type ToolStatus='running'|'success'|'failed'
export type StageStatus='pending'|'running'|'done'|'failed'
export interface BaseMessage{id:string;type:MessageType;timestamp:string;revision?:number}
export interface UserMessage extends BaseMessage{type:'user';content:string}
export interface AssistantMessage extends BaseMessage{type:'assistant';content:string}
export interface ToolCallMessage extends BaseMessage{type:'tool-call';callId:string;toolName:string;args:Record<string,unknown>;status:ToolStatus;summary?:string}
export interface OperationRef{kind:'ingestion'|'run'|'analysis';id:string}
export interface ProgressMessage extends BaseMessage{type:'progress';operation:OperationRef;stages:{key:string;label:string;status:StageStatus}[]}
export interface ResultMessage extends BaseMessage{type:'result';runId?:string;analysisRunId?:string;title:string;outputs:ResultOutputRef[]}
export interface Attachment{fileName:string;fileSize:string;fileType:string}
export interface AttachmentMessage extends BaseMessage{type:'attachment';file:Attachment;clientBatchId:string;clientFileId:string;ingestionId:string;uploadStatus:'accepted'|'ingesting'|'ready'|'failed'}
export interface ModelStatusMessage extends BaseMessage{type:'model-status';workspaceId:string;scope:'active'|'draft';snapshot:unknown|null}
export interface ComparisonMessage extends BaseMessage{type:'comparison';comparisonId:string;baselineRunId:string;candidateRunId:string;analysisRunId:string;title:string;outputs:ResultOutputRef[]}
export interface ScriptMessage extends BaseMessage{type:'script';script:ScriptSummary}
export interface EventDiagnostic{code:string;message:string}
export interface UploadBatchMessage extends BaseMessage{type:'upload-batch';card:{batchId:string;batchRevision:number;state:string;candidates:{id:string;name:string;state:string}[];modelTransactionState:string;resourceTransactionState:string;diagnostic?:EventDiagnostic}}
export interface StudyConfigMessage extends BaseMessage{type:'study-config';card:{configurationId:string;revision:number;state:string;actionCard:unknown}}
export interface WorkflowProgressMessage extends BaseMessage{type:'workflow-progress';card:{workflowId:string;state:string;executionIndex:number;executionMax:number;repairIndex:number;repairMax:number;elapsedMs:number;attempts:{executionIndex:number;repairIndex:number;state:string;elapsedMs:number;failureCode?:string}[];scriptId?:string;outputDatasetIds:string[];failureCode?:string}}
export type Message=UserMessage|AssistantMessage|ToolCallMessage|ProgressMessage|ResultMessage|AttachmentMessage|ModelStatusMessage|ComparisonMessage|ScriptMessage|UploadBatchMessage|StudyConfigMessage|WorkflowProgressMessage
export interface MessageHandlers{onOpenDataset:(datasetId:string)=>void;onOpenComparison:(comparisonId:string)=>void;onRunScript:(script:ScriptSummary)=>Promise<void>;notify:(text:string)=>void}
