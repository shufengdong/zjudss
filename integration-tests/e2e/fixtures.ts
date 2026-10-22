import { test as base, expect, type APIRequestContext, type BrowserContext, type Page } from '@playwright/test'
import { ControlClient } from './helpers/control'

export interface AppIdentity { subject: string; tenant?: string }
export interface AppHandle { page: Page; context: BrowserContext; api: APIRequestContext; identity: AppIdentity }

type Fixtures = {
  appPage: (identity: AppIdentity) => Promise<AppHandle>
  control: ControlClient
}

export const test = base.extend<Fixtures>({
  appPage: async ({ browser }, use) => {
    const contexts: BrowserContext[] = []
    await use(async (identity) => {
      const context = await browser.newContext()
      contexts.push(context)
      const cookies = [
        { name: 'zjudss-e2e-subject', value: identity.subject, url: 'http://127.0.0.1:15173' },
      ]
      if (identity.tenant) {
        cookies.push({ name: 'zjudss-e2e-tenant', value: identity.tenant, url: 'http://127.0.0.1:15173' })
      }
      await context.addCookies(cookies)
      const page = await context.newPage()
      return { page, context, api: context.request, identity }
    })
    for (const context of contexts) await context.close()
  },
  control: async ({}, use) => use(new ControlClient()),
})

export { expect }
