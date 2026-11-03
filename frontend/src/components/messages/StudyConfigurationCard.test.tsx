import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  ApiError,
  confirmStudyConfiguration,
  getStudyConfiguration,
  selectStudyFacade,
  selectStudyResults,
  updateStudyStruct,
} from '../../api/client'
import type { SchemaFieldContract, StudyConfigMessage, StudyConfigurationView } from '../../types'
import StudyConfigurationCard from './StudyConfigurationCard'

vi.mock('../../api/client', async (importOriginal) => ({
  ...await importOriginal<typeof import('../../api/client')>(),
  bindStudyResource: vi.fn(), confirmStudyConfiguration: vi.fn(), getStudyConfiguration: vi.fn(),
  mapStudyResource: vi.fn(), selectStudyFacade: vi.fn(), selectStudyResults: vi.fn(), updateStudyStruct: vi.fn(),
}))

const card = (overrides: Partial<SchemaFieldContract>): SchemaFieldContract => ({
  name: 'value', dtype: { kind: 'primitive', name: 'str' }, required: false, nullable: true,
  unit: 'not_applicable', card: {
    default: { state: 'omitted' }, unit: { state: 'not_applicable' },
    range: { state: 'not_applicable' }, enum: { state: 'not_applicable', values: [], caseInsensitive: false },
    visibleWhen: { state: 'always' }, enabledWhen: { state: 'always' },
  }, ...overrides,
})
const configuration = (overrides: Partial<StudyConfigurationView> = {}): StudyConfigurationView => ({
  id: 41, sessionId: 3, status: 'choosing_facade', revision: 7, engineSnapshotId: 'snapshot-4',
  facade: {
    id: 'newton', function: 'newton_raphson_pf', family: 'power-flow', title: '牛顿法潮流',
    description: '适用于一般网络。', differences: ['支持有环网络', '按用户指定方法求解'],
    inputs: [], results: [], resultConstraints: [], contractHash: 'contract',
  }, intent: {}, inputs: [], results: { available: [], selected: [], constraints: [] }, ...overrides,
})
const message = (value: StudyConfigurationView): StudyConfigMessage => ({
  id: 'message-1', type: 'study-config', timestamp: '2026-01-01T00:00:00Z', revision: value.revision,
  card: { configurationId: String(value.id), revision: value.revision, state: value.status, actionCard: value },
})

