import type { ChartFieldRef } from '../../../types'
import type { ChartFieldOption } from '../ChartFieldSelect'

export const fieldIdentity = (field: ChartFieldRef): string =>
  JSON.stringify({ column: field.column, path: field.path ?? [] })

export const fieldKey = (field: ChartFieldRef, options: ChartFieldOption[]): string =>
  options.find(option => fieldIdentity(option.field) === fieldIdentity(field))?.key ?? ''

export const fieldName = (field: ChartFieldRef): string =>
  `${field.column}${field.path?.map(index => `[${index}]`).join('') ?? ''}`

const cloneField = (field: ChartFieldRef): ChartFieldRef => ({
  column: field.column,
  ...(field.path ? { path: [...field.path] } : {}),
  ...(field.label ? { label: field.label } : {}),
})

export const resolveField = (
  key: string,
  options: ChartFieldOption[],
  preserved: ChartFieldRef[] = [],
): ChartFieldRef => {
  const option = options.find(candidate => candidate.key === key && !candidate.disabled)
    ?? options.find(candidate => !candidate.disabled)
  if (!option) throw new Error('没有可用的图表字段')
  const original = preserved.find(field => fieldIdentity(field) === fieldIdentity(option.field))
  return cloneField(original ?? option.field)
}
