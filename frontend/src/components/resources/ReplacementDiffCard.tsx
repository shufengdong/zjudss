import type { ReplacementAction, StructuredReplacementPreview } from '../../types'

export default function ReplacementDiffCard({preview,busy,onAction}:{preview:StructuredReplacementPreview;busy:boolean;onAction:(action:ReplacementAction)=>void}){
  return <section className="rounded-lg border border-amber-200 bg-amber-50 p-3 text-xs">
    <h4 className="font-semibold">资源替换差异</h4>
    <p className="mt-1 text-slate-600">schema: {preview.schemaRef} · 当前 hash: {preview.expectedContentHash.slice(0,12)}…</p>
    <pre className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap rounded bg-white p-2">{JSON.stringify(preview.differences,null,2)}</pre>
    <div className="mt-2 flex flex-wrap gap-2"><button disabled={busy} onClick={()=>onAction('replace')} className="rounded bg-brand-500 px-2 py-1 text-white">覆盖当前资源</button><button disabled={busy} onClick={()=>onAction('save_as_new')} className="rounded border bg-white px-2 py-1">另存为新资源</button><button disabled={busy} onClick={()=>onAction('cancel')} className="rounded border bg-white px-2 py-1">取消</button></div>
  </section>
}
