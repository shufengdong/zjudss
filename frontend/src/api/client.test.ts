import { afterEach, describe, expect, it, vi } from 'vitest'
import { login, request } from './client'

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
