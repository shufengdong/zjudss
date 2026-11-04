import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  ApiError,
  downloadDataResource,
  downloadDataset,
  downloadModelDataFrame,
  deleteSession,
  getModelDataFrame,
  getModelDataFrameRows,
  login,
  listDataResources,
  previewSessionDeletion,
  publishDraft,
  request,
  uploadBatchFiles,
} from './client'
import type { ModelDataFrameSource } from '../types'

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('session deletion requests', () => {
  it('previews impact and submits the opaque impact token', async () => {
    const value = {
      impactToken: 'opaque token', modelCount: 1, dataResourceCount: 2, scriptCount: 3,
      runCount: 4, resultCount: 5, analysisArtifactCount: 6, logicalBytes: 7,
      expiresAt: '2026-10-27T00:00:00Z',
    }
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify(value), { status: 200 }))
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
    vi.stubGlobal('fetch', fetchMock)

    await expect(previewSessionDeletion('42')).resolves.toEqual(value)
    await deleteSession('42', 'opaque token')

    expect(fetchMock.mock.calls.map(([url, init]) => [url, init?.method])).toEqual([
      ['/api/sessions/42/deletion-preview', 'POST'],
      ['/api/sessions/42?impactToken=opaque+token', 'DELETE'],
    ])
  })
})

describe('authentication-required notifications', () => {
  it('coalesces protected 401 responses into one browser event', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(
      JSON.stringify({ error: '请先登录', code: 'authentication_required' }),
      { status: 401, headers: { 'content-type': 'application/json' } },
    )))
    const listener = vi.fn()
    window.addEventListener('zjudss:authentication-required', listener)
    await Promise.allSettled([request('/api/sessions'), request('/api/me')])
    expect(listener).toHaveBeenCalledTimes(1)
    window.removeEventListener('zjudss:authentication-required', listener)
  })

  it('does not treat invalid login credentials as an expired protected session', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(
      JSON.stringify({ error: '用户 ID 或密码错误', code: 'invalid_credentials' }),
      { status: 401, headers: { 'content-type': 'application/json' } },
    )))
    const listener = vi.fn()
    window.addEventListener('zjudss:authentication-required', listener)
    await expect(login('alice', 'wrong')).rejects.toMatchObject({ status: 401 })
    expect(listener).not.toHaveBeenCalled()
    window.removeEventListener('zjudss:authentication-required', listener)
  })
})

describe('protected downloads', () => {
  const version: ModelDataFrameSource = {
    kind: 'model-version', dataframeId: '11', versionId: '7', tableName: 'bus',
  }

  it('uses response filenames for all download endpoints', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response('dataset', {
        status: 200,
        headers: { 'content-disposition': 'attachment; filename="dataset-result.parquet"' },
      }))
      .mockResolvedValueOnce(new Response('resource', {
        status: 200,
        headers: { 'content-disposition': 'attachment; filename="resource-source.csv"' },
      }))
      .mockResolvedValueOnce(new Response('model', {
        status: 200,
        headers: { 'content-disposition': 'attachment; filename="model-table.parquet"' },
      }))
    vi.stubGlobal('fetch', fetchMock)

    await expect(downloadDataset('1', 'fallback-dataset')).resolves.toMatchObject({ filename: 'dataset-result.parquet' })
    await expect(downloadDataResource('2', 'original', 'fallback-resource')).resolves.toMatchObject({ filename: 'resource-source.csv' })
    await expect(downloadModelDataFrame(version)).resolves.toMatchObject({ filename: 'model-table.parquet' })
    expect(fetchMock).toHaveBeenCalledTimes(3)
  })

  it('uses endpoint-specific fallback filenames when disposition is absent', async () => {
    const fetchMock = vi.fn().mockImplementation(
      () => Promise.resolve(new Response('content', { status: 200 })),
    )
    vi.stubGlobal('fetch', fetchMock)

    await expect(downloadDataset('1', 'dataset-name')).resolves.toMatchObject({ filename: 'dataset-name.parquet' })
    await expect(downloadDataResource('2', 'normalized', 'resource-name')).resolves.toMatchObject({ filename: 'resource-name.parquet' })
    await expect(downloadModelDataFrame(version)).resolves.toMatchObject({ filename: 'bus.parquet' })
    expect(fetchMock).toHaveBeenCalledTimes(3)
  })

  it('uses structured errors and emits one authentication event for a download 401', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(
      JSON.stringify({ error: '请先登录', code: 'authentication_required', details: { reason: 'expired' } }),
      { status: 401, headers: { 'content-type': 'application/json' } },
    ))
    vi.stubGlobal('fetch', fetchMock)
    await new Promise((resolve) => window.setTimeout(resolve, 0))
    const listener = vi.fn()
    window.addEventListener('zjudss:authentication-required', listener)

    await expect(downloadDataset('1')).rejects.toEqual(
      new ApiError(401, '请先登录', 'authentication_required', { reason: 'expired' }),
    )
    expect(listener).toHaveBeenCalledTimes(1)
    expect(fetchMock).toHaveBeenCalledTimes(1)
    window.removeEventListener('zjudss:authentication-required', listener)
  })

  it('uses a stable fallback error for non-JSON download failures', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response('upstream failed', { status: 500 }))
    vi.stubGlobal('fetch', fetchMock)

    await expect(downloadDataResource('2', 'original')).rejects.toMatchObject({
      status: 500,
      message: '请求失败（500）',
    })
    expect(fetchMock).toHaveBeenCalledTimes(1)
  })

  it('passes the caller abort signal through every download', async () => {
    const fetchMock = vi.fn().mockImplementation(
      () => Promise.resolve(new Response('content', { status: 200 })),
    )
    vi.stubGlobal('fetch', fetchMock)
    const signal = new AbortController().signal

    await downloadDataset('1', 'dataset', signal)
    await downloadDataResource('2', 'original', 'resource', signal)
    await downloadModelDataFrame(version, signal)

    expect(fetchMock.mock.calls).toHaveLength(3)
    for (const [, init] of fetchMock.mock.calls as [string, RequestInit][]) {
      expect(init.signal).toBe(signal)
    }
  })
})

