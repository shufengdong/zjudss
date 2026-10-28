import { useEffect, useMemo, useState } from 'react'
import type { ReplacementAction, SchemaFieldContract, StudyInputCard } from '../../types'

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

export default function StructParameterCard({ input, busy, onSave }: {
  input: StudyInputCard
  busy: boolean
  onSave: (values: Record<string, unknown>, action?: ReplacementAction) => void
}) {
  const card = input.structCard!
  const defaults = useMemo(() => defaultValues(card.fields), [card.fields])
  const [values, setValues] = useState<Record<string, unknown>>(card.values)
  const [action, setAction] = useState<ReplacementAction>(input.binding.resourceId ? 'replace' : 'save_as_new')
  useEffect(() => setValues(card.values), [card.values])
  const set = (name: string, value: unknown) => setValues((current) => {
    if (value === undefined) {
      const next = { ...current }
      delete next[name]
      return next
    }
    return { ...current, [name]: value }
  })
  const changedFields = card.fields.filter((field) => JSON.stringify(values[field.name]) !== JSON.stringify(card.values[field.name])).map((field) => field.name)
  const replacementAction = input.binding.resourceId && changedFields.length > 0 ? action : undefined

  return <section className="rounded-lg border p-3 text-xs">
    <h3 className="font-semibold">参数：{input.alias}</h3>
    <p className="mt-1 text-slate-500">
      {card.schemaRef}{input.binding.resourceId ? ` · 当前资源 #${input.binding.resourceId}` : ' · 将创建普通资源'}
    </p>
    {input.binding.resourceId && <p className={`mt-2 rounded p-2 ${changedFields.length ? 'bg-amber-50 text-amber-800' : 'bg-slate-50 text-slate-600'}`}>
      {changedFields.length ? `已修改：${changedFields.join('、')}` : '当前值来自已选择的普通资源'}
    </p>}
    <div className="mt-3 grid gap-3 sm:grid-cols-2">
      {card.fields.filter((field) => field.card?.visibleWhen.state !== 'never').map((field) => {
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
      {input.binding.resourceId && changedFields.length > 0 && <select aria-label="参数资源保存方式" value={action} disabled={busy} onChange={(event) => setAction(event.target.value as ReplacementAction)} className="rounded border px-2 py-1.5">
        <option value="replace">覆盖当前资源</option>
        <option value="save_as_new">另存为新资源</option>
      </select>}
      <button type="button" disabled={busy} onClick={() => setValues(defaults)} className="rounded border px-2 py-1.5">恢复默认</button>
      <button type="button" disabled={busy} onClick={() => onSave(values, replacementAction)} className="ml-auto rounded bg-brand-500 px-3 py-1.5 text-white">确认参数</button>
    </div>
  </section>
}
