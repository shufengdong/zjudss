import type { ReplacementAction } from './uploadBatch'

export type StudyConfigurationState='choosing_facade'|'collecting_inputs'|'ready_to_confirm'|'confirmed'|'invalidated'|'cancelled'
export type DataTypeContract={kind:'primitive'|'semantic';name:string}|{kind:'constructor';name:string;element:DataTypeContract}
export interface StructCardContract {default:{state:string;value?:unknown};unit:{state:string;value?:string};range:{state:string;minimum?:unknown;maximum?:unknown;minimumInclusive?:boolean;maximumInclusive?:boolean};enum:{state:string;values:unknown[];caseInsensitive:boolean};visibleWhen:{state:string;when?:unknown};enabledWhen:{state:string;when?:unknown}}
export interface SchemaFieldContract {name:string;dtype:DataTypeContract;required:boolean;nullable:boolean;unit:string;card?:StructCardContract|null}
export interface FacadeInputContract {id:string;alias:string;kind:'model'|'struct'|'data';schemaRef:string;required:boolean;position:number}
export interface FacadeResultContract {id:string;name:string;schemaRef:string;description:string;recommended:boolean}
export interface ResultSelectionRule {type:string;[key:string]:unknown}
export interface FacadeContract {id:string;function:string;family:string;title:string;description:string;differences:string[];inputs:FacadeInputContract[];results:FacadeResultContract[];resultConstraints:ResultSelectionRule[];contractHash:string}
export interface StudyResourceOption {resourceId:string;displayName:string;alias:string;contentHash:string;rowCount:number}
export interface StudyInputCard {inputId:string;alias:string;kind:'model'|'struct'|'data';schemaRef:string;required:boolean;position:number;binding:{modelTableName?:string|null;resourceId?:string|null;mapping?:unknown|null};resourceOptions:StudyResourceOption[];reuseOffer?:StudyResourceOption|null;structCard?:{schemaRef:string;fields:SchemaFieldContract[];values:Record<string,unknown>}|null}
export interface StudyResultCard {available:FacadeResultContract[];selected:string[];constraints:ResultSelectionRule[]}
export interface StudyConfigurationView {id:number;sessionId:number;status:StudyConfigurationState;revision:number;engineSnapshotId:string;facade:FacadeContract;intent:unknown;inputs:StudyInputCard[];results:StudyResultCard}
export interface ColumnMapping {source:string;target:string}
export interface StudyStructUpdate {expectedRevision:number;values:Record<string,unknown>;replacementAction?:ReplacementAction|null}
