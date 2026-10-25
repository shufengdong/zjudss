import type {
  AnalysisRun, DatasetMetadata, DatasetPage, DatasetViewData, ExampleDetail, ExampleTreeNode,
  HistoryHit, Message, MessagePage, ModelImport, ModelVersion, RunComparison, ScriptDetail,
  ScriptSummary, Session, SimulationRun, StorageUsage, User, VersionDeletionPreview, Workspace,
} from '../types'
import { emitAuthenticationRequired } from '../state/auth'

export class ApiError extends Error {
  constructor(public status:number, message:string, public code?:string, public details?:unknown) {
    super(message); this.name='ApiError'
  }
}
async function errorFrom(res:Response) {
  try {
    const body=await res.json() as {error?:string;code?:string;details?:unknown}
    return new ApiError(res.status,body.error??`请求失败（${res.status}）`,body.code,body.details)
  } catch { return new ApiError(res.status,`请求失败（${res.status}）`) }
}
export async function request<T>(url:string,init:RequestInit={},parse:(r:Response)=>Promise<T>=(r)=>r.json() as Promise<T>,reportAuthentication=true):Promise<T>{
  const headers=new Headers(init.headers)
  if(init.body!==undefined&&!(init.body instanceof FormData)&&!headers.has('Content-Type')) headers.set('Content-Type','application/json')
  const res=await fetch(url,{...init,headers})
  if(!res.ok) {
    const error=await errorFrom(res)
    if(reportAuthentication&&res.status===401) emitAuthenticationRequired()
    throw error
  }
  return parse(res)
}
const params=(values:Record<string,string|number|undefined|null>)=>{
  const p=new URLSearchParams()
  Object.entries(values).forEach(([k,v])=>{if(v!==undefined&&v!==null&&v!=='')p.set(k,String(v))})
  const value=p.toString(); return value?`?${value}`:''
}
export const login=(id:string,password:string)=>request<{user:User}>('/api/auth/login',{method:'POST',body:JSON.stringify({id,password})},(r)=>r.json() as Promise<{user:User}>,false)
export const logout=()=>request<void>('/api/auth/logout',{method:'POST'},async()=>undefined,false)
export const getCurrentUser=()=>request<User>('/api/me')
export const listSessions=async()=>(await request<{sessions:Session[]}>('/api/sessions')).sessions
export const createSession=(title='')=>request<Session>('/api/sessions',{method:'POST',body:JSON.stringify({title})})
export const renameSession=(id:string,title:string)=>request<Session>(`/api/sessions/${id}`,{method:'PATCH',body:JSON.stringify({title})})
export const deleteSession=async(id:string)=>request<void>(`/api/sessions/${id}`,{method:'DELETE'},async()=>undefined)
export const listMessages=(id:string,q:{beforeId?:string;aroundId?:string;limit?:number}={})=>
  request<MessagePage<Message>>(`/api/sessions/${id}/messages${params(q)}`)
