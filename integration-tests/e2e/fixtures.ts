import { appendFile } from 'node:fs/promises'
import { relative, resolve } from 'node:path'
import { test as base, expect, type APIRequestContext, type APIResponse, type BrowserContext, type Page } from '@playwright/test'
import { ControlClient } from './helpers/control'

export interface AppIdentity { subject: string; tenant?: string }
export interface AppHandle { page: Page; context: BrowserContext; api: APIRequestContext; identity: AppIdentity }

type Fixtures = {
  appPage: (identity: AppIdentity) => Promise<AppHandle>
  loginAs: (id: string) => Promise<AppHandle>
  control: ControlClient
}

export const test = base.extend<Fixtures>({
  appPage: async ({ browser }, use, testInfo) => {
    const contexts: BrowserContext[] = []
    const requests = new Map<string, { method: string; path: string; status: number }>()
    const record = (method: string, value: string, status: number) => {
      const url = new URL(value, 'http://127.0.0.1:15173')
      if (url.pathname !== '/health' && !url.pathname.startsWith('/api/')) return
      const hit = { method: method.toUpperCase(), path: url.pathname, status }
      requests.set(`${hit.method} ${hit.path} ${hit.status}`, hit)
    }
    const trackApi = (api: APIRequestContext) => new Proxy(api, {
      get(target, property, receiver) {
        if (typeof property === 'string' && ['get', 'post', 'patch', 'delete', 'put', 'head'].includes(property)) {
          return async (url: string, options?: object) => {
            const response = await (target[property as keyof APIRequestContext] as (url: string, options?: object) => Promise<APIResponse>)(url, options)
            record(property, url, response.status())
            return response
          }
        }
        const value = Reflect.get(target, property, receiver)
        return typeof value === 'function' ? value.bind(target) : value
      },
    }) as APIRequestContext
    await use(async (identity) => {
      const context = await browser.newContext()
      contexts.push(context)
      const login = await context.request.post('http://127.0.0.1:15173/api/auth/login', {
        data: { id: identity.subject, password: identity.subject },
      })
      record('POST', '/api/auth/login', login.status())
      if (!login.ok()) throw new Error(`e2e login failed for ${identity.subject}: ${login.status()}`)
      const page = await context.newPage()
      page.on('response', (response) => record(response.request().method(), response.url(), response.status()))
      return { page, context, api: trackApi(context.request), identity }
    })
    for (const context of contexts) await context.close()
    const evidencePath = resolve(process.cwd(), '../target/functional-coverage/e2e-routes.ndjson')
    await appendFile(evidencePath, `${JSON.stringify({
      file: relative(resolve(process.cwd(), '..'), testInfo.file).replaceAll('\\', '/'),
      name: testInfo.title,
      passed: testInfo.errors.length === 0 && testInfo.status !== 'skipped',
      requests: [...requests.values()],
    })}\n`)
  },
  loginAs: async ({ appPage }, use) => use((id) => appPage({ subject: id })),
  control: async ({}, use) => use(new ControlClient()),
})

export { expect }
