import type { ComparisonMessage } from '../../types'

export default function ComparisonCard({ message }: { message: ComparisonMessage }) {
  return (
    <div className="mx-4 my-2 max-w-[78%] rounded-xl border border-slate-200 bg-white px-4 py-3 shadow-sm sm:mx-6">
      <div className="text-sm font-medium text-slate-700">{message.title}</div>
      <div className="mt-1 text-xs text-slate-500">
        基准任务 {message.baselineTaskId} · 候选任务 {message.candidateTaskId} · {message.outputs.length} 个结果集
      </div>
    </div>
  )
}
