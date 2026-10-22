import type { MessageHandlers, ResultMessage } from '../../types'
import { ChartIcon } from '../icons'

export default function ResultCard({
  message,
  handlers,
}: {
  message: ResultMessage
  handlers: MessageHandlers
}) {
  const producerId = message.taskId ?? message.analysisRunId ?? ''
  return (
    <div className="mx-4 my-2 max-w-[78%] rounded-xl border border-slate-200 bg-white p-4 shadow-sm sm:mx-6">
      <div className="mb-3 flex items-center gap-2">
        <ChartIcon className="h-4 w-4 text-brand-600" />
        <span className="text-sm font-semibold text-slate-800">{message.title}</span>
        <span className="ml-auto font-mono text-xs text-slate-400">{producerId}</span>
      </div>
      {message.outputs.length === 0 ? (
        <div className="text-xs text-slate-500">结果数据集正在登记</div>
      ) : (
        <div className="space-y-2">
          {message.outputs.map((output) => (
            <div key={output.datasetId} className="rounded-lg bg-slate-50 px-3 py-2 text-xs text-slate-600">
              <div className="font-medium text-slate-700">{output.name}</div>
              <div className="mt-0.5">
                {output.rowCount} 行 · {output.columns.length} 列
              </div>
            </div>
          ))}
        </div>
      )}
      {message.outputs.length > 0 && (
        <div className="mt-3.5 flex gap-2">
          {message.outputs.map(output=><button key={output.datasetId} type="button"
            onClick={() => handlers.onOpenDataset(output.datasetId)}
            className="inline-flex items-center gap-1.5 rounded-lg bg-brand-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-brand-600">
            <ChartIcon className="h-3.5 w-3.5" />查看 {output.name}
          </button>)}
        </div>
      )}
    </div>
  )
}
