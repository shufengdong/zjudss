import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { ResultMessage } from '../../types'
import ResultCard from './ResultCard'

const output = {
  datasetId: '22', name: 'bus_voltage', rowCount: 2, columns: [], previewTruncated: false, views: [],
}
const handlers = { onOpenDataset: vi.fn(), notify: vi.fn() }

function message(producer: { runId?: string; analysisRunId?: string }): ResultMessage {
  return { id: 'm1', type: 'result', timestamp: '', title: '结果', outputs: [output], ...producer }
}

describe('ResultCard analysis details entry', () => {
  it('does not show analysis controls for simulation results', () => {
    render(<ResultCard message={message({ runId: '8' })} handlers={handlers} />)
    expect(screen.queryByRole('button', { name: '查看分析脚本与输入' })).not.toBeInTheDocument()
  })

  it('shows the lazy details entry only for analysis results', () => {
    render(<ResultCard message={message({ analysisRunId: '9' })} handlers={handlers} />)
    expect(screen.getByRole('button', { name: '查看分析脚本与输入' })).toBeInTheDocument()
  })
})
