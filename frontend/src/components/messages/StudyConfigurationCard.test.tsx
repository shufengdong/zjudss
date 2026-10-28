import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  ApiError,
  confirmStudyConfiguration,
  getStudyConfiguration,
  selectStudyFacade,
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
  beforeEach(() => vi.clearAllMocks())

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

  it('offers generation only for the complete summary and confirms explicitly', async () => {
    const ready = configuration({ status: 'ready_to_confirm' })
    vi.mocked(confirmStudyConfiguration).mockResolvedValue({
      id: 1, sessionId: 3, configurationId: 41, configurationRevision: 7, sourceWorkflowId: null,
      status: 'queued', correctionBudget: 3, correctionsUsed: 0, progressMessageId: 1,
      finalScriptResourceId: null, runId: 5, scriptMessageId: null, error: null, attempts: [],
      createdAt: '2026-01-01T00:00:00Z', updatedAt: '2026-01-01T00:00:00Z', finishedAt: null,
    })
    render(<StudyConfigurationCard message={message(ready)} notify={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: '确认并生成脚本' }))
    await waitFor(() => expect(confirmStudyConfiguration).toHaveBeenCalledWith('41', 7, expect.any(String)))
  })
})
