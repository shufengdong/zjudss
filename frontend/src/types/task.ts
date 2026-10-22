import type { DatasetMetadata } from './dataset'
export interface SimulationTask {
  id:string; sessionId:string; status:string; stage:string|null; studyType:string; studySpec:unknown
  modelVersionId:string; scriptArtifactId:string; agentBundleVersion:string; agentSkillRefs:unknown
  engineContractVersion:string; error:unknown; createdAt:string; startedAt:string|null; finishedAt:string|null
}
export interface AnalysisRun { id:string;sessionId:string;status:string;stage:string;intent:unknown;createdAt:string;datasets:DatasetMetadata[] }
export interface TaskComparison {
  id:string;sessionId:string;baselineTaskId:string;candidateTaskId:string;compatibility:unknown
  analysisRunId:string;createdAt:string;datasets:DatasetMetadata[]
}
export interface StorageUsage {
  scope:string;artifactLimit:number;byteLimit:number;warningRatio:number;artifactCount:number
  logicalBytes:number;isWarning:boolean;isExceeded:boolean
}
