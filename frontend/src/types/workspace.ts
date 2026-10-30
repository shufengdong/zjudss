export interface DataframeTable {id:string;tableName:string;artifactId:string;schema:unknown[];rowCount:number;contentHash:string}
export interface WorkspaceDraft {id:string;workspaceId:string;baseVersionId:string|null;revision:number;diagnostics:unknown;tables:DataframeTable[];createdAt:string;updatedAt:string}
export interface ModelVersion {id:string;workspaceId:string;contentHash:string;description:string;sourceSummary:unknown;engineContractVersion:string;tables:DataframeTable[];createdAt:string}
export interface Workspace {id:string;sessionId:string;active:ModelVersion|null;draft:WorkspaceDraft|null}
export interface VersionDeletionPreview {impactToken:string;runCount:number;resultCount:number;reclaimableBytes:number;expiresAt:string}
export interface ExampleManifestFile {path:string;role:string;sha256:string}
export interface ExampleProjectionResult {source:string;target:string;binding:string}
export interface ExampleProjection {
  sourceFunction:string;profile:string;projector:string;targetFunction:string;managedEntry:string
  status:string;stableDiagnostic:string;results:ExampleProjectionResult[]
}
export interface ExampleManifestSummary {
  schemaVersion:number;artifact:string;generator:string;psaContractVersion:string;entry:string
  study:{kind:string;compatibility:string;solver:string|null;results:string[]}
  source:{entry:string;root:string;treeSha256:string}
  projection:ExampleProjection
  files:ExampleManifestFile[];inputs:unknown
}
export interface ExampleSummary {
  id:string;title:string;description:string;sourceKind:string;categoryPath:string[];bundlePath:string
  studyType:string|null;solver:string|null;compatibility:string|null;results:string[];sourceEntry:string|null
  fileCount:number;sizeBytes:number;verification:string;importable:boolean;diagnostics:string[]
}
export interface ExampleDetail extends ExampleSummary {
  entryPath:string|null;manifest:ExampleManifestSummary|null;treeHash:string;files:ExampleManifestFile[]
}
export type ExampleTreeNode =
  | {type:'directory';name:string;path:string[];children:ExampleTreeNode[]}
  | {type:'example';example:ExampleSummary}
