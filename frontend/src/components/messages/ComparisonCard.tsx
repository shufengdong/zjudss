import type { ComparisonMessage, MessageHandlers } from '../../types'

export default function ComparisonCard({ message, handlers }: { message: ComparisonMessage; handlers:MessageHandlers }) {
  return (
    <div className="mx-4 my-2 max-w-[78%] rounded-xl border border-slate-200 bg-white px-4 py-3 shadow-sm sm:mx-6">
      <div className="text-sm font-medium text-slate-700">{message.title}</div>
      <div className="mt-1 text-xs text-slate-500">
        基准运行 {message.baselineRunId} · 候选运行 {message.candidateRunId} · {message.outputs.length} 个结果集
      </div>
      <button onClick={()=>handlers.onOpenComparison(message.comparisonId)} className="mt-2 rounded-lg bg-brand-500 px-3 py-1.5 text-xs text-white">查看比较</button>
    </div>
  )
}
