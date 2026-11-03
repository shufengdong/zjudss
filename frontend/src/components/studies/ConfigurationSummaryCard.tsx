import type { StudyConfigurationView, StudyInputCard } from '../../types'

function structBaseline(input: StudyInputCard): Record<string, unknown> {
  const mapping = input.binding.mapping
  if (mapping && typeof mapping === 'object' && !Array.isArray(mapping)) {
    const baseline = (mapping as Record<string, unknown>).baselineValues
    if (baseline && typeof baseline === 'object' && !Array.isArray(baseline)) {
      return baseline as Record<string, unknown>
    }
  }
  return input.structCard?.values ?? {}
}

function structDifferences(input: StudyInputCard): string[] {
  if (input.kind !== 'struct' || !input.structCard) return []
  const baseline = structBaseline(input)
  const selectors = new Set(input.structCard.resultSelectorFields ?? [])
  return input.structCard.fields
    .filter((field) => !selectors.has(field.name))
    .filter((field) => JSON.stringify(input.structCard!.values[field.name]) !== JSON.stringify(baseline[field.name]))
    .map((field) => field.name)
}

function resourceLabel(input: StudyInputCard): string {
  const differences = structDifferences(input)
  const override = differences.length ? ` · 运行覆盖：${differences.join('、')}` : ''
  if (!input.binding.resourceId) {
    return `${input.alias} → ${input.kind === 'struct' ? '目录默认值' : '新资源'}${override}`
  }
  const resource = input.resourceOptions.find((option) => option.resourceId === input.binding.resourceId)
  return `${input.alias} → ${resource ? `${resource.displayName}（${resource.alias}）` : `资源 #${input.binding.resourceId}`}${override}`
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