describe('model dataframe requests', () => {
  const version: ModelDataFrameSource = {
    kind: 'model-version', dataframeId: '11', versionId: '7', tableName: 'bus',
  }
  const draft: ModelDataFrameSource = {
    kind: 'model-draft', dataframeId: '12', draftId: '8', revision: 3, tableName: 'load',
  }

  it('uses the version dataframe id without draft query fields', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ id: '11' }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ resourceId: '11' }), { status: 200 }))
      .mockResolvedValueOnce(new Response('parquet', {
        status: 200,
        headers: { 'content-disposition': 'attachment; filename="model-v7-bus.parquet"' },
      }))
    vi.stubGlobal('fetch', fetchMock)
    await getModelDataFrame(version)
    await getModelDataFrameRows(version, { limit: 100, cursor: 'next', columns: 'id,name' })
    await downloadModelDataFrame(version)
    expect(fetchMock.mock.calls.map(([url]) => url)).toEqual([
      '/api/model-dataframes/11',
      '/api/model-dataframes/11/rows?limit=100&cursor=next&columns=id%2Cname',
      '/api/model-dataframes/11/download',
    ])
  })

  it('always sends the exact draft identity and revision', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ id: '12' }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ resourceId: '12' }), { status: 200 }))
      .mockResolvedValueOnce(new Response('parquet', { status: 200 }))
    vi.stubGlobal('fetch', fetchMock)
    await getModelDataFrame(draft)
    await getModelDataFrameRows(draft, { limit: 2, columns: 'id' })
    await downloadModelDataFrame(draft)
    expect(fetchMock.mock.calls.map(([url]) => url)).toEqual([
      '/api/draft-dataframes/12?draftId=8&expectedDraftRevision=3',
      '/api/draft-dataframes/12/rows?draftId=8&expectedDraftRevision=3&limit=2&columns=id',
      '/api/draft-dataframes/12/download?draftId=8&expectedDraftRevision=3',
    ])
  })
})

describe('V4 upload and resource requests', () => {
  it('sends only the unified multipart fields accepted by the backend', async () => {
    const response = { uploadBatch: { id: 'batch-1' } }
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify(response), { status: 202 }))
    vi.stubGlobal('fetch', fetchMock)
    const file = new File(['id\n1'], 'data.csv')
    await uploadBatchFiles('7', 'client-1', [{ id: 'file-1', file, relativePath: 'folder/data.csv' }])
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(url).toBe('/api/sessions/7/upload-batches')
    expect(init.method).toBe('POST')
    const form = init.body as FormData
    expect(Array.from(form.keys())).toEqual(['clientBatchId', 'file.file-1', 'relativePaths'])
    expect(form.get('clientBatchId')).toBe('client-1')
    expect(JSON.parse(String(form.get('relativePaths')))).toEqual({ 'file-1': 'folder/data.csv' })
  })

  it('keeps opaque resource cursors and publishes the optional version description', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ resources: [], nextCursor: null }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ version: { id: 'v' }, deduplicated: false, replayed: false }), { status: 201 }))
    vi.stubGlobal('fetch', fetchMock)
    await listDataResources('9', { cursor: 'opaque cursor', limit: 50 })
    await publishDraft('9', { id: '4', revision: 3 }, 'publish-1', '牛顿法基线')
    expect(fetchMock.mock.calls[0][0]).toBe('/api/sessions/9/data-resources?cursor=opaque+cursor&limit=50')
    expect(JSON.parse(String(fetchMock.mock.calls[1][1]?.body))).toEqual({ expectedDraftId: 4, expectedDraftRevision: 3, clientPublishId: 'publish-1', description: '牛顿法基线' })
  })
})
