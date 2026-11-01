import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { cancelManagedWorkflow, getManagedWorkflow, listRunDatasets } from '../../api/client'
import type { ManagedWorkflowView, WorkflowProgressMessage } from '../../types'
import WorkflowProgressCard from './WorkflowProgressCard'

vi.mock('../../api/client', () => ({ cancelManagedWorkflow: vi.fn(), getManagedWorkflow: vi.fn(), listRunDatasets: vi.fn() }))

const workflow = (status: string): ManagedWorkflowView => ({
  id: 9, sessionId: 1, configurationId: 2, configurationRevision: 3, sourceWorkflowId: null,
  status, correctionBudget: 3, correctionsUsed: 1, progressMessageId: 4, finalScriptResourceId: 7,
  runId: 8, scriptMessageId: 10, outputDatasetIds: [], error: status === 'failed' ? { code: 'repair_exhausted', message: '三次修正后仍失败' } : null,
  attempts: [{ id: 1, attemptNumber: 1, parentAttemptId: null, phase: 'execute', status: 'failed',
    scriptContentHash: 'abc', exitCode: 2, sanitizedStderr: 'unknown function', diagnostic: null,
    createdAt: '2026-01-01T00:00:00Z', startedAt: '2026-01-01T00:00:00Z', finishedAt: '2026-01-01T00:00:01Z' }],
  createdAt: '2026-01-01T00:00:00Z', updatedAt: '2026-01-01T00:00:01Z', finishedAt: status === 'running' ? null : '2026-01-01T00:00:01Z',
})
const message = (state: string, outputDatasetIds: string[] = []): WorkflowProgressMessage => ({
  id: 'progress', type: 'workflow-progress', timestamp: '2026-01-01T00:00:00Z', revision: 1,
  card: { workflowId: '9', state, executionIndex: 2, executionMax: 4, repairIndex: 1, repairMax: 3,
    elapsedMs: 1000, attempts: [], scriptId: '7', outputDatasetIds, failureCode: state === 'failed' ? 'repair_exhausted' : undefined },
})

describe('WorkflowProgressCard', () => {
  beforeEach(() => { vi.clearAllMocks(); vi.mocked(listRunDatasets).mockResolvedValue([]) })

  it('keeps attempts collapsed and reveals only sanitized diagnostics on demand', async () => {
    vi.mocked(getManagedWorkflow).mockResolvedValue(workflow('running'))
    render(<WorkflowProgressCard message={message('running')} notify={vi.fn()} />)
    const summary = await screen.findByText('执行尝试（1）')
    expect(screen.queryByText('unknown function')).not.toBeInTheDocument()
    fireEvent.click(summary)
    expect(screen.getByText('unknown function')).toBeInTheDocument()
    expect(screen.getByText(/退出码 2/)).toBeInTheDocument()
  })

  it('cancels once while an idempotent request is pending', async () => {
    vi.mocked(getManagedWorkflow).mockResolvedValue(workflow('running'))
    let resolve!: (value: ManagedWorkflowView) => void
    vi.mocked(cancelManagedWorkflow).mockReturnValue(new Promise(value => { resolve = value }))
    render(<WorkflowProgressCard message={message('running')} notify={vi.fn()} />)
    const button = await screen.findByRole('button', { name: '取消' })
    fireEvent.click(button)
    fireEvent.click(button)
    expect(cancelManagedWorkflow).toHaveBeenCalledTimes(1)
    resolve(workflow('cancelled_by_user'))
    await waitFor(() => expect(screen.queryByRole('button', { name: '取消' })).not.toBeInTheDocument())
  })

  it('lists successful datasets without opening them automatically', async () => {
    vi.mocked(getManagedWorkflow).mockResolvedValue({ ...workflow('succeeded'), outputDatasetIds: ['21', '22'] })
    const changed = vi.fn()
    render(<WorkflowProgressCard message={message('succeeded', ['21', '22'])} onChanged={changed} notify={vi.fn()} />)
    expect(await screen.findByText('结果数据集 #21')).toBeInTheDocument()
    expect(screen.getByText('结果数据集 #22')).toBeInTheDocument()
    expect(screen.getByText(/1\.0 秒/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /查看/ })).not.toBeInTheDocument()
    await waitFor(() => expect(changed).toHaveBeenCalledTimes(1))
  })

  it('shows all live run results after success and opens the selected dataset', async () => {
    vi.mocked(getManagedWorkflow).mockResolvedValue(workflow('succeeded'))
    vi.mocked(listRunDatasets).mockResolvedValue(['电压', '功率'].map((name, index) => ({
      id: String(21 + index), runId: '8', analysisRunId: null, name, rowCount: 12,
      sizeBytes: 100, columns: [{ name: 'value', dtype: 'Float64', nullable: false }],
      views: [], schemaHash: 'hash', lineage: [],
    })))
    const open = vi.fn()
    render(<WorkflowProgressCard message={message('succeeded')} notify={vi.fn()} onOpenDataset={open} />)
    fireEvent.click(await screen.findByRole('button', { name: '查看 功率' }))
    expect(open).toHaveBeenCalledWith('22')
    expect(screen.getByRole('button', { name: '查看 电压' })).toBeInTheDocument()
    expect(screen.getAllByText('12 行 · 1 列')).toHaveLength(2)
    expect(listRunDatasets).toHaveBeenCalledWith('8')
  })

  it('retries failed result loading without rerunning the workflow', async () => {
    vi.mocked(getManagedWorkflow).mockResolvedValue(workflow('succeeded'))
    vi.mocked(listRunDatasets).mockRejectedValueOnce(new Error('结果读取失败')).mockResolvedValueOnce([])
    render(<WorkflowProgressCard message={message('succeeded')} notify={vi.fn()} onOpenDataset={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: '重试读取结果' }))
    expect(await screen.findByText('本次运行暂无可用结果数据集')).toBeInTheDocument()
    expect(listRunDatasets).toHaveBeenCalledTimes(2)
  })

  it('does not load results before success', async () => {
    vi.mocked(getManagedWorkflow).mockResolvedValue(workflow('running'))
    render(<WorkflowProgressCard message={message('running')} notify={vi.fn()} onOpenDataset={vi.fn()} />)
    await screen.findByText('执行尝试（1）')
    expect(listRunDatasets).not.toHaveBeenCalled()
  })

  it('uses 重新尝试 for terminal failure and never labels it 运行', async () => {
    vi.mocked(getManagedWorkflow).mockResolvedValue(workflow('failed'))
    const retry = vi.fn().mockResolvedValue(undefined)
    render(<WorkflowProgressCard message={message('failed')} onRetry={retry} notify={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: '重新尝试' }))
    await waitFor(() => expect(retry).toHaveBeenCalledWith('7'))
    expect(screen.queryByRole('button', { name: '运行' })).not.toBeInTheDocument()
  })
})
