import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cancelManagedWorkflow, getManagedWorkflow, listRunDatasets } from '../../api/client'
import type { ManagedWorkflowView, ScriptMessage, WorkflowProgressMessage } from '../../types'
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
const linkedScript: ScriptMessage = {
  id: '10', type: 'script', timestamp: '2026-01-01T00:00:01Z',
  script: { id: '7', sessionId: '1', name: 'managed-workflow-9.rss', sourceKind: 'agent',
    contentHash: 'abc', validationStatus: 'valid', requiredAliases: [], outputs: [], diagnostics: {},
    preview: 'voltage = ds_newton_raphson_pf(...);', previewTruncated: false, canRun: true,
    disabledReason: null, createdAt: '2026-01-01T00:00:01Z' },
}

describe('WorkflowProgressCard', () => {
  afterEach(() => vi.useRealTimers())
  beforeEach(() => { vi.clearAllMocks(); vi.mocked(listRunDatasets).mockResolvedValue([]) })

  it('keeps loaded progress visible without a loading row during background polling', async () => {
    vi.useFakeTimers()
    let resolve!: (value: ManagedWorkflowView) => void
    vi.mocked(getManagedWorkflow).mockReturnValueOnce(new Promise(value => { resolve = value }))
      .mockReturnValue(new Promise(() => {}))
    render(<WorkflowProgressCard message={message('running')} notify={vi.fn()} />)
    expect(screen.getByText('读取详细进度…')).toBeInTheDocument()
    await act(async () => { resolve(workflow('running')) })
    expect(screen.queryByText('读取详细进度…')).not.toBeInTheDocument()
    await act(async () => { vi.advanceTimersByTime(750) })
    expect(getManagedWorkflow).toHaveBeenCalledTimes(2)
    expect(screen.getByText('执行尝试（1）')).toBeInTheDocument()
    expect(screen.queryByText('读取详细进度…')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '取消' })).toBeInTheDocument()
  })

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

  it('renders the linked script before run results in one highlighted research outcome', async () => {
    vi.mocked(getManagedWorkflow).mockResolvedValue(workflow('succeeded'))
    vi.mocked(listRunDatasets).mockResolvedValue([{
      id: '21', runId: '8', analysisRunId: null, name: '电压', rowCount: 12,
      sizeBytes: 100, columns: [{ name: 'value', dtype: 'Float64', nullable: false }],
      views: [], schemaHash: 'hash', lineage: [],
    }])
    const run = vi.fn().mockResolvedValue(undefined)

    render(<WorkflowProgressCard message={message('succeeded')} notify={vi.fn()}
      onOpenDataset={vi.fn()} linkedScript={linkedScript} onRunScript={run} highlightedMessageId="10" />)

    const outcome = await screen.findByRole('region', { name: '研究成果' })
    const scriptAnchor = within(outcome).getByTestId('message-script')
    const resultTitle = await within(outcome).findByText('运行结果')
    expect(scriptAnchor).toHaveAttribute('data-message-id', '10')
    expect(scriptAnchor).toHaveClass('ring-2', 'ring-brand-400')
    expect(scriptAnchor.compareDocumentPosition(resultTitle) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    fireEvent.click(within(scriptAnchor).getByRole('button', { name: '运行' }))
    await waitFor(() => expect(run).toHaveBeenCalledWith(linkedScript.script))
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

  it('shows sanitized engine logs and structured AI analysis directly on failure', async () => {
    const failed = workflow('failed')
    failed.attempts = [{
      ...failed.attempts[0],
      durationMs: 1234,
      engineLogs: [{ stream: 'stderr', text: 'line one\nline two', totalBytes: 17, sentBytes: 17, truncated: false }],
      failureAnalysis: {
        summary: '所选潮流算法拒绝了当前网络拓扑',
        facts: [{ statement: '引擎明确报告拓扑不受支持', evidence: 'stderr: network topology is unsupported' }],
        possibleCauses: [{ statement: '算法与拓扑约束可能不兼容', evidence: 'stderr: selected power-flow method' }],
        unknowns: ['当前网络是否包含算法不支持的结构'],
        recommendations: ['核对算法的拓扑适用条件'],
      },
      scriptChangedFromParent: null,
    }, {
      ...failed.attempts[0], id: 2, attemptNumber: 2, parentAttemptId: 1, phase: 'repair',
      scriptChangedFromParent: false,
    }] as unknown as ManagedWorkflowView['attempts']
    vi.mocked(getManagedWorkflow).mockResolvedValue(failed)

    render(<WorkflowProgressCard message={message('failed')} notify={vi.fn()} />)

    expect(await screen.findByRole('heading', { name: '失败诊断' })).toBeInTheDocument()
    expect(screen.getByText((_, element) => element?.tagName === 'PRE' && element.textContent === 'line one\nline two')).toBeInTheDocument()
    expect(screen.getByText('所选潮流算法拒绝了当前网络拓扑')).toBeInTheDocument()
    expect(screen.getByText('引擎明确报告拓扑不受支持')).toBeInTheDocument()
    expect(screen.getByText('算法与拓扑约束可能不兼容')).toBeInTheDocument()
    expect(screen.getByText('当前网络是否包含算法不支持的结构')).toBeInTheDocument()
    expect(screen.getByText('核对算法的拓扑适用条件')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '下载 stderr 脱敏日志' })).toBeInTheDocument()
    const history = screen.getByRole('button', { name: '自动修正记录（2）' })
    expect(history).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByText('未产生实质修改')).not.toBeInTheDocument()
    fireEvent.click(history)
    expect(screen.getByText('未产生实质修改')).toBeInTheDocument()
    expect(screen.getAllByText('依据：所选潮流算法拒绝了当前网络拓扑')).toHaveLength(2)
  })

  it('keeps logs visible when AI analysis is unavailable', async () => {
    const failed = workflow('failed')
    failed.attempts = [{
      ...failed.attempts[0],
      engineLogs: [{ stream: 'stderr', text: 'solver failed', totalBytes: 13, sentBytes: 13, truncated: false }],
      failureAnalysis: null,
      scriptChangedFromParent: null,
    }] as unknown as ManagedWorkflowView['attempts']
    vi.mocked(getManagedWorkflow).mockResolvedValue(failed)

    render(<WorkflowProgressCard message={message('failed')} notify={vi.fn()} />)

    expect(await screen.findByText('solver failed')).toBeInTheDocument()
    expect(screen.getByText('智能分析暂不可用')).toBeInTheDocument()
  })
})
