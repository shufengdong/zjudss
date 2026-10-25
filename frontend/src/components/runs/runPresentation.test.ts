import { describe, expect, it } from 'vitest'
import type { SimulationRun } from '../../types'
import {
  formatBytes,
  formatDuration,
  runDurationMs,
  runFailure,
  runStageLabel,
  runStatusLabel,
  runTitle,
} from './runPresentation'

const run = (overrides: Partial<SimulationRun> = {}): SimulationRun => ({
  id: '9', sessionId: '1', modelVersionId: '3', modelContentHash: 'm'.repeat(64),
  scriptResourceId: '4', scriptName: 'manual-study.rss', scriptArtifactId: '5',
  scriptContentHash: 's'.repeat(64), clientRunId: 'client', status: 'succeeded',
  stage: 'completed', engineContractVersion: 'v3', agentBundleVersion: null,
  agentSkillRefs: [], exitCode: 0, error: null,
  createdAt: '2026-01-01T00:00:00.000Z', startedAt: '2026-01-01T00:00:01.000Z',
  finishedAt: '2026-01-01T00:00:03.500Z', ...overrides,
})

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
    expect(runTitle(run({ agentSkillRefs: [{ kind: 'study-plan', function: studyFunction }] }))).toBe(title)
  })

  it('falls back to the immutable script name for manual or malformed audit data', () => {
    expect(runTitle(run())).toBe('manual-study.rss')
    expect(runTitle(run({ agentSkillRefs: [{ kind: 'study-plan', function: 'unknown' }] }))).toBe('manual-study.rss')
    expect(runTitle(run({ agentSkillRefs: { function: 'ds_fault_study' } }))).toBe('manual-study.rss')
  })

  it('localizes statuses and execution stages without guessing unknown values', () => {
    expect(['排队中', '运行中', '成功', '失败']).toEqual(
      ['queued', 'running', 'succeeded', 'failed'].map(runStatusLabel),
    )
    expect(['准备输入', '执行计算', '收集结果', '已完成']).toEqual(
      ['prepare', 'execute', 'collect', 'completed'].map(runStageLabel),
    )
    expect(runStageLabel('engine-step')).toBe('engine-step')
    expect(runStageLabel('')).toBe('未知阶段')
  })

  it('derives terminal, running and queued durations from authoritative timestamps', () => {
    expect(runDurationMs(run(), Date.parse('2026-01-01T00:00:10Z'))).toBe(2_500)
    expect(runDurationMs(run({ status: 'running', finishedAt: null }), Date.parse('2026-01-01T00:00:04Z'))).toBe(3_000)
    expect(runDurationMs(run({ status: 'queued', startedAt: null, finishedAt: null }), Date.parse('2026-01-01T00:00:02Z'))).toBe(2_000)
    expect(runDurationMs(run({ finishedAt: '2025-12-31T23:59:59Z' }), Date.now())).toBeNull()
    expect(runDurationMs(run({ startedAt: 'not-a-date' }), Date.now())).toBeNull()
  })

  it('formats durations and byte sizes deterministically', () => {
    expect(formatDuration(null)).toBe('—')
    expect(formatDuration(800)).toBe('0.8 秒')
    expect(formatDuration(65_000)).toBe('1 分 5 秒')
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(1_024)).toBe('1 KiB')
    expect(formatBytes(1_536)).toBe('1.5 KiB')
  })

  it('renders only bounded public failure strings and otherwise uses a safe fallback', () => {
    expect(runFailure(run({ status: 'failed', error: { code: 'run_failed', message: '求解未收敛' } }))).toEqual({
      code: 'run_failed', message: '求解未收敛',
    })
    expect(runFailure(run({ status: 'failed', error: null })).message).toBe('运行失败，未提供可用原因')
    expect(runFailure(run({ status: 'failed', error: ['raw', 'diagnostics'] })).message).toBe('运行失败，未提供可用原因')
    const bounded = runFailure(run({ status: 'failed', error: { message: '错'.repeat(600) } })).message
    expect(Array.from(bounded)).toHaveLength(501)
    expect(bounded.endsWith('…')).toBe(true)
  })
})
