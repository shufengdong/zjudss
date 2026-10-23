import type { ModelStatusMessage } from '../../types'

export default function ModelStatusCard({ message }: { message: ModelStatusMessage }) {
  const label = message.scope === 'active' ? '当前模型' : '候选模型'
  return (
    <div className="mx-4 my-2 max-w-[78%] rounded-xl border border-slate-200 bg-white px-4 py-3 text-sm shadow-sm sm:mx-6">
      <div className="font-medium text-slate-700">{label}</div>
      <div className="mt-1 text-xs text-slate-500">
        {message.snapshot ? `${label}状态已更新` : `${label}已清空`}
      </div>
    </div>
  )
}