describe('StudyConfigurationCard', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    window.sessionStorage.clear()
  })

  it('starts expanded and can collapse and restore the current confirmation controls', () => {
    const ready = configuration({ status: 'ready_to_confirm' })
    render(<StudyConfigurationCard message={message(ready)} notify={vi.fn()} />)

    const collapse = screen.getByRole('button', { name: '收起研究配置' })
    expect(collapse).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByText('修订 7 · 待确认')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '确认并生成脚本' })).toBeInTheDocument()

    fireEvent.click(collapse)
    expect(screen.getByText('已收起，展开后可继续配置或确认')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '确认并生成脚本' })).not.toBeInTheDocument()
    expect(window.sessionStorage.getItem('study-configuration:41:collapsed')).toBe('true')

    const expand = screen.getByRole('button', { name: '展开研究配置' })
    expect(expand).toHaveAttribute('aria-expanded', 'false')
    fireEvent.click(expand)
    expect(screen.getByText('修订 7 · 待确认')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '确认并生成脚本' })).toBeInTheDocument()
    expect(window.sessionStorage.getItem('study-configuration:41:collapsed')).toBeNull()
  })

  it('keeps a collapsed preference across remounts and newer revisions', () => {
    const ready = configuration({ status: 'ready_to_confirm' })
    const first = render(<StudyConfigurationCard message={message(ready)} notify={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: '收起研究配置' }))
    first.unmount()

    const revised = { ...ready, revision: 8 }
    render(<StudyConfigurationCard message={message(revised)} notify={vi.fn()} />)

    expect(screen.getByRole('button', { name: '展开研究配置' })).toHaveAttribute('aria-expanded', 'false')
    expect(screen.getByText('修订 8 · 待确认')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '确认并生成脚本' })).not.toBeInTheDocument()
  })

  it('automatically expands a collapsed card when a mutation reports a stale configuration', async () => {
    const current = configuration()
    let rejectMutation: ((reason: unknown) => void) | undefined
    vi.mocked(selectStudyFacade).mockReturnValue(new Promise((_resolve, reject) => {
      rejectMutation = reject
    }))
    vi.mocked(getStudyConfiguration).mockResolvedValue({ ...current, revision: 8 })
    render(<StudyConfigurationCard message={message(current)} notify={vi.fn()} />)

    fireEvent.click(screen.getByRole('button', { name: '选择此方法' }))
    fireEvent.click(screen.getByRole('button', { name: '收起研究配置' }))
    expect(screen.queryByRole('button', { name: '选择此方法' })).not.toBeInTheDocument()
    await act(async () => rejectMutation?.(new ApiError(409, '配置已过期', 'configuration_stale')))

    expect(await screen.findByRole('alert')).toHaveTextContent('配置已更新，请确认最新内容后重试')
    expect(screen.getByRole('button', { name: '收起研究配置' })).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByRole('button', { name: '选择此方法' })).toBeInTheDocument()
    expect(window.sessionStorage.getItem('study-configuration:41:collapsed')).toBeNull()
  })

  it('keeps terminal configuration cards compact without a collapse control', () => {
    const confirmed = render(<StudyConfigurationCard message={message(configuration({ status: 'confirmed' }))} notify={vi.fn()} />)
    expect(screen.getByText('研究配置 #41：已确认')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /研究配置/ })).not.toBeInTheDocument()
    confirmed.unmount()

    render(<StudyConfigurationCard message={message(configuration({ status: 'cancelled' }))} notify={vi.fn()} />)
    expect(screen.getByText('研究配置 #41：已取消')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /研究配置/ })).not.toBeInTheDocument()
  })

  it('explains the selected facade differences and sends the current revision', async () => {
    const next = configuration({ status: 'collecting_inputs', revision: 8 })
    vi.mocked(selectStudyFacade).mockResolvedValue(next)
    render(<StudyConfigurationCard message={message(configuration())} notify={vi.fn()} />)
    expect(screen.getByText('支持有环网络')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '选择此方法' }))
    await waitFor(() => expect(selectStudyFacade).toHaveBeenCalledWith('41', 7, 'newton'))
  })

  it('renders bool, semantic number, enum, string, date and unit fields and preserves typed values', async () => {
    const fields = [
      card({ name: 'enabled', dtype: { kind: 'primitive', name: 'bool' } }),
      card({ name: 'max_iter', dtype: { kind: 'semantic', name: 'u32_semantics' } }),
      card({ name: 'method', card: { ...card({}).card!, enum: { state: 'values', values: ['exact', 'modified'], caseInsensitive: false } } }),
      card({ name: 'label' }),
      card({ name: 'start_at', dtype: { kind: 'semantic', name: 'datetime' } }),
      card({ name: 'frequency', dtype: { kind: 'primitive', name: 'f64' }, unit: 'Hz', card: { ...card({}).card!, unit: { state: 'value', value: 'Hz' } } }),
    ]
    const current = configuration({
      status: 'collecting_inputs', inputs: [{
        inputId: 'para', alias: 'para', kind: 'struct', schemaRef: 'frames.para', required: true, position: 1,
        binding: { resourceId: 'resource-7' }, resourceOptions: [],
        structCard: { schemaRef: 'frames.para', fields, values: { enabled: true, max_iter: 20, method: 'exact', label: 'case', start_at: '2026-01-01T00:00', frequency: 50 } },
      }],
    })
    vi.mocked(updateStudyStruct).mockResolvedValue({ ...current, revision: 8 })
    render(<StudyConfigurationCard message={message(current)} notify={vi.fn()} />)
    expect(screen.getByLabelText(/enabled/)).toHaveAttribute('type', 'checkbox')
    expect(screen.getByLabelText(/max_iter/)).toHaveAttribute('type', 'number')
    expect(screen.getByLabelText(/method/).tagName).toBe('SELECT')
    expect(screen.getByLabelText(/label/)).toHaveAttribute('type', 'text')
    expect(screen.getByLabelText(/start_at/)).toHaveAttribute('type', 'datetime-local')
    expect(screen.getByLabelText(/frequency \(Hz\)/)).toHaveValue(50)
    fireEvent.change(screen.getByLabelText(/max_iter/), { target: { value: '30' } })
    fireEvent.click(screen.getByRole('button', { name: '确认参数' }))
    await waitFor(() => expect(updateStudyStruct).toHaveBeenCalledWith('41', 'para', expect.objectContaining({
      expectedRevision: 7, values: expect.objectContaining({ max_iter: 30 }), replacementAction: 'replace',
    })))
  })

  it('requires every catalog-required Struct field before confirming parameters', () => {
    const current = configuration({
      status: 'collecting_inputs', inputs: [{
        inputId: 'para', alias: 'para', kind: 'struct', schemaRef: 'frames.para', required: true, position: 1,
        binding: { resourceId: null }, resourceOptions: [],
        structCard: { schemaRef: 'frames.para', fields: [card({ name: 'base_frequency_hz', required: true, nullable: false, dtype: { kind: 'primitive', name: 'f64' } })], values: {} },
      }],
    })
    render(<StudyConfigurationCard message={message(current)} notify={vi.fn()} />)
    const confirm = screen.getByRole('button', { name: '确认参数' })
    expect(confirm).toBeDisabled()
    fireEvent.change(screen.getByLabelText(/base_frequency_hz/), { target: { value: '50' } })
    expect(confirm).toBeEnabled()
  })

  it('hides server-managed result selector fields from Struct editing', () => {
    const current = configuration({
      status: 'collecting_inputs', inputs: [{
        inputId: 'para', alias: 'para', kind: 'struct', schemaRef: 'frames.para', required: true, position: 1,
        binding: { resourceId: 'resource-7' }, resourceOptions: [],
        structCard: {
          schemaRef: 'frames.para',
          fields: [card({ name: 'max_iter', dtype: { kind: 'primitive', name: 'u32' } }), card({ name: 'result', required: true })],
          values: { max_iter: 20, result: 'bus_voltage' },
          resultSelectorFields: ['result'],
        },
      }],
    })
    render(<StudyConfigurationCard message={message(current)} notify={vi.fn()} />)

    expect(screen.getByLabelText(/max_iter/)).toBeInTheDocument()
    expect(screen.queryByLabelText(/^result/)).not.toBeInTheDocument()
    expect(screen.getByText('结果字段由结果选择自动写入')).toBeInTheDocument()
    expect(screen.getByText('当前值来自已选择的参数资源')).toBeInTheDocument()
  })

  it('asks for an ambiguous Struct resource before showing its parameter editor', () => {
    const current = configuration({
      status: 'collecting_inputs', inputs: [{
        inputId: 'para', alias: 'para', kind: 'struct', schemaRef: 'frames.para', required: true, position: 1,
        binding: { resourceId: null, mapping: { selectionRequired: true } },
        resourceOptions: [
          { resourceId: 'resource-1', displayName: '参数一', alias: 'para_one', contentHash: 'a', rowCount: 1 },
          { resourceId: 'resource-2', displayName: '参数二', alias: 'para_two', contentHash: 'b', rowCount: 1 },
        ],
        structCard: { schemaRef: 'frames.para', fields: [card({ name: 'max_iter' })], values: { max_iter: 20 }, resultSelectorFields: [] },
      }],
    })
    render(<StudyConfigurationCard message={message(current)} notify={vi.fn()} />)

    expect(screen.getByRole('button', { name: '确认文件' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '确认参数' })).not.toBeInTheDocument()
  })

  it('renders resolved non-Struct inputs as compact ready rows', () => {
    const current = configuration({
      status: 'collecting_inputs', inputs: [{
        inputId: 'load_profile', alias: 'load_profile', kind: 'data', schemaRef: 'frames.load_profile', required: true, position: 1,
        binding: { resourceId: 'resource-7' },
        resourceOptions: [{ resourceId: 'resource-7', displayName: '负荷曲线', alias: 'load_profile', contentHash: 'a', rowCount: 24 }],
      }],
    })
    render(<StudyConfigurationCard message={message(current)} notify={vi.fn()} />)

    expect(screen.getByText('已就绪')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '确认文件' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '生成映射资源' })).not.toBeInTheDocument()
  })

  it('reserves column mapping for an unresolved Data input', () => {
    const current = configuration({
      status: 'collecting_inputs', inputs: [{
        inputId: 'load_profile', alias: 'load_profile', kind: 'data', schemaRef: 'frames.load_profile', required: true, position: 1,
        binding: { resourceId: null }, resourceOptions: [],
      }],
    })
    render(<StudyConfigurationCard message={message(current)} notify={vi.fn()} />)
    expect(screen.getByRole('button', { name: '生成映射资源' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '确认文件' })).not.toBeInTheDocument()
  })

  it('keeps durable Struct cards without resultSelectorFields renderable', () => {
    const current = configuration({
      status: 'collecting_inputs', inputs: [{
        inputId: 'para', alias: 'para', kind: 'struct', schemaRef: 'frames.para', required: true, position: 1,
        binding: { resourceId: 'resource-7' }, resourceOptions: [],
        structCard: { schemaRef: 'frames.para', fields: [card({ name: 'result' })], values: { result: 'bus_voltage' } },
      }],
    })
    render(<StudyConfigurationCard message={message(current)} notify={vi.fn()} />)
    expect(screen.getByLabelText(/^result/)).toHaveValue('bus_voltage')
  })

  it('keeps a pending Struct replacement visible after the configuration is reloaded', async () => {
    const current = configuration({
      status: 'collecting_inputs', inputs: [{
        inputId: 'para', alias: 'para', kind: 'struct', schemaRef: 'frames.para', required: true, position: 1,
        binding: { resourceId: 'resource-7', mapping: { baselineValues: { base_frequency_hz: null, max_iter: 20 }, replacementAction: null } },
        resourceOptions: [],
        structCard: {
          schemaRef: 'frames.para',
          fields: [
            card({ name: 'base_frequency_hz', required: true, nullable: false, dtype: { kind: 'primitive', name: 'f64' } }),
            card({ name: 'max_iter', dtype: { kind: 'primitive', name: 'u32' } }),
          ],
          values: { base_frequency_hz: 50, max_iter: 20 },
        },
      }],
    })
    vi.mocked(updateStudyStruct).mockResolvedValue({ ...current, status: 'ready_to_confirm', revision: 8 })
    render(<StudyConfigurationCard message={message(current)} notify={vi.fn()} />)
    expect(screen.getByText('已修改：base_frequency_hz')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '确认参数' }))
    await waitFor(() => expect(updateStudyStruct).toHaveBeenCalledWith('41', 'para', expect.objectContaining({
      expectedRevision: 7, replacementAction: 'replace',
    })))
  })

  it('shows the final confirmation after saving the last required parameter', async () => {
    const current = configuration({
      status: 'collecting_inputs', inputs: [{
        inputId: 'para', alias: 'para', kind: 'struct', schemaRef: 'frames.para', required: true, position: 1,
        binding: { resourceId: null }, resourceOptions: [],
        structCard: { schemaRef: 'frames.para', fields: [card({ name: 'base_frequency_hz', required: true, nullable: false, dtype: { kind: 'primitive', name: 'f64' } })], values: {} },
      }],
    })
    vi.mocked(updateStudyStruct).mockResolvedValue({ ...current, status: 'ready_to_confirm', revision: 8 })
    render(<StudyConfigurationCard message={message(current)} notify={vi.fn()} />)
    fireEvent.change(screen.getByLabelText(/base_frequency_hz/), { target: { value: '50' } })
    fireEvent.click(screen.getByRole('button', { name: '确认参数' }))
    expect(await screen.findByRole('button', { name: '确认并生成脚本' })).toBeInTheDocument()
  })

  it('refreshes a stale configuration and shows the server error without confirming', async () => {
    const current = configuration()
    vi.mocked(selectStudyFacade).mockRejectedValue(new ApiError(409, '配置已过期', 'configuration_stale'))
    vi.mocked(getStudyConfiguration).mockResolvedValue({ ...current, revision: 8 })
    render(<StudyConfigurationCard message={message(current)} notify={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: '选择此方法' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('配置已更新，请确认最新内容后重试')
    expect(getStudyConfiguration).toHaveBeenCalledWith('41')
    expect(screen.queryByRole('button', { name: '确认并生成脚本' })).not.toBeInTheDocument()
  })

  it('ready-to-confirm configurations can change results before final confirmation', async () => {
    const available = [
      { id: 'bus_voltage', name: 'bus_voltage', schemaRef: 'outputs.bus_voltage', description: '母线电压', recommended: true },
      { id: 'summary', name: 'summary', schemaRef: 'outputs.summary', description: '执行摘要', recommended: false },
    ]
    const ready = configuration({
      status: 'ready_to_confirm',
      results: { available, selected: ['bus_voltage'], constraints: [] },
    })
    vi.mocked(selectStudyResults).mockResolvedValue({
      ...ready,
      revision: 8,
      results: { ...ready.results, selected: ['bus_voltage', 'summary'] },
    })

    render(<StudyConfigurationCard message={message(ready)} notify={vi.fn()} />)

    expect(screen.getByRole('checkbox', { name: /bus_voltage/ })).toBeChecked()
    expect(screen.getByRole('checkbox', { name: /summary/ })).not.toBeChecked()
    expect(screen.getByRole('button', { name: '确认并生成脚本' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('checkbox', { name: /summary/ }))
    fireEvent.click(screen.getByRole('button', { name: '确认结果' }))

    await waitFor(() => expect(selectStudyResults).toHaveBeenCalledWith('41', 7, ['bus_voltage', 'summary']))
    expect(await screen.findByText('修订 8 · 待确认')).toBeInTheDocument()
    expect(screen.getByText('bus_voltage、summary')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '确认并生成脚本' })).toBeInTheDocument()
  })

  it('offers generation only for the complete summary and confirms explicitly', async () => {
    const ready = configuration({ status: 'ready_to_confirm' })
    vi.mocked(confirmStudyConfiguration).mockResolvedValue({
      id: 1, sessionId: 3, configurationId: 41, configurationRevision: 7, sourceWorkflowId: null,
      status: 'queued', correctionBudget: 3, correctionsUsed: 0, progressMessageId: 1,
      finalScriptResourceId: null, runId: 5, scriptMessageId: null, error: null, attempts: [],
      outputDatasetIds: [],
      createdAt: '2026-01-01T00:00:00Z', updatedAt: '2026-01-01T00:00:00Z', finishedAt: null,
    })
    render(<StudyConfigurationCard message={message(ready)} notify={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: '确认并生成脚本' }))
    await waitFor(() => expect(confirmStudyConfiguration).toHaveBeenCalledWith('41', 7, expect.any(String)))
  })
})
