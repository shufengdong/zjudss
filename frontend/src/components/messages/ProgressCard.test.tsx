import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getRun } from '../../api/client'
import type { ProgressMessage, SimulationRun } from '../../types'
import ProgressCard from './ProgressCard'

vi.mock('../../api/client', () => ({ getRun: vi.fn() }))

const message = (kind: 'ingestion' | 'run' | 'analysis' = 'run', failed = true): ProgressMessage => ({
  id: 'progress-1', type: 'progress', timestamp: '2026-01-01T00:00:00Z',
  operation: { kind, id: '9' },
  stages: [
    { key: 'prepare', label: '准备输入', status: 'done' },
    { key: 'run', label: '执行计算', status: failed ? 'failed' : 'running' },
  ],
})

const run = (error: unknown): SimulationRun => ({
  id: '9', sessionId: '1', modelVersionId: '3', modelContentHash: null,
  scriptResourceId: '4', scriptName: 'study.rss', scriptArtifactId: '5', scriptContentHash: null,
  scriptMessageId: null, managedWorkflowId: null, clientRunId: 'client-9', status: 'failed', stage: 'run',
  engineContractVersion: 'v4', agentBundleVersion: null, agentSkillRefs: [], exitCode: 7, error,
  createdAt: '2026-01-01T00:00:00Z', startedAt: '2026-01-01T00:00:01Z', finishedAt: '2026-01-01T00:00:02Z',
})

const modernRun = run({
  code: 'run_failed', message: '仿真引擎执行失败', stage: 'run',
  engineLogs: [{ stream: 'stderr', text: 'parse failed', totalBytes: 12, sentBytes: 12, truncated: false }],
  analysisStatus: 'complete',
  analysis: {
    summary: '脚本解析失败', facts: [], possibleCauses: [], unknowns: [], recommendations: [],
  },
})

describe('ProgressCard', () => {
  beforeEach(() => vi.mocked(getRun).mockReset())
  afterEach(() => vi.useRealTimers())

  it('loads_and_shows_failed_run_logs_and_analysis_inline', async () => {
    vi.mocked(getRun).mockResolvedValue(modernRun)
    render(<ProgressCard message={message()} />)

    expect(screen.getByText('准备输入')).toBeInTheDocument()
    expect(screen.getByText('执行计算')).toBeInTheDocument()
    expect(await screen.findByText('parse failed')).toBeInTheDocument()
    expect(screen.getByText('脚本解析失败')).toBeInTheDocument()
    expect(getRun).toHaveBeenCalledWith('9')
  })

  it('renders_legacy_engine_output_without_polling', async () => {
    vi.useFakeTimers()
    vi.mocked(getRun).mockResolvedValue(run({
      code: 'run_failed', message: '旧运行失败', engineOutput: 'legacy stderr',
    }))
    render(<ProgressCard message={message()} />)
    await act(async () => { await Promise.resolve() })
    expect(screen.getByText('legacy stderr')).toBeInTheDocument()
    expect(screen.getByText('智能分析暂不可用')).toBeInTheDocument()
    await act(async () => { vi.advanceTimersByTime(1_500); await Promise.resolve() })
    expect(getRun).toHaveBeenCalledTimes(1)
  })

  it('keeps_stage_list_when_run_detail_loading_fails_and_retries', async () => {
    vi.mocked(getRun).mockRejectedValueOnce(new Error('详情读取失败')).mockResolvedValueOnce(modernRun)
    render(<ProgressCard message={message()} />)

    expect(await screen.findByRole('alert')).toHaveTextContent('详情读取失败')
    expect(screen.getByText('准备输入')).toBeInTheDocument()
    expect(screen.getByText('执行计算')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '重试读取失败详情' }))
    expect(await screen.findByText('parse failed')).toBeInTheDocument()
    expect(getRun).toHaveBeenCalledTimes(2)
  })

  it('never_loads_run_detail_for_non_run_or_nonfailed_progress', async () => {
    const { rerender } = render(<ProgressCard message={message('ingestion')} />)
    rerender(<ProgressCard message={message('analysis')} />)
    rerender(<ProgressCard message={message('run', false)} />)
    await act(async () => { await Promise.resolve() })
    expect(getRun).not.toHaveBeenCalled()
  })
})
