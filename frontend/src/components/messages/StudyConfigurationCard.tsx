import { bindStudyResource, confirmStudyConfiguration, mapStudyResource, selectStudyFacade, selectStudyResults, updateStudyStruct } from '../../api/client'
import { useStudyConfiguration } from '../../hooks/useStudyConfiguration'
import type { StudyConfigMessage } from '../../types'
import ColumnMappingCard from '../studies/ColumnMappingCard'
import ConfigurationSummaryCard from '../studies/ConfigurationSummaryCard'
import FacadeChoiceCard from '../studies/FacadeChoiceCard'
import ResourceBindingCard from '../studies/ResourceBindingCard'
import ResultSelectionCard from '../studies/ResultSelectionCard'
import StructParameterCard from '../studies/StructParameterCard'

const id=()=>crypto.randomUUID?.()??`${Date.now()}-${Math.random()}`
export default function StudyConfigurationCard({message,onWorkflowStarted,notify}:{message:StudyConfigMessage;onWorkflowStarted?:()=>void;notify:(text:string)=>void}){
  const{configuration,busy,error,mutate}=useStudyConfiguration(message.card.actionCard)
  const configId=String(configuration.id)
  const run=async(action:Parameters<typeof mutate>[0])=>{const result=await mutate(action);if(result===null&&error)notify(error)}
  if(['confirmed','cancelled'].includes(configuration.status))return <article className="mx-auto my-3 max-w-3xl rounded-xl border bg-white p-3 text-sm">研究配置 #{configuration.id}：{configuration.status==='confirmed'?'已确认':'已取消'}</article>
  return <article className="mx-auto my-3 max-w-3xl space-y-3 rounded-xl border bg-white p-3" aria-label="研究配置卡">
    <header><h2 className="text-sm font-semibold">研究配置 · {configuration.facade.title}</h2><p className="text-xs text-slate-500">修订 {configuration.revision}</p></header>
    {error&&<p role="alert" className="rounded bg-red-50 p-2 text-xs text-red-700">{error}</p>}
    {configuration.status==='choosing_facade'&&<FacadeChoiceCard facade={configuration.facade} busy={busy} onSelect={()=>void run(current=>selectStudyFacade(configId,current.revision,current.facade.id))}/>} 
    {configuration.status==='collecting_inputs'&&configuration.inputs.map(input=>input.kind==='model'?<section key={input.inputId} className="rounded-lg border p-3 text-xs"><b>{input.alias}</b><p className="mt-1 text-slate-500">模型表：{input.binding.modelTableName??'等待 active 模型'}</p></section>:input.kind==='struct'&&input.structCard?<StructParameterCard key={input.inputId} input={input} busy={busy} onSave={(values,replacementAction)=>void run(current=>updateStudyStruct(configId,input.inputId,{expectedRevision:current.revision,values,replacementAction}))}/>:input.resourceOptions.length?<ResourceBindingCard key={input.inputId} input={input} busy={busy} onSave={resourceId=>void run(current=>bindStudyResource(configId,input.inputId,current.revision,resourceId))}/>:<ColumnMappingCard key={input.inputId} input={input} busy={busy} onSave={(sourceResourceId,mappings)=>void run(current=>mapStudyResource(configId,input.inputId,{expectedRevision:current.revision,sourceResourceId,displayName:`${input.alias} 映射`,alias:`${input.alias}_mapped`,mappings}))}/>)}
    {configuration.status==='collecting_inputs'&&<ResultSelectionCard card={configuration.results} busy={busy} onSave={results=>void run(current=>selectStudyResults(configId,current.revision,results))}/>} 
    {configuration.status==='ready_to_confirm'&&<ConfigurationSummaryCard configuration={configuration} busy={busy} onConfirm={()=>void (async()=>{try{await confirmStudyConfiguration(configId,configuration.revision,id());notify('已确认，正在生成并执行脚本');onWorkflowStarted?.()}catch(cause){notify(cause instanceof Error?cause.message:'确认失败')}})()}/>} 
  </article>
}
