import { resolve } from 'node:path'
import type { APIRequestContext, Page } from '@playwright/test'
import { test, expect } from './fixtures'
import { createSessionFromPage } from './helpers/journeys'
import { confirmLatestModelUpload } from './helpers/uploads'

const fixtures = resolve(process.cwd(), '../integration-tests/fixtures/uploads')
async function workspace(api:APIRequestContext,id:string){return (await (await api.get(`/api/sessions/${id}/workspace`)).json()) as any}
function isCreateUpload(response:{url():string;request():{method():string}}){
  return /\/api\/sessions\/[^/]+\/upload-batches$/.test(new URL(response.url()).pathname)&&response.request().method()==='POST'
}
async function upload(page:Page,path:string,directory=false){
  const response=page.waitForResponse(isCreateUpload)
  await page.locator('input[type=file]').nth(directory?1:0).setInputFiles(path)
  expect((await response).status()).toBe(202)
  await confirmLatestModelUpload(page)
}

test('directory and ZIP bundles publish equivalent immutable model content',async({appPage})=>{
  const{page,api}=await appPage({subject:'bundles@example.com',tenant:'grid-b'})
  const direct=await createSessionFromPage(page,'目录 bundle')
  await upload(page,resolve(fixtures,'bundle/demo.rssbundle'),true)
  await expect.poll(async()=>(await workspace(api,direct)).draft?.tables?.length).toBe(20)
  expect((await (await api.get(`/api/sessions/${direct}/data-resources`)).json()).resources.map((item:any)=>item.alias)).toContain('para')
  await page.getByRole('button',{name:/^展开可变草稿/}).click()
  await page.getByRole('button',{name:'生成版本'}).click()
  const directHash=(await workspace(api,direct)).active.contentHash
  const zipped=await createSessionFromPage(page,'ZIP bundle')
  await upload(page,resolve(fixtures,'bundle/demo.zip'))
  await expect.poll(async()=>(await workspace(api,zipped)).draft?.tables?.length).toBe(20)
  await page.getByRole('button',{name:/^展开可变草稿/}).click()
  await page.getByRole('button',{name:'生成版本'}).click()
  const zippedVersion=(await workspace(api,zipped)).active
  expect(zippedVersion.contentHash).toBe(directHash)
})

test('single-Solve bundles become drafts while repeated Solve and specialist lifecycles fail without mutation',async({appPage})=>{
  const{page,api}=await appPage({subject:'bundles@example.com',tenant:'grid-b'})
  const session=await createSessionFromPage(page,'生命周期分类')
  await upload(page,resolve(fixtures,'bundle/single-solve.rssbundle'),true)
  const accepted=await workspace(api,session)
  expect(accepted.draft.tables).toHaveLength(20)
  const identity={id:accepted.draft.id,revision:accepted.draft.revision,tables:accepted.draft.tables.map((table:any)=>table.contentHash)}
  await expect(page.getByRole('button',{name:/^展开可变草稿/})).toContainText(`修订 ${accepted.draft.revision}`)

  for(const name of ['unsupported-multi-solve.rssbundle','unsupported-specialist.rssbundle']){
    const responsePromise=page.waitForResponse(isCreateUpload)
    await page.locator('input[type=file]').nth(1).setInputFiles(resolve(fixtures,`bundle/${name}`))
    const response=await responsePromise
    expect(response.status()).toBe(202)
    const batchId=(await response.json()).uploadBatch.id
    await expect.poll(async()=>(await (await api.get(`/api/upload-batches/${batchId}`)).json()).uploadBatch.status).toBe('failed')
    const failed=(await (await api.get(`/api/upload-batches/${batchId}`)).json()).uploadBatch
    expect(failed.diagnostics.errors[0].code).toBe('model_import_unsupported')
    const current=await workspace(api,session)
    expect({id:current.draft.id,revision:current.draft.revision,tables:current.draft.tables.map((table:any)=>table.contentHash)}).toEqual(identity)
    const discardResponse=page.waitForResponse(candidate=>candidate.request().method()==='DELETE'&&new URL(candidate.url()).pathname===`/api/upload-batches/${batchId}`)
    await page.getByRole('button',{name:'抛弃'}).last().click()
    expect((await discardResponse).status()).toBe(204)
    expect((await (await api.get(`/api/upload-batches/${batchId}`)).json()).uploadBatch.status).toBe('failed')
  }
})

test('case application uses the fixed action and creates only a mutable draft',async({appPage})=>{
  const{page,api}=await appPage({subject:'examples@example.com',tenant:'grid-b'})
  const session=await createSessionFromPage(page,'案例导入')
  await page.getByRole('button',{name:'案例库'}).click()
  await expect(page.getByRole('button',{name:'E2E'})).toHaveAttribute('aria-expanded','true')
  await expect(page.getByRole('button',{name:'Demo',exact:true})).toHaveAttribute('aria-expanded','false')
  await page.getByRole('button',{name:'Demo',exact:true}).click()
  await page.getByRole('button',{name:'E2E demo',exact:true}).click()
  const detail=page.getByLabel('案例详情“E2E demo”')
  await expect(detail).toContainText('E2E/Demo/E2E demo.rssbundle')
  await expect(detail).toContainText('dss/model.dss')
  await expect(detail).toContainText('execution_plan')
  await expect(detail).toContainText('dss-entry：1')
  await page.getByRole('button',{name:'使用案例“E2E demo”'}).click()
  await expect(page.getByRole('button',{name:'确认用途并发布'}).last()).toBeVisible()
  const uploadCard=page.getByRole('button',{name:'确认用途并发布'}).last().locator('xpath=ancestor::section[1]')
  await expect(uploadCard.getByRole('checkbox')).toHaveCount(22)
  await confirmLatestModelUpload(page)
  const state=await workspace(api,session)
  expect(state.draft.tables).toHaveLength(20)
  expect((await (await api.get(`/api/sessions/${session}/data-resources`)).json()).resources.map((item:any)=>item.alias)).toContain('para')
  expect(state.active).toBeNull()
  expect((await (await api.get(`/api/sessions/${session}/runs`)).json()).runs).toHaveLength(0)

  await page.setViewportSize({width:390,height:844})
  await page.getByRole('button',{name:'案例库'}).click()
  await page.getByRole('button',{name:'Demo',exact:true}).click()
  await page.getByRole('button',{name:'E2E demo',exact:true}).click()
  await expect(page.getByLabel('案例详情“E2E demo”')).toBeVisible()
  await page.getByRole('button',{name:'返回案例目录'}).click()
  await expect(page.getByRole('button',{name:'Demo',exact:true})).toBeVisible()
  await page.getByRole('button',{name:'E2E demo',exact:true}).click()
  await expect(page.getByLabel('案例详情“E2E demo”')).toBeVisible()
})
