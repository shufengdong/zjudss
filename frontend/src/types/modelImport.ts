export type ModelImportStatus='accepted'|'processing'|'awaiting_resolution'|'ready_to_apply'|'applied'|'failed'|'discarded'
export interface ModelImportCandidate {id:string;kind:'entry'|'table';displayName:string;tableName:string|null;schema:unknown[]|null;rowCount:number|null;selected:boolean}
export interface ModelImport {id:string;sessionId:string;clientImportId:string;sourceKind:'upload'|'case';baseDraft:{id:number;revision:number}|null;status:ModelImportStatus;stages:unknown;diagnostics:unknown;candidates:ModelImportCandidate[];errorCode:string|null;errorMessage:string|null}
