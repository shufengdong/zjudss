import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { listRunDatasets } from '../../api/client'
import type { DatasetMetadata, SimulationRun } from '../../types'
import type { RunHistoryState } from '../../hooks/useRunHistory'
import RunHistory from './RunHistory'

vi.mock('../../api/client', () => ({ listRunDatasets: vi.fn() }))

const mockedListRunDatasets = vi.mocked(listRunDatasets)
const makeRun = (overrides: Partial<SimulationRun> = {}): SimulationRun => ({
  id: '9', sessionId: '1', modelVersionId: '3', modelContentHash: 'm'.repeat(64),
  scriptResourceId: '4', scriptName: 'long-script.rss', scriptArtifactId: '5',
  scriptContentHash: 's'.repeat(64), clientRunId: 'client-9', status: 'succeeded',
  stage: 'completed', engineContractVersion: 'tensoreval-parquet-v3', agentBundleVersion: null,
  agentSkillRefs: [{ kind: 'study-plan', function: 'ds_radial_pf' }], exitCode: 0, error: null,
  createdAt: '2026-01-01T00:00:00Z', startedAt: '2026-01-01T00:00:01Z',
  finishedAt: '2026-01-01T00:00:03Z', ...overrides,
})
const makeDataset = (id: string, name: string, rowCount: number, sizeBytes: number): DatasetMetadata => ({
  id, runId: '9', analysisRunId: null, name, rowCount, sizeBytes, columns: [], views: [],
  schemaHash: 'h'.repeat(64), lineage: [],
})
const state = (runs: SimulationRun[], overrides: Partial<RunHistoryState> = {}): RunHistoryState => ({
  runs, status: 'ready', error: null, reload: vi.fn(), ...overrides,
})
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((accept) => { resolve = accept })
  return { promise, resolve }
}

