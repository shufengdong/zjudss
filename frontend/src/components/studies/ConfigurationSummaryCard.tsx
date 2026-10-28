import type { StudyConfigurationView, StudyInputCard } from '../../types'

function resourceLabel(input: StudyInputCard): string {
  if (!input.binding.resourceId) return `${input.alias} → 新资源`
  const resource = input.resourceOptions.find((option) => option.resourceId === input.binding.resourceId)
  const metadata = input.binding.mapping && typeof input.binding.mapping === 'object'
    ? input.binding.mapping as { replacementAction?: unknown; differences?: unknown }
    : null
  const action = metadata?.replacementAction === 'replace' ? '覆盖' : metadata?.replacementAction === 'save_as_new' ? '另存' : null
  const differences = Array.isArray(metadata?.differences) ? metadata.differences.length : 0
  return `${input.alias} → ${resource ? `${resource.displayName}（${resource.alias}）` : `资源 #${input.binding.resourceId}`}${action ? ` · ${action}` : ''}${differences ? ` · ${differences} 项修改` : ''}`
}

export default function ConfigurationSummaryCard({ configuration, busy, onConfirm }: {
  configuration: StudyConfigurationView
  busy: boolean
  onConfirm: () => void
}) {
  const model = configuration.inputs.filter((input) => input.kind === 'model')
  const resources = configuration.inputs.filter((input) => input.kind !== 'model')
  return <section className="rounded-lg border border-emerald-200 bg-emerald-50 p-3 text-xs">
    <h3 className="font-semibold">请确认研究配置</h3>
    <dl className="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1">
      <dt>函数</dt><dd className="break-all">{configuration.facade.function}</dd>
      <dt>引擎快照</dt><dd className="break-all">{configuration.engineSnapshotId}</dd>
      <dt>模型输入</dt><dd>{model.length ? model.map((input) => `${input.alias} → ${input.binding.modelTableName ?? '缺失'}`).join('；') : '无需模型'}</dd>
      <dt>数据/参数</dt><dd>{resources.map(resourceLabel).join('；') || '无'}</dd>
      <dt>结果</dt><dd>{configuration.results.selected.join('、')}</dd>
    </dl>
    <button type="button" disabled={busy} onClick={onConfirm} className="mt-3 rounded bg-emerald-600 px-3 py-1.5 text-white">确认并生成脚本</button>
  </section>
}
