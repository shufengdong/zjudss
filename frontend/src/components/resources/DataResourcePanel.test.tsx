import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { deleteDataResource, listDataResources, previewDataResourceDeletion } from '../../api/client'
import DataResourcePanel from './DataResourcePanel'

vi.mock('../../api/client', () => ({
  listDataResources: vi.fn(), getDataResource: vi.fn(), getDataResourceRows: vi.fn(),
  downloadDataResource: vi.fn(), previewDataResourceDeletion: vi.fn(), deleteDataResource: vi.fn(),
}))

const make = (overrides: Record<string, unknown>) => ({
  id: 'r', sessionId: 's', displayName: '数据', alias: 'data', status: 'live', declaredSchemaRef: null,
  sourceKind: 'upload', schema: [{ name: 'id', dtype: 'Utf8', nullable: false }], schemaHash: 'schema',
  contentHash: 'content', rowCount: 2, sizeBytes: 20, originalAvailable: false, sourceBatchId: null,
  sourceModelVersionId: null, replacedByResourceId: null, lineage: {}, createdAtMs: 1, ...overrides,
})

describe('DataResourcePanel', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(listDataResources).mockResolvedValue({ resources: [
      make({ id: 'para', displayName: '潮流参数', alias: 'para', declaredSchemaRef: 'frames.para', rowCount: 1, sourceKind: 'case' }),
      make({ id: 'curve', displayName: '负荷曲线', alias: 'load_curve', rowCount: 24, originalAvailable: true }),
    ], nextCursor: null })
  })

  it('shows para as an ordinary resource and only offers parameter editing for a struct-compatible single row', async () => {
    render(<DataResourcePanel sessionId="s" batches={[]} notify={vi.fn()} onChanged={vi.fn()} />)
    const para = (await screen.findByText('潮流参数')).closest('article')!
    expect(within(para).getByText(/alias: para/)).toBeInTheDocument()
    expect(within(para).getByText(/frames\.para/)).toBeInTheDocument()
    expect(within(para).getByRole('button', { name: '修改参数' })).toBeInTheDocument()
    const curve = screen.getByText('负荷曲线').closest('article')!
    expect(within(curve).queryByRole('button', { name: '修改参数' })).not.toBeInTheDocument()
    expect(within(curve).getByRole('button', { name: '下载原文件' })).toBeInTheDocument()
  })

  it('loads deletion impact before requiring explicit confirmation', async () => {
    vi.mocked(previewDataResourceDeletion).mockResolvedValue({
      impactToken: 'token', resourceId: 'curve', displayName: '负荷曲线', alias: 'load_curve', sizeBytes: 20,
      pendingConfigurationCount: 1, frozenReferenceCount: 2, historicalReferenceCount: 3, expiresAtMs: 10,
    })
    vi.mocked(deleteDataResource).mockResolvedValue({ resourceId: 'curve', tombstone: {} })
    render(<DataResourcePanel sessionId="s" batches={[]} notify={vi.fn()} onChanged={vi.fn()} />)
    const curve = (await screen.findByText('负荷曲线')).closest('article')!
    fireEvent.click(within(curve).getByRole('button', { name: '删除' }))
    expect(await screen.findByRole('dialog', { name: '删除数据文件' })).toHaveTextContent('1 个未确认配置')
    expect(deleteDataResource).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: '确认删除' }))
    await waitFor(() => expect(deleteDataResource).toHaveBeenCalledWith('curve', 'token'))
  })

  it('requires one of the three explicit actions for a replacement batch', async () => {
    const actions = {
      updateParseSettings: vi.fn(), updateCandidates: vi.fn().mockResolvedValue(undefined),
      confirm: vi.fn().mockResolvedValue(undefined), discard: vi.fn(), addBatch: vi.fn(), watch: vi.fn(),
    }
    render(<DataResourcePanel sessionId="s" batches={[{
      id: 'batch', sessionId: 's', clientBatchId: 'client', sourceKind: 'upload', replacementResourceId: 'curve',
      status: 'ready_to_confirm', modelPublishStatus: 'not_applicable', resourcePublishStatus: 'ready',
      parseSettings: {}, diagnostics: [], reservedBytes: 10, expiresAtMs: 10, revision: 2,
      candidates: [{ id: 'candidate', displayName: 'new.csv', proposedAlias: 'load_curve', purpose: 'data_resource', evidence: { rowCountBefore: 24, rowCountAfter: 48 }, selected: true }],
    }]} actions={actions} notify={vi.fn()} onChanged={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: '另存为新资源' }))
    await waitFor(() => expect(actions.confirm).toHaveBeenCalledWith('batch', expect.any(Array), 'save_as_new'))
  })
})
