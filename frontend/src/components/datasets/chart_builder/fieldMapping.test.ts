import { expect, it } from 'vitest'
import type { ChartFieldOption } from '../ChartFieldSelect'
import {
  fieldIdentity,
  fieldKey,
  fieldName,
  resolveField,
} from './fieldMapping'

const options: ChartFieldOption[] = [
  { key: 'scalar', field: { column: 'bus' }, label: 'bus', disabled: false },
  {
    key: 'nested',
    field: { column: 'voltage', path: [1, 2], label: 'B.2' },
    label: 'voltage[1][2] · B.2',
    disabled: false,
  },
  {
    key: 'disabled',
    field: { column: 'wide' },
    label: 'wide',
    disabled: true,
  },
]

it('uses physical column and nested path as field identity', () => {
  expect(fieldIdentity({ column: 'bus', label: 'display' })).toBe(fieldIdentity({ column: 'bus' }))
  expect(fieldIdentity({ column: 'voltage', path: [1, 2] })).not.toBe(
    fieldIdentity({ column: 'voltage', path: [2, 1] }),
  )
  expect(fieldKey({ column: 'voltage', path: [1, 2] }, options)).toBe('nested')
  expect(fieldName({ column: 'voltage', path: [1, 2] })).toBe('voltage[1][2]')
})

it('falls back from disabled fields and preserves the original label for matching fields', () => {
  expect(resolveField('disabled', options)).toEqual({ column: 'bus' })
  expect(
    resolveField('nested', options, [
      { column: 'voltage', path: [1, 2], label: '冻结标签' },
    ]),
  ).toEqual({ column: 'voltage', path: [1, 2], label: '冻结标签' })
})
