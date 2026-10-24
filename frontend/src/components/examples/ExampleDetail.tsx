import type { ExampleDetail as Detail } from '../../types'
export default function ExampleDetail({example}:{example:Detail}){
  const roles=example.files.reduce<Record<string,number>>((counts,file)=>({...counts,[file.role]:(counts[file.role]??0)+1}),{})
  return <div aria-label={`案例详情“${example.title}”`} className="space-y-4">
    <div><h3 className="text-lg font-semibold text-slate-900">{example.title}</h3><p className="mt-1 break-all text-xs text-slate-500">{example.bundlePath}</p></div>
    <dl className="grid gap-3 text-sm sm:grid-cols-2">
      <div><dt className="text-xs text-slate-400">分析类型</dt><dd>{example.studyType??'未知'}</dd></div>
      <div><dt className="text-xs text-slate-400">求解器</dt><dd>{example.solver??'未指定'}</dd></div>
      <div><dt className="text-xs text-slate-400">兼容模式</dt><dd>{example.compatibility??'未知'}</dd></div>
      <div><dt className="text-xs text-slate-400">来源入口</dt><dd className="break-all">{example.sourceEntry??'不可用'}</dd></div>
      <div><dt className="text-xs text-slate-400">脚本入口</dt><dd className="break-all">{example.entryPath??'不可用'}</dd></div>
      <div><dt className="text-xs text-slate-400">生成器</dt><dd>{example.manifest?.generator??'不可用'}</dd></div>
    </dl>
    <div><h4 className="text-xs font-medium text-slate-500">结果类型</h4><p className="mt-1 text-sm">{example.results.length?example.results.join('、'):'未声明'}</p></div>
    <div><h4 className="text-xs font-medium text-slate-500">文件角色</h4><div className="mt-2 flex flex-wrap gap-2">{Object.entries(roles).map(([role,count])=><span key={role} className="rounded bg-slate-100 px-2 py-1 text-xs">{role}：{count}</span>)}</div></div>
    {example.diagnostics.length>0&&<div className="rounded-lg border border-amber-200 bg-amber-50 p-3"><h4 className="text-xs font-medium text-amber-800">验证诊断</h4><ul className="mt-1 list-disc pl-5 text-xs text-amber-800">{example.diagnostics.map(diagnostic=><li key={diagnostic}>{diagnostic}</li>)}</ul></div>}
    <p className="break-all text-[10px] text-slate-400">案例树哈希：{example.treeHash}</p>
  </div>
}
