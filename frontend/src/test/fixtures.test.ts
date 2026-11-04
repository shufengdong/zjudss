import { describe, expect, it } from 'vitest'
import { chartCapabilities, failureDiagnosis, simulationRun, uploadBatch } from './fixtures'

describe('shared DTO fixtures', () => {
  it('creates independent nested arrays and objects for every call', () => {
    const first = simulationRun()
    const second = simulationRun()
    ;(first.agentSkillRefs as unknown[]).push({ kind: 'study-plan', function: 'changed' })
    expect(second.agentSkillRefs).toEqual([])

    const one = uploadBatch()
    const two = uploadBatch()
    one.candidates.push({ id: 'x' } as never)
    ;(one.parseSettings as { encoding?: string }).encoding = 'gbk'
    expect(two.candidates).toEqual([])
    expect(two.parseSettings).toEqual({})

    const diagnosisOne = failureDiagnosis()
    const diagnosisTwo = failureDiagnosis()
    diagnosisOne.engineLogs.push({
      stream: 'stderr', text: 'changed', totalBytes: 7, sentBytes: 7, truncated: false,
    })
    diagnosisOne.analysis?.unknowns.push('changed')
    expect(diagnosisTwo.engineLogs).toHaveLength(2)
    expect(diagnosisTwo.analysis?.unknowns).toEqual(['具体语法位置'])

    const capabilitiesOne = chartCapabilities()
    const capabilitiesTwo = chartCapabilities()
    capabilitiesOne.fields[1]?.shape.kind === 'list'
      && capabilitiesOne.fields[1].shape.observedPaths.push([9])
    capabilitiesOne.limits.maxSeries = 1
    expect(capabilitiesTwo.fields[1]?.shape).toMatchObject({ observedPaths: [[0], [1], [2]] })
    expect(capabilitiesTwo.limits.maxSeries).toBe(100)
  })

  it('applies only top-level overrides', () => {
    const replacementError = { code: 'custom', message: 'replacement' }
    expect(simulationRun({ id: 'changed', error: replacementError })).toMatchObject({
      id: 'changed',
      sessionId: '1',
      error: replacementError,
    })

    const replacementAnalysis = {
      summary: 'replacement', facts: [], possibleCauses: [], unknowns: [], recommendations: [],
    }
    expect(failureDiagnosis({ analysis: replacementAnalysis }).analysis).toBe(replacementAnalysis)

    const replacementLimits = {
      maxFields: 1, maxObservedPathsPerField: 2, maxSeries: 3,
      maxPoints: 4, maxCategories: 5, maxHeatmapCells: 6,
    }
    expect(chartCapabilities({ limits: replacementLimits }).limits).toBe(replacementLimits)
  })
})
