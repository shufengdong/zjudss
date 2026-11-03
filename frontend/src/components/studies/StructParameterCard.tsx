import { useEffect, useMemo, useState } from 'react'
import type { SchemaFieldContract, StudyInputCard } from '../../types'

function typeName(field: SchemaFieldContract): string {
  return field.dtype.name.toLowerCase()
}

function isNumeric(field: SchemaFieldContract): boolean {
  const name = typeName(field)
  return /^(u|i)\d+/.test(name)
    || name.includes('float')
    || name.includes('number')
    || name === 'f32'
    || name === 'f64'
    || name.endsWith('_semantics')
}

function defaultValues(fields: SchemaFieldContract[]): Record<string, unknown> {
  return Object.fromEntries(fields.flatMap((field) => (
    field.card?.default.state === 'value' ? [[field.name, field.card.default.value]] : []
  )))
}

function fieldUnit(field: SchemaFieldContract): string | null {
  if (field.card?.unit.state === 'value' && field.card.unit.value) return field.card.unit.value
  return field.unit && field.unit !== 'not_applicable' ? field.unit : null
}

function structBaseline(input: StudyInputCard): Record<string, unknown> {
  const mapping = input.binding.mapping
  if (mapping && typeof mapping === 'object' && !Array.isArray(mapping)) {
    const baseline = (mapping as Record<string, unknown>).baselineValues
    if (baseline && typeof baseline === 'object' && !Array.isArray(baseline)) {
      return baseline as Record<string, unknown>
    }
  }
  return input.structCard!.values
}

export default function StructParameterCard({ input, busy, onSave }: {
  input: StudyInputCard
  busy: boolean
  onSave: (values: Record<string, unknown>) => void
}) {
  const card = input.structCard!
  const selectorFields = useMemo(() => new Set(card.resultSelectorFields ?? []), [card.resultSelectorFields])
  const editableFields = useMemo(() => card.fields.filter((field) => !selectorFields.has(field.name)), [card.fields, selectorFields])
  const defaults = useMemo(() => defaultValues(editableFields), [editableFields])
  const baseline = structBaseline(input)
  const [values, setValues] = useState<Record<string, unknown>>(card.values)
  useEffect(() => setValues(card.values), [card.values])
  const set = (name: string, value: unknown) => setValues((current) => {
    if (value === undefined) {
      const next = { ...current }
      delete next[name]
      return next
    }
    return { ...current, [name]: value }
  })
  const changedFields = editableFields.filter((field) => JSON.stringify(values[field.name]) !== JSON.stringify(baseline[field.name])).map((field) => field.name)
  const editableValues = Object.fromEntries(editableFields.flatMap((field) => (
    field.name in values ? [[field.name, values[field.name]]] : []
  )))
  const missingRequired = editableFields.some((field) => field.required
    && (values[field.name] === undefined || values[field.name] === null || values[field.name] === ''))
  const restoreDefaults = () => setValues((current) => ({
    ...Object.fromEntries([...selectorFields].filter((field) => field in current).map((field) => [field, current[field]])),
    ...defaults,
  }))

  return <section className="rounded-lg border p-3 text-xs">
    <h3 className="font-semibold">参数：{input.alias}</h3>
    <p className="mt-1 text-slate-500">
      {card.schemaRef}{input.binding.resourceId ? ` · 参数模板 #${input.binding.resourceId}` : ' · 使用目录默认值'}
    </p>
    <p className={`mt-2 rounded p-2 ${changedFields.length ? 'bg-amber-50 text-amber-800' : 'bg-slate-50 text-slate-600'}`}>
      {!input.binding.resourceId
        ? '使用目录默认值；保存后仅用于本次研究'
        : changedFields.length
          ? `本次运行覆盖：${changedFields.join('、')}；不会修改参数模板`
          : '当前值来自参数模板；本次研究不会修改模板'}
    </p>
    {selectorFields.size > 0 && <p className="mt-2 rounded bg-blue-50 p-2 text-blue-700">结果字段由结果选择自动写入</p>}
    <div className="mt-3 grid gap-3 sm:grid-cols-2">
      {editableFields.filter((field) => field.card?.visibleWhen.state !== 'never').map((field) => {
        const name = typeName(field)
        const enumValues = field.card?.enum.values ?? []
        const value = values[field.name] ?? ''
        const unit = fieldUnit(field)
        const disabled = busy || field.card?.enabledWhen.state === 'never'
        const label = `${field.name}${unit ? ` (${unit})` : ''}${field.required ? ' *' : ''}`
        return <label key={field.name} className="grid min-w-0 gap-1">
          <span>{label}</span>
          {enumValues.length > 0
            ? <select value={String(value)} disabled={disabled} onChange={(event) => set(field.name, event.target.value)} className="min-w-0 rounded border px-2 py-1.5">
              {enumValues.map((option) => <option key={String(option)} value={String(option)}>{String(option)}</option>)}
            </select>
            : name.includes('bool')
              ? <input type="checkbox" disabled={disabled} checked={Boolean(value)} onChange={(event) => set(field.name, event.target.checked)} />
              : name.includes('date') || name.includes('time')
                ? <input type="datetime-local" disabled={disabled} value={String(value)} onChange={(event) => set(field.name, event.target.value || undefined)} className="min-w-0 rounded border px-2 py-1.5" />
                : isNumeric(field)
                  ? <input type="number" disabled={disabled} value={String(value)} min={field.card?.range.minimum as number | undefined} max={field.card?.range.maximum as number | undefined} onChange={(event) => set(field.name, event.target.value === '' ? undefined : event.target.valueAsNumber)} className="min-w-0 rounded border px-2 py-1.5" />
                  : <input type="text" disabled={disabled} value={String(value)} onChange={(event) => set(field.name, event.target.value)} className="min-w-0 rounded border px-2 py-1.5" />}
        </label>
      })}
    </div>
    <div className="mt-3 flex flex-wrap items-center gap-2">
      <button type="button" disabled={busy} onClick={restoreDefaults} className="rounded border px-2 py-1.5">恢复默认</button>
      <button type="button" disabled={busy || missingRequired} onClick={() => onSave(editableValues)} className="ml-auto rounded bg-brand-500 px-3 py-1.5 text-white disabled:opacity-40">应用参数</button>
    </div>
  </section>
}
