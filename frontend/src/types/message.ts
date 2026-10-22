import type { ResultOutputRef } from './dataset'
import type { ModelVersionSnapshot } from './workspace'
export const MESSAGE_TYPES = ['user','assistant','tool-call','progress','result','attachment','model-status','comparison'] as const
export type MessageType=(typeof MESSAGE_TYPES)[number]
export type ToolStatus='running'|'success'|'failed'
export type StageStatus='pending'|'running'|'done'|'failed'
export interface BaseMessage{id:string;type:MessageType;timestamp:string}
export interface UserMessage extends BaseMessage{type:'user';content:string}
export interface AssistantMessage extends BaseMessage{type:'assistant';content:string}
export interface ToolCallMessage extends BaseMessage{type:'tool-call';callId:string;toolName:string;args:Record<string,unknown>;status:ToolStatus;summary?:string}
export interface OperationRef{kind:'ingestion'|'simulation'|'analysis';id:string}
export interface ProgressMessage extends BaseMessage{type:'progress';operation:OperationRef;stages:{key:string;label:string;status:StageStatus}[]}
export interface ResultMessage extends BaseMessage{type:'result';taskId?:string;analysisRunId?:string;title:string;outputs:ResultOutputRef[]}
export interface Attachment{fileName:string;fileSize:string;fileType:string}
export interface AttachmentMessage extends BaseMessage{type:'attachment';file:Attachment;clientBatchId:string;clientFileId:string;ingestionId:string;uploadStatus:'accepted'|'ingesting'|'ready'|'failed'}
export interface ModelStatusMessage extends BaseMessage{type:'model-status';workspaceId:string;scope:'active'|'draft';snapshot:ModelVersionSnapshot|null}
export interface ComparisonMessage extends BaseMessage{type:'comparison';comparisonId:string;baselineTaskId:string;candidateTaskId:string;analysisRunId:string;title:string;outputs:ResultOutputRef[]}
export type Message=UserMessage|AssistantMessage|ToolCallMessage|ProgressMessage|ResultMessage|AttachmentMessage|ModelStatusMessage|ComparisonMessage
export interface MessageHandlers{onOpenDataset:(datasetId:string)=>void;onOpenComparison:(comparisonId:string)=>void;notify:(text:string)=>void}
