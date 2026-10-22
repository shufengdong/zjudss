import { resolve } from 'node:path'
import { test, expect } from './fixtures'

const root=resolve(process.cwd(),'../integration-tests/fixtures/uploads/malicious')

test('path traversal absolute duplicate symlink and compression-bomb ZIPs fail atomically',async({appPage})=>{
  const{page,api}=await appPage({subject:'malicious@example.com',tenant:'grid-b'})
  for(const name of ['traversal.zip','absolute.zip','duplicate.zip','symlink.zip','bomb.zip']){
    const created=await api.post('/api/sessions',{data:{title:`恶意归档 ${name}`}});const session=(await created.json()).id as string
    await page.goto('/');await page.getByRole('button',{name:`打开会话“恶意归档 ${name}”`}).click()
    const responsePromise=page.waitForResponse(r=>r.url().includes('/attachments')&&r.request().method()==='POST')
    await page.locator('input[type=file]').first().setInputFiles(resolve(root,name))
    const accepted=await (await responsePromise).json() as {ingestionId:string}
    await expect.poll(async()=>{
      const value=await (await api.get(`/api/ingestions/${accepted.ingestionId}`)).json();return value.ingestion.status
    }).toBe('failed')
    const workspace=await (await api.get(`/api/sessions/${session}/workspace`)).json()
    expect(workspace.active).toBeNull();expect(workspace.draft).toBeNull()
    await expect(page.getByText('failed',{exact:true})).toBeVisible()
  }
})
