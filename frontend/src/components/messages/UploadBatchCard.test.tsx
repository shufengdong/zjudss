import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { UploadBatchMessage, UploadBatchView } from '../../types'
import UploadBatchCard from './UploadBatchCard'

const batch: UploadBatchView = {
  id: '12', sessionId: '1', clientBatchId: 'client', sourceKind: 'upload', status: 'ready_to_confirm',
  modelPublishStatus: 'ready', resourcePublishStatus: 'ready', parseSettings: {}, diagnostics: [],
  reservedBytes: 100, expiresAtMs: 1, revision: 2,
  candidates: [
    { id: 'visible', displayName: '负荷曲线', sheetName: 'load', proposedAlias: 'load_curve', purpose: 'data_resource', evidence: {}, selected: true },
    { id: 'hidden', displayName: '隐藏说明', sheetName: 'help', proposedAlias: 'help', purpose: 'data_resource', evidence: { hidden: true }, selected: false },
  ],
}
const message: UploadBatchMessage = {
  id: 'message', type: 'upload-batch', timestamp: '2026-01-01T00:00:00Z', revision: 2,
  card: { batchId: '12', batchRevision: 2, state: 'ready_to_confirm', candidates: [], modelTransactionState: 'ready', resourceTransactionState: 'ready' },
}

describe('UploadBatchCard', () => {
  it('uses the complete batch confirmation controls in the conversation', async () => {
    const actions = {
      updateParseSettings: vi.fn(), updateCandidates: vi.fn().mockResolvedValue(undefined),
      confirm: vi.fn().mockResolvedValue(undefined), discard: vi.fn(), addBatch: vi.fn(), watch: vi.fn(),
    }
    render(<UploadBatchCard message={message} batch={batch} actions={actions} notify={vi.fn()} />)
    expect(screen.getByRole('checkbox', { name: '全选可见工作表' })).toBeChecked()
    expect(screen.getByText('隐藏 sheet')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '确认用途并发布' }))
    await waitFor(() => expect(actions.confirm).toHaveBeenCalledWith('12', expect.arrayContaining([
      expect.objectContaining({ candidateId: 'visible', purpose: 'data_resource', selected: true }),
    ]), undefined))
  })
})
