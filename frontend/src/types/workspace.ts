export interface VersionInput {
  inputId:string; artifactId:string; fileName:string; sheetName:string|null; tableKind:string|null
  confidence:number|null; resolution:string; candidates:unknown[]; matchedRequiredColumns:string[]
  missingRequiredColumns:string[]; typeConflicts:unknown[]
}
export interface ModelVersionSnapshot {
  versionId:string; parentVersionId?:string|null; sourceKind?:string; originKind?:string; origin?:unknown
  status:'collecting'|'ready'|'invalid'; inputs?:VersionInput[]
  recognizedTables?:{inputId:string;artifactId:string;tableKind:string;confidence:number}[]
  missing:string[]; ambiguities?:{inputId:string;candidates:string[]}[]; diagnostics?:unknown[]; createdAt?:string
}
export interface IngestionItem {
  clientFileId:string; fileName:string; relativePath:string|null; status:string
  errorCode:string|null; errorMessage:string|null
}
export interface Ingestion {
  id:string; clientBatchId:string; status:string; stages:unknown[]; items:IngestionItem[]
  errorCode:string|null; errorMessage:string|null
}
export interface Workspace {
  id:string; sessionId:string; active:ModelVersionSnapshot|null; draft:ModelVersionSnapshot|null
  activeIngestions:Ingestion[]
}
export interface ExampleSummary {
  id:string; title:string; description:string; sourceKind:string; studyType:string; solver:string|null
  tags:string[]; requiredFeatures:string[]; fileCount:number; sizeBytes:number; verification:string; importable:boolean
}
export interface ExampleDetail extends ExampleSummary {
  entryPath:string; treeHash:string; files:{path:string;sha256:string}[]; diagnostics:string[]
}