export const postMessage=async(id:string,message:string)=>request<{ok:boolean;sessionTitle:string}>(`/api/sessions/${id}/messages`,{method:'POST',body:JSON.stringify({message})})
export async function uploadModelFiles(sessionId:string,clientImportId:string,files:{id:string;file:File;relativePath?:string}[],draft?:{id:string;revision:number}|null) {
  const form=new FormData(); form.append('clientImportId',clientImportId)
  if(draft){form.append('expectedDraftId',draft.id);form.append('expectedDraftRevision',String(draft.revision))}
  const paths:Record<string,string>={}
  files.forEach(({id,file,relativePath})=>{form.append(`file.${id}`,file);if(relativePath)paths[id]=relativePath})
  form.append('relativePaths',JSON.stringify(paths))
  return request<{ok:boolean;modelImport:ModelImport}>(`/api/sessions/${sessionId}/model-imports`,{method:'POST',body:form})
}
export const getModelImport=async(id:string)=>(await request<{modelImport:ModelImport}>(`/api/model-imports/${id}`)).modelImport
export const resolveModelImport=(id:string,candidateIds:string[],draft?:{id:string;revision:number}|null)=>request<{modelImport:ModelImport;draft:Workspace['draft']}>(`/api/model-imports/${id}/resolve`,{method:'POST',body:JSON.stringify({candidateIds,expectedDraftId:draft?Number(draft.id):null,expectedDraftRevision:draft?.revision??null})})
export const discardModelImport=(id:string)=>request<void>(`/api/model-imports/${id}`,{method:'DELETE'},async()=>undefined)
export async function uploadScript(sessionId:string,file:File){const form=new FormData();form.append('file',file);return request<{script:ScriptSummary}>(`/api/sessions/${sessionId}/scripts`,{method:'POST',body:form})}
export const createScriptText=(sessionId:string,body:{name:string;content:string})=>request<{script:ScriptSummary}>(`/api/sessions/${sessionId}/scripts/text`,{method:'POST',body:JSON.stringify(body)})
export const listScripts=async(sessionId:string)=>(await request<{scripts:ScriptSummary[]}>(`/api/sessions/${sessionId}/scripts`)).scripts
export const getScript=async(id:string)=>(await request<{script:ScriptDetail}>(`/api/scripts/${id}`)).script
export const getWorkspace=(id:string)=>request<Workspace>(`/api/sessions/${id}/workspace`)
export const listVersions=async(id:string)=>(await request<{versions:ModelVersion[]}>(`/api/sessions/${id}/versions`)).versions
export const publishDraft=async(sessionId:string,draft:{id:string;revision:number},clientPublishId:string)=>(await request<{version:ModelVersion;deduplicated:boolean;replayed:boolean}>(`/api/sessions/${sessionId}/versions`,{method:'POST',body:JSON.stringify({expectedDraftId:Number(draft.id),expectedDraftRevision:draft.revision,clientPublishId})})).version
export const activateVersion=(id:string,draft:{id:string;revision:number}|null,discardDraft=false)=>request<Workspace>(`/api/versions/${id}/activate`,{method:'POST',body:JSON.stringify({discardDraft,expectedDraft:draft?{id:Number(draft.id),revision:draft.revision}:null})})
export const previewVersionDeletion=(id:string)=>request<VersionDeletionPreview>(`/api/versions/${id}/deletion-preview`,{method:'POST'})
export const deleteVersion=(id:string,impactToken:string)=>request<void>(`/api/versions/${id}${params({impactToken})}`,{method:'DELETE'},async()=>undefined)
export const listExamples=(filters:{query?:string;studyType?:string}={})=>
  request<{engineResourceSnapshot:string;tree:ExampleTreeNode[]}>(`/api/examples${params(filters)}`)
export const getExample=(id:string)=>request<ExampleDetail>(`/api/examples/${id}`)
export const applyExample=(sessionId:string,exampleId:string,engineResourceSnapshot:string,clientImportId:string,draft?:{id:string;revision:number}|null)=>request<{ok:boolean;modelImport:ModelImport}>(`/api/sessions/${sessionId}/draft/example`,{method:'POST',body:JSON.stringify({exampleId,engineResourceSnapshot,clientImportId,expectedDraftId:draft?Number(draft.id):null,expectedDraftRevision:draft?.revision??null})})
export const searchHistory=async(query:string)=>(await request<{items:HistoryHit[]}>(`/api/history/search${params({query})}`)).items
export const listRuns=async(sessionId:string)=>(await request<{runs:SimulationRun[]}>(`/api/sessions/${sessionId}/runs`)).runs
export const getRun=(id:string)=>request<SimulationRun>(`/api/runs/${id}`)
export const listRunDatasets=async(id:string)=>(await request<{datasets:DatasetMetadata[]}>(`/api/runs/${id}/datasets`)).datasets
export const createRun=(scriptId:string,modelVersionId:string,clientRunId:string)=>request<{run:SimulationRun;replayed:boolean}>(`/api/scripts/${scriptId}/runs`,{method:'POST',body:JSON.stringify({modelVersionId,clientRunId})})
export const getDataset=(id:string)=>request<DatasetMetadata>(`/api/datasets/${id}`)
export const getDatasetRows=(id:string,q:{cursor?:string;limit?:number;columns?:string}={})=>request<DatasetPage>(`/api/datasets/${id}/rows${params(q)}`)
export const getDatasetView=(id:string)=>request<DatasetViewData>(`/api/dataset-views/${id}/data`)
export const getAnalysis=(id:string)=>request<AnalysisRun>(`/api/analysis-runs/${id}`)
export const getComparison=(id:string)=>request<RunComparison>(`/api/run-comparisons/${id}`)
export const deleteDataset=(id:string)=>request<{datasetId:string;tombstone:unknown}>(`/api/datasets/${id}`,{method:'DELETE'})
export const getStorageUsage=()=>request<StorageUsage>('/api/storage-usage')
export async function downloadDataset(id:string,name='dataset'){
  const res=await fetch(`/api/datasets/${id}/download`); if(!res.ok){const error=await errorFrom(res);if(res.status===401)emitAuthenticationRequired();throw error}
  return {blob:await res.blob(),filename:`${name}.parquet`}
}
