import type { ModelVersionSnapshot, Workspace } from '../../types'
function Version({label,value,onSuggest}:{label:string;value:ModelVersionSnapshot|null|undefined;onSuggest:(text:string)=>void}){
  return <section className="rounded-xl border border-slate-200 bg-white p-3">
    <div className="flex items-center"><h3 className="text-xs font-semibold text-slate-700">{label}</h3>
      {value&&<span className="ml-auto rounded bg-slate-100 px-1.5 py-0.5 font-mono text-[10px]">{value.versionId}</span>}
    </div>
    {!value?<p className="mt-2 text-xs text-slate-400">暂无模型</p>:<>
      <div className="mt-2 text-xs text-slate-500">{value.sourceKind} · {value.status} · {value.inputs?.length??value.recognizedTables?.length??0} 个输入</div>
      {(value.inputs??[]).map(input=><div key={input.inputId} className="mt-2 rounded-lg bg-slate-50 p-2 text-[11px] text-slate-600">
        <div className="truncate font-medium">{input.fileName}{input.sheetName?` / ${input.sheetName}`:''}</div>
        <div>{input.tableKind??'待识别'} {input.confidence!==null?`· ${Math.round(input.confidence*100)}%`:''}</div>
      </div>)}
      {value.missing.length>0&&<p className="mt-2 text-xs text-amber-600">缺少：{value.missing.join('、')}</p>}
      {label==='候选版本'&&value.status==='ready'&&<button onClick={()=>onSuggest(`请激活候选模型版本 ${value.versionId}`)} className="mt-3 rounded-lg bg-brand-500 px-3 py-1.5 text-xs text-white">在对话中激活</button>}
    </>}
  </section>
}
export default function WorkspacePanel({workspace,onSuggest}:{workspace?:Workspace;onSuggest:(text:string)=>void}){
  return <div className="space-y-3 p-3">
    <Version label="当前模型" value={workspace?.active} onSuggest={onSuggest}/>
    <Version label="候选版本" value={workspace?.draft} onSuggest={onSuggest}/>
    {!!workspace?.activeIngestions.length&&<section className="rounded-xl border border-blue-100 bg-blue-50 p-3 text-xs text-blue-700">
      正在处理 {workspace.activeIngestions.length} 个上传批次
    </section>}
  </div>
}
