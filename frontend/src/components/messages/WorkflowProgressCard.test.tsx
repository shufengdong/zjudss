import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { cancelManagedWorkflow, getManagedWorkflow } from '../../api/client'
import type { ManagedWorkflowView, WorkflowProgressMessage } from '../../types'
import WorkflowProgressCard from './WorkflowProgressCard'

vi.mock('../../api/client', () => ({ cancelManagedWorkflow: vi.fn(), getManagedWorkflow: vi.fn() }))

const workflow = (status: string): ManagedWorkflowView => ({
  id: 9, sessionId: 1, configurationId: 2, configurationRevision: 3, sourceWorkflowId: null,
  status, correctionBudget: 3, correctionsUsed: 1, progressMessageId: 4, finalScriptResourceId: 7,
  runId: 8, scriptMessageId: 10, error: status === 'failed' ? { code: 'repair_exhausted', message: '三次修正后仍失败' } : null,
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
  beforeEach(() => vi.clearAllMocks())

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
    vi.mocked(getManagedWorkflow).mockResolvedValue(workflow('succeeded'))
    render(<WorkflowProgressCard message={message('succeeded', ['21', '22'])} notify={vi.fn()} />)
    expect(await screen.findByText('结果数据集 #21')).toBeInTheDocument()
    expect(screen.getByText('结果数据集 #22')).toBeInTheDocument()
    expect(screen.getByText(/1\.0 秒/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /查看/ })).not.toBeInTheDocument()
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
