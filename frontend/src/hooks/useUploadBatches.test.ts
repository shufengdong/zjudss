import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { confirmUploadBatch, getUploadBatch, uploadBatchFiles } from '../api/client'
import { classifyUploads, selectableCandidateIds, useUploadBatches } from './useUploadBatches'
import type { UploadBatchView } from '../types'

vi.mock('../api/client', () => ({
  getUploadBatch: vi.fn(), uploadBatchFiles: vi.fn(), uploadScript: vi.fn(),
  updateUploadCandidates: vi.fn(), updateUploadParseSettings: vi.fn(), confirmUploadBatch: vi.fn(),
  discardUploadBatch: vi.fn(),
}))

const batch = (id: string, sessionId = 's'):UploadBatchView => ({
  id, sessionId, clientBatchId: `client-${id}`, sourceKind: 'upload', replacementResourceId: null,
  status: 'awaiting_purpose', modelPublishStatus: 'ready', resourcePublishStatus: 'ready',
  parseSettings: {}, diagnostics: [], reservedBytes: 1, expiresAtMs: 10, revision: 1,
  candidates: [],
})

describe('useUploadBatches', () => {
  beforeEach(() => vi.resetAllMocks())

  it('classifies unified data/model inputs while keeping RSS scripts separate', () => {
    const files = [new File([''], 'model.dss'), new File([''], 'load.csv'), new File([''], 'run.rss'), new File([''], 'bad.exe')]
    const classified = classifyUploads(files)
    expect(classified.tabular.map(file => file.name)).toEqual(['model.dss', 'load.csv'])
    expect(classified.scripts.map(file => file.name)).toEqual(['run.rss'])
    expect(classified.unsupported.map(file => file.name)).toEqual(['bad.exe'])
  })

  it('keeps every dependency in an uploaded model directory inside one unified batch', () => {
    const entry=new File(['redirect settings.json'], 'model.dss'),manifest=new File(['{}'],'manifest.json'),script=new File(['x=1;'],'entry.rss')
    Object.defineProperty(entry,'webkitRelativePath',{value:'case.rssbundle/model.dss'})
    Object.defineProperty(manifest,'webkitRelativePath',{value:'case.rssbundle/manifest.json'})
    Object.defineProperty(script,'webkitRelativePath',{value:'case.rssbundle/entry.rss'})
    const classified=classifyUploads([entry,manifest,script])
    expect(classified.tabular.map(file=>file.name)).toEqual(['model.dss','manifest.json','entry.rss'])
    expect(classified.scripts).toEqual([])
    expect(classified.unsupported).toEqual([])
  })

  it('never includes hidden sheets in select-all', () => {
    expect(selectableCandidateIds([
      { id: 'visible', selected: true, evidence: {}, sheetName: 'visible' },
      { id: 'hidden', selected: false, evidence: { hidden: true }, sheetName: 'hidden' },
    ] as never)).toEqual(['visible'])
  })

  it('recovers durable batch ids and clears them when the session changes', async () => {
    vi.mocked(getUploadBatch).mockImplementation(async id => batch(id, id === 'one' ? 's1' : 's2'))
    const { result, rerender } = renderHook(({ sessionId, ids }) => useUploadBatches(sessionId, null, vi.fn(), ids), {
      initialProps: { sessionId: 's1' as string | null, ids: ['one'] },
    })
    await waitFor(() => expect(result.current.batches.map(value => value.id)).toEqual(['one']))
    rerender({ sessionId: 's2', ids: ['two'] })
    await waitFor(() => expect(result.current.batches.map(value => value.id)).toEqual(['two']))
  })

  it('adds newly observed durable ids without clearing batches already accepted in the session', async () => {
    vi.mocked(uploadBatchFiles).mockResolvedValue(batch('local'))
    vi.mocked(getUploadBatch).mockResolvedValue(batch('durable'))
    const { result, rerender } = renderHook(({ids})=>useUploadBatches('s',null,vi.fn(),ids),{initialProps:{ids:[] as string[]}})
    await act(async()=>result.current.upload([new File(['id\n1'],'data.csv')]))
    expect(result.current.batches.map(value=>value.id)).toContain('local')
    rerender({ids:['durable']})
    await waitFor(()=>expect(result.current.batches.map(value=>value.id).sort()).toEqual(['durable','local']))
  })

  it('uploads all supported non-RSS files in one unified batch', async () => {
    vi.mocked(uploadBatchFiles).mockResolvedValue(batch('new'))
    const { result } = renderHook(() => useUploadBatches('s', null, vi.fn(), []))
    await act(async () => result.current.upload([new File(['id\n1'], 'data.csv'), new File(['x'], 'case.dss')]))
    expect(uploadBatchFiles).toHaveBeenCalledTimes(1)
    expect(result.current.batches[0].id).toBe('new')
  })

  it('reuses the confirmation operation id when the same request is retried after a lost response', async () => {
    vi.mocked(confirmUploadBatch).mockRejectedValueOnce(new Error('network')).mockResolvedValueOnce({ uploadBatchId:'batch',status:'confirmed',modelStatus:'published',resourceStatus:'published',resources:[],diagnostics:[] })
    const { result } = renderHook(() => useUploadBatches('s', null, vi.fn(), []))
    const decisions = [{ candidateId:'candidate',purpose:'data_resource' as const,selected:true }]
    await expect(result.current.confirm('batch',decisions)).rejects.toThrow('network')
    await result.current.confirm('batch',decisions)
    const first=vi.mocked(confirmUploadBatch).mock.calls[0][1].clientOperationId,second=vi.mocked(confirmUploadBatch).mock.calls[1][1].clientOperationId
    expect(second).toBe(first)
  })
})
