import { resolve } from 'node:path'
import type { APIRequestContext, Page } from '@playwright/test'
import { test, expect } from './fixtures'

const fixtures = resolve(process.cwd(), '../integration-tests/fixtures/uploads')
async function create(api:APIRequestContext,title:string){const r=await api.post('/api/sessions',{data:{title}});return (await r.json()).id as string}
async function workspace(api:APIRequestContext,id:string){return (await (await api.get(`/api/sessions/${id}/workspace`)).json()) as any}
const current=(value:any)=>value.draft??value.active
async function upload(page:Page,path:string,directory=false){
  const response=page.waitForResponse(r=>r.url().includes('/attachments')&&r.request().method()==='POST')
  await page.locator('input[type=file]').nth(directory?1:0).setInputFiles(path)
  expect((await response).status()).toBe(202)
}

test('directory and ZIP bundles publish equivalent immutable model content',async({appPage})=>{
  const{page,api}=await appPage({subject:'bundles@example.com',tenant:'grid-b'})
  const direct=await create(api,'目录 bundle');await page.goto('/')
  await upload(page,resolve(fixtures,'bundle/demo.rssbundle'),true)
  await expect.poll(async()=>current(await workspace(api,direct))?.status).toBe('ready')
  const directHash=current(await workspace(api,direct)).contentHash
  const zipped=await create(api,'ZIP bundle');await page.reload();await page.getByRole('button',{name:'打开会话“ZIP bundle”'}).click()
  await upload(page,resolve(fixtures,'bundle/demo.zip'))
  await expect.poll(async()=>current(await workspace(api,zipped))?.status).toBe('ready')
  const zippedVersion=current(await workspace(api,zipped))
  expect(zippedVersion.contentHash).toBe(directHash)
})

test('example search and natural-language import copy a snapshot-pinned editable model',async({appPage,control})=>{
  const{page,api}=await appPage({subject:'examples@example.com',tenant:'grid-b'})
  const session=await create(api,'案例导入');await page.goto('/')
  await page.getByRole('button',{name:'案例库'}).click()
  await page.getByRole('textbox',{name:'搜索内置案例'}).fill('E2E')
  await expect(page.getByText('E2E demo',{exact:true})).toBeVisible()
  await control.enqueueLlm({id:'import-example-ui',expectedPrompt:'请导入内置案例',turns:[
    {kind:'tool_call',call_id:'import-example',name:'import_example',args:{exampleId:'e2e-demo',engineResourceSnapshot:'e2e-snapshot',expectedDraftVersionId:null}},
    {kind:'text',text:'案例副本已导入，可以修改参数'},
  ]})
  await page.getByRole('button',{name:'使用案例“E2E demo”'}).click()
  await expect(page.getByText('案例副本已导入，可以修改参数',{exact:true})).toBeVisible()
  const draft=current(await workspace(api,session))
  expect(draft.origin.exampleId).toBe('e2e-demo');expect(draft.origin.engineResourceSnapshot).toBe('e2e-snapshot')
  expect(draft.status).toBe('ready')
})
