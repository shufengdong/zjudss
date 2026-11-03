import type { ChartModel } from '../../types'
import ChartRenderer from './ChartRenderer'

export default function ChartPreview({ model, loading, error, stale }:{model:ChartModel|null;loading:boolean;error:string|null;stale:boolean}) {
  return <section className="min-h-24 rounded border bg-slate-50 p-3" aria-label="图表预览">
    {loading&&<p className="text-sm text-slate-500">正在生成预览…</p>}
    {error&&<p role="alert" className="text-sm text-red-600">{error}</p>}
    {stale&&model&&<p className="text-xs text-amber-600">配置已更改，请重新生成预览。</p>}
    {model&&!stale&&<><ChartRenderer model={model}/><p className="text-xs text-slate-500">输出点 {model.diagnostics.outputPoints}，丢弃缺失 {model.diagnostics.droppedMissing}</p></>}
  </section>
}
