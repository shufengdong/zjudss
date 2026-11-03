import { useEffect, useState } from 'react'
import { bindStudyResource, confirmStudyConfiguration, mapStudyResource, selectStudyFacade, selectStudyResults, updateStudyStruct } from '../../api/client'
import { useStudyConfiguration } from '../../hooks/useStudyConfiguration'
import type { ColumnMapping, StudyConfigMessage, StudyInputCard } from '../../types'
import ColumnMappingCard from '../studies/ColumnMappingCard'
import ConfigurationSummaryCard from '../studies/ConfigurationSummaryCard'
import FacadeChoiceCard from '../studies/FacadeChoiceCard'
import ResourceBindingCard from '../studies/ResourceBindingCard'
import ResultSelectionCard from '../studies/ResultSelectionCard'
import StructParameterCard from '../studies/StructParameterCard'

const id = () => crypto.randomUUID?.() ?? `${Date.now()}-${Math.random()}`
const statusLabel = (status: string) => ({
  choosing_facade: '待选择方法', collecting_inputs: '待补充配置', ready_to_confirm: '待确认', invalidated: '已失效',
}[status] ?? status)
const readCollapsed = (key: string) => { try { return window.sessionStorage.getItem(key) === 'true' } catch { return false } }
const writeCollapsed = (key: string, collapsed: boolean) => { try { if (collapsed) window.sessionStorage.setItem(key, 'true'); else window.sessionStorage.removeItem(key) } catch { /* storage unavailable */ } }

const mappingFlag = (input: StudyInputCard, name: string) => {
  const mapping = input.binding.mapping
  return Boolean(mapping && typeof mapping === 'object' && !Array.isArray(mapping) && (mapping as Record<string, unknown>)[name] === true)
}

function ReadyInput({ input, text = '已就绪' }: { input: StudyInputCard; text?: string }) {
  const resource = input.resourceOptions.find((option) => option.resourceId === input.binding.resourceId)
  return <section className="flex items-center justify-between gap-3 rounded-lg border bg-slate-50 p-3 text-xs">
    <div><b>{input.alias}</b><p className="mt-1 text-slate-500">{resource ? `${resource.displayName}（${resource.alias}）` : input.schemaRef}</p></div>
    <span className="shrink-0 text-emerald-700">{text}</span>
  </section>
}

function StudyInputSection({ input, busy, onBind, onMap, onStruct }: {
  input: StudyInputCard; busy: boolean
  onBind: (resourceId: string | null) => void
  onMap: (sourceResourceId: string, mappings: ColumnMapping[]) => void
  onStruct: (values: Record<string, unknown>) => void
}) {
  if (input.kind === 'model') return <ReadyInput input={input} text={input.binding.modelTableName ? '已就绪' : '等待 active 模型'} />
  if (mappingFlag(input, 'selectionRequired')) return <ResourceBindingCard input={input} busy={busy} onSave={onBind} />
  if (input.kind === 'struct' && input.structCard) return <StructParameterCard input={input} busy={busy} onSave={onStruct} />
  if (input.binding.resourceId) return <ReadyInput input={input} />
  if (input.resourceOptions.length) return <ResourceBindingCard input={input} busy={busy} onSave={onBind} />
  if (input.kind === 'data') return <ColumnMappingCard input={input} busy={busy} onSave={onMap} />
  return <ReadyInput input={input} text="等待可用资源" />
}

export default function StudyConfigurationCard({ message, onWorkflowStarted, notify }: { message: StudyConfigMessage; onWorkflowStarted?: () => void; notify: (text: string) => void }) {
  const { configuration, busy, error, mutate } = useStudyConfiguration(message.card.actionCard)
  const configId = String(configuration.id), collapseKey = `study-configuration:${message.card.configurationId}:collapsed`
  const [collapsed, setCollapsed] = useState(() => readCollapsed(collapseKey))
  const run = async (action: Parameters<typeof mutate>[0]) => { const result = await mutate(action); if (result === null && error) notify(error) }
  useEffect(() => { if (error) { writeCollapsed(collapseKey, false); setCollapsed(false) } }, [collapseKey, error])
  const toggle = () => setCollapsed((current) => { const next = !current; writeCollapsed(collapseKey, next); return next })
  if (['confirmed', 'cancelled'].includes(configuration.status)) return <article className="mx-auto my-3 max-w-3xl rounded-xl border bg-white p-3 text-sm">研究配置 #{configuration.id}：{configuration.status === 'confirmed' ? '已确认' : '已取消'}</article>
  return <article className="mx-auto my-3 max-w-3xl space-y-3 rounded-xl border bg-white p-3" aria-label="研究配置卡">
    <header className="flex items-start justify-between gap-3">
      <div><h2 className="text-sm font-semibold">研究配置 · {configuration.facade.title}</h2><p className="text-xs text-slate-500">修订 {configuration.revision} · {statusLabel(configuration.status)}</p></div>
      <button type="button" aria-expanded={!collapsed} onClick={toggle} className="shrink-0 rounded border px-2.5 py-1 text-xs text-slate-700">{collapsed ? '展开研究配置' : '收起研究配置'}</button>
    </header>
    {collapsed ? <p className="rounded bg-slate-50 p-2 text-xs text-slate-500">已收起，展开后可继续配置或确认</p> : <>
      {error && <p role="alert" className="rounded bg-red-50 p-2 text-xs text-red-700">{error}</p>}
      {configuration.status === 'choosing_facade' && <FacadeChoiceCard facade={configuration.facade} busy={busy} onSelect={() => void run((current) => selectStudyFacade(configId, current.revision, current.facade.id))} />}
      {configuration.status === 'collecting_inputs' && configuration.inputs.map((input) => <StudyInputSection
        key={input.inputId}
        input={input}
        busy={busy}
        onBind={(resourceId) => void run((current) => bindStudyResource(configId, input.inputId, current.revision, resourceId))}
        onStruct={(values) => void run((current) => updateStudyStruct(configId, input.inputId, { expectedRevision: current.revision, values }))}
        onMap={(sourceResourceId, mappings) => void run((current) => mapStudyResource(configId, input.inputId, { expectedRevision: current.revision, sourceResourceId, displayName: `${input.alias} 映射`, alias: `${input.alias}_mapped`, mappings }))}
      />)}
      {['collecting_inputs', 'ready_to_confirm'].includes(configuration.status) && <ResultSelectionCard card={configuration.results} busy={busy} onSave={(results) => void run((current) => selectStudyResults(configId, current.revision, results))} />}
      {configuration.status === 'ready_to_confirm' && <ConfigurationSummaryCard configuration={configuration} busy={busy} onConfirm={() => void (async () => { try { await confirmStudyConfiguration(configId, configuration.revision, id()); notify('已确认，正在生成并执行脚本'); onWorkflowStarted?.() } catch (cause) { notify(cause instanceof Error ? cause.message : '确认失败') } })()} />}
    </>}
  </article>
}
