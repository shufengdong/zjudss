import { describe, expect, it } from 'vitest'
import type { SimulationRun } from '../../types'
import { simulationRun } from '../../test/fixtures'
import {
  formatBytes,
  formatDuration,
  formatTimestamp,
  parseRunFailureDiagnosis,
  runDurationMs,
  runFailure,
  runStageLabel,
  runStatusLabel,
  runTitle,
} from './runPresentation'

describe('run presentation', () => {
  it.each([
    ['ds_radial_pf', '静态潮流'],
    ['ds_newton_raphson_pf', '静态潮流'],
    ['ds_radial_qsts', '时序潮流'],
    ['ds_newton_raphson_qsts', '时序潮流'],
    ['ds_fault_study', '短路计算'],
    ['ds_harmonic_study', '谐波分析'],
    ['ds_motor_dynamics', '电机暂态'],
  ])('maps governed function %s to %s', (studyFunction, title) => {
    expect(runTitle(simulationRun({ agentSkillRefs: [{ kind: 'study-plan', function: studyFunction }] }))).toBe(title)
  })

  it('falls back to the immutable script name for manual or malformed audit data', () => {
    expect(runTitle(simulationRun())).toBe('manual-study.rss')
    expect(runTitle(simulationRun({ agentSkillRefs: [{ kind: 'study-plan', function: 'unknown' }] }))).toBe('manual-study.rss')
    expect(runTitle(simulationRun({ agentSkillRefs: { function: 'ds_fault_study' } }))).toBe('manual-study.rss')
    expect(runTitle(simulationRun({ agentSkillRefs: [{ kind: 'study-plan', function: '__proto__' }] }))).toBe('manual-study.rss')
  })

  it('localizes statuses and execution stages without guessing unknown values', () => {
    const statuses: SimulationRun['status'][] = ['queued', 'running', 'succeeded', 'failed']
    expect(['排队中', '运行中', '成功', '失败']).toEqual(statuses.map(runStatusLabel))
    expect(['排队等待', '准备输入', '执行计算', '收集结果', '已完成', '未知失败阶段', '恢复中断']).toEqual(
      ['queued', 'prepare', 'run', 'collect', 'completed', 'failed', 'recovery'].map(runStageLabel),
    )
    expect(runStageLabel('engine-step')).toBe('engine-step')
    expect(runStageLabel('')).toBe('未知阶段')
    expect(runStatusLabel('unexpected' as SimulationRun['status'])).toBe('未知状态')
  })

  it('derives terminal, running and queued durations from authoritative timestamps', () => {
    expect(runDurationMs(simulationRun(), Date.parse('2026-01-01T00:00:10Z'))).toBe(2_500)
    expect(runDurationMs(simulationRun({ status: 'running', finishedAt: null }), Date.parse('2026-01-01T00:00:04Z'))).toBe(3_000)
    expect(runDurationMs(simulationRun({ status: 'queued', startedAt: null, finishedAt: null }), Date.parse('2026-01-01T00:00:02Z'))).toBe(2_000)
    expect(runDurationMs(simulationRun({ finishedAt: '2025-12-31T23:59:59Z' }), Date.now())).toBeNull()
    expect(runDurationMs(simulationRun({ startedAt: 'not-a-date' }), Date.now())).toBeNull()
  })

  it('formats durations and byte sizes deterministically', () => {
    expect(formatDuration(null)).toBe('—')
    expect(formatDuration(800)).toBe('0.8 秒')
    expect(formatDuration(65_000)).toBe('1 分 5 秒')
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(1_024)).toBe('1 KiB')
    expect(formatBytes(1_536)).toBe('1.5 KiB')
    expect(formatTimestamp(null)).toBe('—')
    expect(formatTimestamp('invalid')).toBe('—')
    expect(formatTimestamp('2026-01-01T00:00:00Z')).toBe(new Date('2026-01-01T00:00:00Z').toLocaleString())
  })

  it('renders only bounded public failure strings and otherwise uses a safe fallback', () => {
    expect(runFailure(simulationRun({ status: 'failed', error: {
      code: 'run_failed', message: '求解未收敛', engineOutput: 'line 1\nline 2',
    } }))).toEqual({
      code: 'run_failed', message: '求解未收敛', engineOutput: 'line 1\nline 2',
    })
    expect(runFailure(simulationRun({ status: 'failed', error: null })).message).toBe('运行失败，未提供可用原因')
    expect(runFailure(simulationRun({ status: 'failed', error: ['raw', 'diagnostics'] })).message).toBe('运行失败，未提供可用原因')
    const bounded = runFailure(simulationRun({ status: 'failed', error: { message: '错'.repeat(600) } })).message
    expect(Array.from(bounded)).toHaveLength(501)
    expect(bounded.endsWith('…')).toBe(true)
    const boundedOutput = runFailure(simulationRun({ status: 'failed', error: {
      message: '失败', engineOutput: '错'.repeat(17_000),
    } })).engineOutput!
    expect(Array.from(boundedOutput)).toHaveLength(16_385)
    expect(boundedOutput.endsWith('…')).toBe(true)
  })

  it('parses modern run diagnosis into the shared bounded shape', () => {
    const parsed = parseRunFailureDiagnosis(simulationRun({
      status: 'failed', exitCode: 7, stage: 'run', error: {
        code: 'run_failed', message: '仿真引擎执行失败', stage: 'run', durationMs: 999,
        engineLogs: [{ stream: 'stderr', text: 'parse failed', totalBytes: 12, sentBytes: 12, truncated: false }],
        analysisStatus: 'complete',
        analysis: {
          summary: '脚本解析失败',
          facts: [{ statement: '引擎报告解析失败', evidence: 'stderr: parse failed' }],
          possibleCauses: [], unknowns: ['具体语法位置'], recommendations: ['核对脚本语法'],
        },
      },
    }))

    expect(parsed).toEqual({
      code: 'run_failed', message: '仿真引擎执行失败', stage: 'run', exitCode: 7,
      durationMs: 2_500,
      engineLogs: [{ stream: 'stderr', text: 'parse failed', totalBytes: 12, sentBytes: 12, truncated: false }],
      analysisStatus: 'complete',
      analysis: {
        summary: '脚本解析失败',
        facts: [{ statement: '引擎报告解析失败', evidence: 'stderr: parse failed' }],
        possibleCauses: [], unknowns: ['具体语法位置'], recommendations: ['核对脚本语法'],
      },
    })
  })

  it('maps legacy engine output to one stderr log with unavailable analysis', () => {
    const parsed = parseRunFailureDiagnosis(simulationRun({ status: 'failed', error: {
      code: 'run_failed', message: '旧运行失败', engineOutput: 'legacy stderr',
    } }))

    expect(parsed.engineLogs).toEqual([{
      stream: 'stderr', text: 'legacy stderr', totalBytes: null, sentBytes: null, truncated: false,
    }])
    expect(parsed.analysisStatus).toBe('unavailable')
    expect(parsed.analysis).toBeNull()
  })
})
