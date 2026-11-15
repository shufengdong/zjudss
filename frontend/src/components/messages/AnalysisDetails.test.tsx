import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getAnalysis } from '../../api/client'
import type { AnalysisRun } from '../../types'
import AnalysisDetails from './AnalysisDetails'

vi.mock('../../api/client', () => ({ getAnalysis: vi.fn() }))

function details(id = '9', script = 'combined = limit(1, active_bus);'): AnalysisRun {
  return {
    id,
    sessionId: '1',
    status: 'succeeded',
    stage: 'present',
    intent: 'combine sources',
    scriptArtifactId: '41',
    script: { content: script, contentHash: 'script-hash' },
    inputs: [
      {
        sourceKind: 'active_model', modelVersionId: '7', modelDataframeId: '11', alias: 'active_bus', ordinal: 0,
        sourceName: 'bus', sourceSchema: [], sourceSchemaHash: 'schema-model', sourceContentHash: 'content-model', sourceProducer: {},
      },
      {
        sourceKind: 'dataset', datasetId: '22', alias: 'prior_result', ordinal: 1,
        sourceName: 'bus_voltage', sourceSchema: [], sourceSchemaHash: 'schema-result', sourceContentHash: 'content-result', sourceProducer: {},
      },
      {
        sourceKind: 'data_resource', resourceId: 'resource-3', alias: 'local_data', ordinal: 2,
        sourceName: '本地量测', sourceSchema: [], sourceSchemaHash: 'schema-local', sourceContentHash: 'content-local', sourceProducer: {},
      },
    ],
    agentBundleVersion: 'v4.5.0',
    agentSkillRefs: [],
    engineContractVersion: 'tensoreval-parquet-v3',
    engineResourceSnapshotId: 'snapshot-1',
    error: null,
    createdAt: '2026-11-15T00:00:00Z',
    startedAt: '2026-11-15T00:00:01Z',
    finishedAt: '2026-11-15T00:00:02Z',
    datasets: [],
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((accept, decline) => { resolve = accept; reject = decline })
  return { promise, resolve, reject }
}

describe('AnalysisDetails', () => {
  beforeEach(() => vi.clearAllMocks())

  it('loads only after expansion and renders immutable sources in ordinal order', async () => {
    vi.mocked(getAnalysis).mockResolvedValue(details())
    render(<AnalysisDetails analysisRunId="9" notify={vi.fn()} />)

    expect(getAnalysis).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: '查看分析脚本与输入' }))
    expect(screen.getByRole('status')).toHaveTextContent('正在读取分析详情')
    expect(await screen.findByLabelText('分析 RSS 脚本')).toHaveTextContent('combined = limit')
    expect(getAnalysis).toHaveBeenCalledWith('9', expect.any(AbortSignal))
    const inputs = screen.getAllByTestId('analysis-input')
    expect(inputs.map(input => input.textContent)).toEqual([
      expect.stringContaining('active 模型'),
      expect.stringContaining('结果数据集'),
      expect.stringContaining('本地数据'),
    ])
    expect(inputs[0]).toHaveTextContent('modelDataframeId 11')
    expect(inputs[1]).toHaveTextContent('datasetId 22')
    expect(inputs[2]).toHaveTextContent('resourceId resource-3')
    for (const action of ['编辑', '运行', '下载']) {
      expect(screen.queryByRole('button', { name: new RegExp(action) })).not.toBeInTheDocument()
    }
  })

  it('isolates failures and retries only this detail request', async () => {
    const notify = vi.fn()
    vi.mocked(getAnalysis)
      .mockRejectedValueOnce(new Error('详情暂时不可用'))
      .mockResolvedValueOnce(details())
    render(<AnalysisDetails analysisRunId="9" notify={notify} />)

    fireEvent.click(screen.getByRole('button', { name: '查看分析脚本与输入' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('详情暂时不可用')
    expect(notify).toHaveBeenCalledWith('详情暂时不可用')
    fireEvent.click(screen.getByRole('button', { name: '重试读取分析详情' }))
    expect(await screen.findByLabelText('分析 RSS 脚本')).toBeInTheDocument()
    expect(getAnalysis).toHaveBeenCalledTimes(2)
  })

  it('ignores late responses after id change or unmount', async () => {
    const first = deferred<AnalysisRun>()
    const second = deferred<AnalysisRun>()
    const notify = vi.fn()
    vi.mocked(getAnalysis).mockImplementation(id => id === '1' ? first.promise : second.promise)
    const view = render(<AnalysisDetails analysisRunId="1" notify={notify} />)
    fireEvent.click(screen.getByRole('button', { name: '查看分析脚本与输入' }))
    view.rerender(<AnalysisDetails analysisRunId="2" notify={notify} />)
    await waitFor(() => expect(getAnalysis).toHaveBeenCalledWith('2', expect.any(AbortSignal)))

    first.resolve(details('1', 'old_result = limit(1, source);'))
    await Promise.resolve()
    expect(screen.queryByText(/old_result/)).not.toBeInTheDocument()
    second.resolve(details('2', 'new_result = limit(1, source);'))
    expect(await screen.findByText(/new_result/)).toBeInTheDocument()

    const late = deferred<AnalysisRun>()
    vi.mocked(getAnalysis).mockReturnValueOnce(late.promise)
    view.rerender(<AnalysisDetails analysisRunId="3" notify={notify} />)
    await waitFor(() => expect(getAnalysis).toHaveBeenCalledWith('3', expect.any(AbortSignal)))
    view.unmount()
    late.reject(new Error('late failure'))
    await Promise.resolve()
    expect(notify).not.toHaveBeenCalledWith('late failure')
  })
})
