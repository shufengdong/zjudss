import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  downloadModelDataFrame,
  getModelDataFrame,
  getModelDataFrameRows,
  login,
  request,
} from './client'
import type { ModelDataFrameSource } from '../types'

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
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
