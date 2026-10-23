const token = 'e2e-control-token'

export async function restartBackend() {
  const response = await fetch('http://127.0.0.1:15174/restart', {
    method: 'POST',
    headers: { 'x-zjudss-e2e-token': token },
  })
  if (!response.ok) throw new Error(`backend restart failed: ${response.status} ${await response.text()}`)
}

export async function resetBackend() {
  const response = await fetch('http://127.0.0.1:15174/reset', {
    method: 'POST',
    headers: { 'x-zjudss-e2e-token': token },
  })
  if (!response.ok) throw new Error(`backend reset failed: ${response.status} ${await response.text()}`)
}

export async function directApi(
  path: string,
  userId?: string,
  init: RequestInit = {},
) {
  const headers = new Headers(init.headers)
  if (userId) {
    const login = await fetch('http://127.0.0.1:18080/api/auth/login', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ id: userId, password: userId }),
    })
    if (!login.ok) throw new Error(`direct API login failed: ${login.status}`)
    const cookie = login.headers.get('set-cookie')?.split(';', 1)[0]
    if (!cookie) throw new Error('direct API login returned no cookie')
    headers.set('cookie', cookie)
  }
  return fetch(`http://127.0.0.1:18080${path}`, { ...init, headers })
}
