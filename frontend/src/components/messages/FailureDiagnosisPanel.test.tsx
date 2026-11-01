import { fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { FailureDiagnosis } from '../../types'
import FailureDiagnosisPanel from './FailureDiagnosisPanel'

const diagnosis = (overrides: Partial<FailureDiagnosis> = {}): FailureDiagnosis => ({
  code: 'run_failed',
  message: '仿真引擎执行失败',
  stage: 'run',
  exitCode: 7,
  durationMs: 1_234,
  engineLogs: [
    { stream: 'stdout', text: 'engine banner', totalBytes: 13, sentBytes: 13, truncated: false },
    { stream: 'stderr', text: 'line one\nline two', totalBytes: 99, sentBytes: 17, truncated: true },
  ],
  analysisStatus: 'complete',
  analysis: {
    summary: '脚本解析失败',
    facts: [{ statement: '引擎报告解析失败', evidence: 'stderr: line one' }],
    possibleCauses: [{ statement: '当前脚本可能存在语法错误', evidence: 'stderr: line two' }],
    unknowns: ['具体语法位置'],
    recommendations: ['核对脚本语法'],
  },
  ...overrides,
})

describe('FailureDiagnosisPanel', () => {
  const blobs: Blob[] = []

  beforeEach(() => {
    blobs.length = 0
    vi.stubGlobal('URL', {
      ...URL,
      createObjectURL: vi.fn((blob: Blob) => { blobs.push(blob); return 'blob:diagnosis' }),
      revokeObjectURL: vi.fn(),
    })
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined)
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('shows logs, metadata, truncation and all structured analysis sections', () => {
    render(<FailureDiagnosisPanel diagnosis={diagnosis()} downloadPrefix="run-9" />)

    expect(screen.getByRole('heading', { name: '失败诊断' })).toBeInTheDocument()
    expect(screen.getByText('仿真引擎执行失败')).toBeInTheDocument()
    expect(screen.getByText(/错误码 run_failed/)).toBeInTheDocument()
    expect(screen.getByText(/阶段 run/)).toBeInTheDocument()
    expect(screen.getByText(/退出码 7/)).toBeInTheDocument()
    expect(screen.getByText(/1\.2 秒/)).toBeInTheDocument()
    expect(screen.getByText((_, node) => node?.tagName === 'PRE' && node.textContent === 'engine banner')).toBeInTheDocument()
    expect(screen.getByText((_, node) => node?.tagName === 'PRE' && node.textContent === 'line one\nline two')).toBeInTheDocument()
    expect(screen.getByText('日志已截断；下载内容也是当前已保留的脱敏片段。')).toBeInTheDocument()
    for (const text of ['脚本解析失败', '引擎报告解析失败', '当前脚本可能存在语法错误', '具体语法位置', '核对脚本语法']) {
      expect(screen.getByText(text)).toBeInTheDocument()
    }
  })

  it('downloads exactly the displayed sanitized text with a safe filename', async () => {
    render(<FailureDiagnosisPanel diagnosis={diagnosis()} downloadPrefix="run 9/private" />)
    const anchor = document.createElement('a')
    const createElement = vi.spyOn(document, 'createElement').mockReturnValue(anchor)

    fireEvent.click(screen.getByRole('button', { name: '下载 stderr 脱敏日志' }))

    expect(createElement).toHaveBeenCalledWith('a')
    expect(anchor.download).toBe('run-9-private-stderr-sanitized.log')
    expect(blobs).toHaveLength(1)
    await expect(blobs[0].text()).resolves.toBe('line one\nline two')
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:diagnosis')
  })

  it('distinguishes pending, unavailable and missing engine output', () => {
    const { rerender } = render(<FailureDiagnosisPanel diagnosis={diagnosis({
      engineLogs: [], analysisStatus: 'pending', analysis: null,
    })} downloadPrefix="run-9" />)
    expect(screen.getByText('本次失败没有可用的引擎输出。')).toBeInTheDocument()
    expect(screen.getByText('正在生成智能分析')).toBeInTheDocument()

    rerender(<FailureDiagnosisPanel diagnosis={diagnosis({
      engineLogs: [], analysisStatus: 'unavailable', analysis: null,
    })} downloadPrefix="run-9" />)
    expect(screen.getByText('智能分析暂不可用')).toBeInTheDocument()
    expect(screen.queryByText('正在生成智能分析')).not.toBeInTheDocument()
  })
})
