import type { DatasetMetadata } from './dataset'
export interface SimulationRun {id:string;sessionId:string;modelVersionId:string|null;modelContentHash:string|null;scriptResourceId:string|null;scriptName:string;scriptArtifactId:string|null;scriptContentHash:string|null;scriptMessageId:string|null;managedWorkflowId:string|null;clientRunId:string;status:'queued'|'running'|'succeeded'|'failed';stage:string;engineContractVersion:string;agentBundleVersion:string|null;agentSkillRefs:unknown;exitCode:number|null;error:unknown;createdAt:string;startedAt:string|null;finishedAt:string|null}
export interface RunDeletionPreview {impactToken:string;status:string;resultCount:number;resultBytes:number;derivedReferenceCount:number;promotedReferenceCount:number;resultIds:number[];expiresAt:string}
interface AnalysisInputBase {alias:string;ordinal:number;sourceName:string;sourceSchema:unknown;sourceSchemaHash:string;sourceContentHash:string;sourceProducer:unknown}
export type AnalysisInputSnapshot =
  | AnalysisInputBase & {sourceKind:'active_model';modelVersionId:string;modelDataframeId:string}
  | AnalysisInputBase & {sourceKind:'dataset';datasetId:string}
  | AnalysisInputBase & {sourceKind:'data_resource';resourceId:string}
export interface AnalysisRun {
  id:string;sessionId:string;status:string;stage:string;intent:unknown;scriptArtifactId:string
  script:{content:string;contentHash:string};inputs:AnalysisInputSnapshot[]
  agentBundleVersion:string;agentSkillRefs:unknown;engineContractVersion:string;engineResourceSnapshotId:string
  error:unknown;createdAt:string;startedAt:string|null;finishedAt:string|null;datasets:DatasetMetadata[]
}
export interface RunComparison {id:string;sessionId:string;baselineRunId:string;candidateRunId:string;compatibility:unknown;analysisRunId:string;createdAt:string;datasets:DatasetMetadata[]}
export interface StorageUsage {scope:string;artifactLimit:number;byteLimit:number;warningRatio:number;artifactCount:number;logicalBytes:number;isWarning:boolean;isExceeded:boolean}
