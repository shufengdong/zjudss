import type { ProgressMessage, StageStatus } from '../../types'
import { CheckIcon, XIcon } from '../icons'

function StageIcon({ status }: { status: StageStatus }) {
  if (status === 'done') {
    return (
      <span className="flex h-5 w-5 items-center justify-center rounded-full bg-emerald-500 text-white">
        <CheckIcon className="h-3 w-3" strokeWidth={3} />
      </span>
    )
  }
  if (status === 'failed') {
    return (
      <span className="flex h-5 w-5 items-center justify-center rounded-full bg-red-500 text-white">
        <XIcon className="h-3 w-3" strokeWidth={3} />
      </span>
    )
  }
  if (status === 'running') {
    return <span className="h-5 w-5 animate-spin rounded-full border-2 border-brand-500 border-t-transparent" />
  }
  return <span className="h-5 w-5 rounded-full border-2 border-slate-300" />
}

export default function ProgressCard({ message }: { message: ProgressMessage }) {
  const operationLabel = {
    ingestion: '模型摄取进度',
    simulation: '仿真进度',
    analysis: '结果分析进度',
  }[message.operation.kind]
  return (
    <div className="mx-4 my-2 max-w-[78%] rounded-xl border border-slate-200 bg-white p-3.5 shadow-sm sm:mx-6">
      <div className="mb-3 flex items-center gap-2 text-xs text-slate-400">
        <span className="font-medium text-slate-500">{operationLabel}</span>
        <span className="font-mono">{message.operation.id}</span>
      </div>
      <ol className="space-y-2.5">
        {message.stages.map((stage) => {
          const textCls =
            stage.status === 'failed'
              ? 'text-red-600'
              : stage.status === 'running'
                ? 'font-medium text-brand-600'
                : stage.status === 'done'
                  ? 'text-slate-700'
                  : 'text-slate-400'
          return (
            <li key={stage.key} className="flex items-center gap-2.5">
              <StageIcon status={stage.status} />
              <span className={`text-sm ${textCls}`}>{stage.label}</span>
            </li>
          )
        })}
      </ol>
    </div>
  )
}
