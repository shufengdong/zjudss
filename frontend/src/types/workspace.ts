import type { ModelImport } from './modelImport'
export interface DataframeTable {id:string;tableName:string;artifactId:string;schema:unknown[];rowCount:number;contentHash:string}
export interface WorkspaceDraft {id:string;workspaceId:string;baseVersionId:string|null;revision:number;diagnostics:unknown;tables:DataframeTable[];createdAt:string;updatedAt:string}
export interface ModelVersion {id:string;workspaceId:string;contentHash:string;sourceSummary:unknown;engineContractVersion:string;tables:DataframeTable[];createdAt:string}
export interface Workspace {id:string;sessionId:string;active:ModelVersion|null;draft:WorkspaceDraft|null;activeModelImports:ModelImport[]}
export interface VersionDeletionPreview {impactToken:string;runCount:number;resultCount:number;reclaimableBytes:number;expiresAt:string}
export interface ExampleSummary {
  id:string; title:string; description:string; sourceKind:string; studyType:string; solver:string|null
  tags:string[]; requiredFeatures:string[]; fileCount:number; sizeBytes:number; verification:string; importable:boolean
}
export interface ExampleDetail extends ExampleSummary {
  entryPath:string; treeHash:string; files:{path:string;sha256:string}[]; diagnostics:string[]
}