describe('RunHistory', () => {
  beforeEach(() => mockedListRunDatasets.mockReset())

  it('renders an informative collapsed summary without requesting results or offering deletion', () => {
    render(<RunHistory history={state([makeRun()])} datasetRevision={0} onOpenDataset={vi.fn()} />)
    const header = screen.getByRole('button', { name: /静态潮流/ })
    expect(header).toHaveAttribute('aria-expanded', 'false')
    expect(header).toHaveTextContent('成功')
    expect(header).toHaveTextContent('long-script.rss')
    expect(header).toHaveTextContent('模型版本 3')
    expect(header).toHaveTextContent('耗时 2 秒')
    expect(screen.queryByText(/删除/)).not.toBeInTheDocument()
    expect(mockedListRunDatasets).not.toHaveBeenCalled()
  })

  it('loads every result only after expansion and opens the selected dataset', async () => {
    mockedListRunDatasets.mockResolvedValue([
      makeDataset('21', 'voltage', 128, 24_678),
      makeDataset('22', 'loss', 8, 3_277),
    ])
    const onOpenDataset = vi.fn()
    render(<RunHistory history={state([makeRun()])} datasetRevision={0} onOpenDataset={onOpenDataset} />)
    fireEvent.click(screen.getByRole('button', { name: /静态潮流/ }))
    expect(screen.getByText('正在加载计算结果…')).toBeInTheDocument()
    expect(await screen.findByText('计算结果（2）')).toBeInTheDocument()
    expect(screen.getByText('voltage')).toBeInTheDocument()
    expect(screen.getByText('128 行')).toBeInTheDocument()
    expect(screen.getByText('24.1 KiB')).toBeInTheDocument()
    expect(screen.getByText('loss')).toBeInTheDocument()
    expect(screen.getByText('8 行')).toBeInTheDocument()
    expect(screen.getByText('3.2 KiB')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '查看 loss' }))
    expect(onOpenDataset).toHaveBeenCalledWith('22')
  })

  it('shows failed and running details without requesting result datasets', () => {
    const failed = makeRun({
      id: '10', status: 'failed', stage: 'execute', exitCode: 7,
      error: { code: 'engine_failed', message: '求解未收敛' },
      finishedAt: '2026-01-01T00:00:01.700Z',
    })
    const running = makeRun({
      id: '11', status: 'running', stage: 'collect', exitCode: null,
      startedAt: '2026-01-01T00:00:01Z', finishedAt: null,
    })
    render(<RunHistory history={state([failed, running])} datasetRevision={0} onOpenDataset={vi.fn()} />)
    fireEvent.click(screen.getAllByRole('button', { name: /静态潮流/ })[0])
    expect(screen.getByRole('alert')).toHaveTextContent('失败阶段：执行计算')
    expect(screen.getByRole('alert')).toHaveTextContent('失败原因：求解未收敛')
    expect(screen.getByRole('alert')).toHaveTextContent('错误码：engine_failed')
    expect(screen.getByRole('alert')).toHaveTextContent('退出码：7')
    expect(screen.getByRole('alert')).toHaveTextContent('耗时：0.7 秒')
    fireEvent.click(screen.getAllByRole('button', { name: /静态潮流/ })[1])
    expect(screen.getByText('当前阶段：收集结果')).toBeInTheDocument()
    expect(screen.getByText('计算完成后可查看结果')).toBeInTheDocument()
    expect(mockedListRunDatasets).not.toHaveBeenCalled()
  })

  it('preserves expansion and loads results when a running record succeeds', async () => {
    mockedListRunDatasets.mockResolvedValue([makeDataset('23', 'completed-result', 2, 200)])
    const running = makeRun({ status: 'running', stage: 'execute', finishedAt: null })
    const { rerender } = render(<RunHistory history={state([running])} datasetRevision={0} onOpenDataset={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /静态潮流/ }))
    expect(screen.getByText('计算完成后可查看结果')).toBeInTheDocument()
    rerender(<RunHistory history={state([makeRun()])} datasetRevision={0} onOpenDataset={vi.fn()} />)
    expect(await screen.findByText('completed-result')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /静态潮流/ })).toHaveAttribute('aria-expanded', 'true')
  })

  it('distinguishes list failures and retries them', () => {
    const reload = vi.fn()
    render(<RunHistory history={state([], { status: 'error', error: '请求失败（503）', reload })} datasetRevision={0} onOpenDataset={vi.fn()} />)
    expect(screen.getByRole('alert')).toHaveTextContent('运行记录加载失败：请求失败（503）')
    fireEvent.click(screen.getByRole('button', { name: '重试加载运行记录' }))
    expect(reload).toHaveBeenCalledOnce()
  })

  it('shows a result error and retries that run only', async () => {
    mockedListRunDatasets.mockRejectedValueOnce(new Error('请求失败（502）'))
      .mockResolvedValueOnce([makeDataset('22', 'loss', 8, 3_277)])
    render(<RunHistory history={state([makeRun()])} datasetRevision={0} onOpenDataset={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /静态潮流/ }))
    expect(await screen.findByRole('alert')).toHaveTextContent('结果加载失败：请求失败（502）')
    fireEvent.click(screen.getByRole('button', { name: '重试加载计算结果' }))
    expect(await screen.findByText('loss')).toBeInTheDocument()
    expect(mockedListRunDatasets).toHaveBeenCalledTimes(2)
  })

  it('refreshes expanded results after deletion and ignores the obsolete response', async () => {
    const oldRequest = deferred<DatasetMetadata[]>()
    const refreshedRequest = deferred<DatasetMetadata[]>()
    mockedListRunDatasets.mockReturnValueOnce(oldRequest.promise).mockReturnValueOnce(refreshedRequest.promise)
    const history = state([makeRun()])
    const { rerender } = render(<RunHistory history={history} datasetRevision={0} onOpenDataset={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /静态潮流/ }))
    rerender(<RunHistory history={history} datasetRevision={1} onOpenDataset={vi.fn()} />)
    act(() => refreshedRequest.resolve([makeDataset('22', 'current-result', 1, 100)]))
    expect(await screen.findByText('current-result')).toBeInTheDocument()
    act(() => oldRequest.resolve([makeDataset('21', 'deleted-result', 1, 100)]))
    await waitFor(() => expect(screen.queryByText('deleted-result')).not.toBeInTheDocument())
    expect(screen.getByText('current-result')).toBeInTheDocument()
  })
})
