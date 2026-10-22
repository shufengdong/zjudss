import type {
  AnalysisRun, DatasetMetadata, DatasetPage, DatasetViewData, ExampleDetail, ExampleSummary,
  HistoryHit, Ingestion, Message, MessagePage, Principal, Session, SimulationTask, StorageUsage,
  TaskComparison, Workspace,
} from '../types'

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
export async function request<T>(url:string,init:RequestInit={},parse:(r:Response)=>Promise<T>=(r)=>r.json() as Promise<T>):Promise<T>{
  const headers=new Headers(init.headers)
  if(init.body!==undefined&&!(init.body instanceof FormData)&&!headers.has('Content-Type')) headers.set('Content-Type','application/json')
  const res=await fetch(url,{...init,headers})
  if(!res.ok) throw await errorFrom(res)
  return parse(res)
}
const params=(values:Record<string,string|number|undefined|null>)=>{
  const p=new URLSearchParams()
  Object.entries(values).forEach(([k,v])=>{if(v!==undefined&&v!==null&&v!=='')p.set(k,String(v))})
  const value=p.toString(); return value?`?${value}`:''
}
export const getPrincipal=()=>request<Principal>('/api/me')
export const listSessions=async()=>(await request<{sessions:Session[]}>('/api/sessions')).sessions
export const createSession=(title='')=>request<Session>('/api/sessions',{method:'POST',body:JSON.stringify({title})})
export const renameSession=(id:string,title:string)=>request<Session>(`/api/sessions/${id}`,{method:'PATCH',body:JSON.stringify({title})})
export const deleteSession=async(id:string)=>request<void>(`/api/sessions/${id}`,{method:'DELETE'},async()=>undefined)
export const listMessages=(id:string,q:{beforeId?:string;aroundId?:string;limit?:number}={})=>
  request<MessagePage<Message>>(`/api/sessions/${id}/messages${params(q)}`)
export const postMessage=async(id:string,message:string)=>request<{ok:boolean}>(`/api/sessions/${id}/messages`,{method:'POST',body:JSON.stringify({message})})
export async function uploadFiles(sessionId:string,batchId:string,files:{id:string;file:File;relativePath?:string}[]) {
  const form=new FormData(); form.append('clientBatchId',batchId)
  const paths:Record<string,string>={}
  files.forEach(({id,file,relativePath})=>{form.append(`file.${id}`,file);if(relativePath)paths[id]=relativePath})
  form.append('relativePaths',JSON.stringify(paths))
  return request<{ok:boolean;ingestionId:string;accepted:unknown[]}>(`/api/sessions/${sessionId}/attachments`,{method:'POST',body:form})
}
export const getIngestion=async(id:string)=>(await request<{ingestion:Ingestion}>(`/api/ingestions/${id}`)).ingestion
export const getWorkspace=(id:string)=>request<Workspace>(`/api/sessions/${id}/workspace`)
export const listExamples=(filters:{query?:string;studyType?:string;sourceKind?:string;limit?:number}={})=>
  request<{engineResourceSnapshot:string;examples:ExampleSummary[];nextCursor:null}>(`/api/examples${params(filters)}`)
export const getExample=(id:string)=>request<ExampleDetail>(`/api/examples/${id}`)
export const searchHistory=async(query:string)=>(await request<{items:HistoryHit[]}>(`/api/history/search${params({query})}`)).items
export const listTasks=(sessionId:string)=>request<SimulationTask[]>(`/api/sessions/${sessionId}/tasks`)
export const getTask=(id:string)=>request<SimulationTask>(`/api/tasks/${id}`)
export const listTaskDatasets=(id:string)=>request<DatasetMetadata[]>(`/api/tasks/${id}/datasets`)
export const getDataset=(id:string)=>request<DatasetMetadata>(`/api/datasets/${id}`)
export const getDatasetRows=(id:string,q:{cursor?:string;limit?:number;columns?:string}={})=>request<DatasetPage>(`/api/datasets/${id}/rows${params(q)}`)
export const getDatasetView=(id:string)=>request<DatasetViewData>(`/api/dataset-views/${id}/data`)
export const getAnalysis=(id:string)=>request<AnalysisRun>(`/api/analysis-runs/${id}`)
export const getComparison=(id:string)=>request<TaskComparison>(`/api/task-comparisons/${id}`)
export const getStorageUsage=()=>request<StorageUsage>('/api/storage-usage')
export async function downloadDataset(id:string,name='dataset'){
  const res=await fetch(`/api/datasets/${id}/download`); if(!res.ok)throw await errorFrom(res)
  return {blob:await res.blob(),filename:`${name}.parquet`}
}
