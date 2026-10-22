const token = 'e2e-control-token'

export async function restartBackend() {
  const response = await fetch('http://127.0.0.1:15174/restart', {
    method: 'POST',
    headers: { 'x-zjudss-e2e-token': token },
  })
  if (!response.ok) throw new Error(`backend restart failed: ${response.status} ${await response.text()}`)
}

export async function directApi(
  path: string,
  identity: { subject?: string; tenant?: string; token?: string },
  init: RequestInit = {},
) {
  const headers = new Headers(init.headers)
  if (identity.subject) headers.set('x-zjudss-subject', identity.subject)
  if (identity.tenant) headers.set('x-zjudss-tenant', identity.tenant)
  if (identity.token) headers.set('x-zjudss-proxy-token', identity.token)
  return fetch(`http://127.0.0.1:18080${path}`, { ...init, headers })
}
