export interface ScriptSummary {id:string;sessionId:string;name:string;sourceKind:'upload'|'agent'|'manual';contentHash:string;validationStatus:'valid'|'invalid';requiredAliases:string[];outputs:string[];diagnostics:unknown;preview:string;previewTruncated:boolean;canRun:boolean;disabledReason:string|null;createdAt:string}
export interface ScriptDetail extends ScriptSummary {content:string}
