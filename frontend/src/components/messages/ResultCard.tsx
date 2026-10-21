import type { MessageHandlers, ResultMessage } from '../../types'
import { ChartIcon, DownloadIcon } from '../icons'

function Metric({
  label,
  value,
  unit,
  warning,
}: {
  label: string
  value: string | number
  unit?: string
  warning?: boolean
}) {
  return (
    <div className="rounded-lg bg-slate-50 px-3 py-2.5">
      <div className="text-xs text-slate-500">{label}</div>
      <div className={`mt-0.5 text-lg font-semibold leading-tight ${warning ? 'text-red-600' : 'text-slate-800'}`}>
        {value}
        {unit ? <span className="ml-0.5 text-xs font-normal text-slate-400">{unit}</span> : null}
      </div>
    </div>
  )
}

export default function ResultCard({
  message,
  handlers,
}: {
  message: ResultMessage
  handlers: MessageHandlers
}) {
  const { metrics } = message
  return (
    <div className="mx-4 my-2 max-w-[78%] rounded-xl border border-slate-200 bg-white p-4 shadow-sm sm:mx-6">
      <div className="mb-3 flex items-center gap-2">
        <ChartIcon className="h-4 w-4 text-brand-600" />
        <span className="text-sm font-semibold text-slate-800">{message.title}</span>
        <span className="ml-auto font-mono text-xs text-slate-400">{message.taskId}</span>
      </div>
      <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
        <Metric label="最大电压" value={metrics.maxVoltage} unit="p.u." />
        <Metric label="最小电压" value={metrics.minVoltage} unit="p.u." warning={metrics.minVoltage < 0.95} />
        <Metric label="越限节点数" value={metrics.violationNodes} unit="个" warning={metrics.violationNodes > 0} />
        <Metric label="越限时长" value={metrics.violationDuration} />
      </div>
      <div className="mt-3.5 flex gap-2">
        <button
          type="button"
          onClick={() => handlers.onViewCurves(message.taskId)}
          className="inline-flex items-center gap-1.5 rounded-lg bg-brand-500 px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-brand-600"
        >
          <ChartIcon className="h-3.5 w-3.5" />
          查看曲线
        </button>
        <button
          type="button"
          onClick={() => handlers.onDownloadReport(message.taskId)}
          className="inline-flex items-center gap-1.5 rounded-lg border border-slate-200 bg-white px-3 py-1.5 text-xs font-medium text-slate-600 transition-colors hover:bg-slate-50"
        >
          <DownloadIcon className="h-3.5 w-3.5" />
          下载报告
        </button>
      </div>
    </div>
  )
}
