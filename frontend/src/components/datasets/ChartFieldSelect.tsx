import type { ChartCapabilities, ChartFieldRef } from '../../types'

export interface ChartFieldOption { key: string; field: ChartFieldRef; label: string; disabled: boolean; reason?: string }

const phaseLabel = (path: number[]) => path.length === 1 && path[0] < 26 ? String.fromCharCode(65 + path[0]) : path.join('.')

export function chartFieldOptions(capabilities: ChartCapabilities, role: 'category' | 'measure'): ChartFieldOption[] {
  const componentColumns = capabilities.fields.filter(field => field.shape.kind === 'list' && !field.shape.truncated && field.roles.includes('measure')).length
  return capabilities.fields.flatMap((field):ChartFieldOption[] => {
    if (!field.roles.includes(role) && !(role === 'measure' && field.shape.kind === 'list' && field.shape.truncated)) return []
    if (field.shape.kind === 'scalar') {
      const ref = { column: field.column }
      return [{ key: JSON.stringify(ref), field: ref, label: field.column, disabled: false }]
    }
    if (field.shape.truncated) {
      const ref = { column: field.column }
      return [{ key: JSON.stringify(ref), field: ref, label: `${field.column}（分量过多，不可选择）`, disabled: true, reason: '分量过多' }]
    }
    return field.shape.observedPaths.map(path => {
      const phase = phaseLabel(path), label = componentColumns > 1 ? `${field.column} ${phase}` : phase
      const ref = { column: field.column, path, label }
      return { key: JSON.stringify(ref), field: ref, label: `${field.column}[${path.join('][')}] · ${label}`, disabled: false }
    })
  })
}

export default function ChartFieldSelect({ label, options, value, onChange, multiple = false }:{
  label:string; options:ChartFieldOption[]; value:string|string[]; onChange:(value:string|string[])=>void; multiple?:boolean
}) {
  return <label className="grid gap-1 text-sm"><span>{label}</span><select aria-label={label} multiple={multiple} value={value}
    onChange={event => onChange(multiple ? [...event.currentTarget.selectedOptions].map(option => option.value) : event.currentTarget.value)}
    className="min-h-9 rounded border px-2 py-1">
    {options.map(option => <option key={option.key} value={option.key} disabled={option.disabled} title={option.reason}>{option.label}</option>)}
  </select></label>
}
