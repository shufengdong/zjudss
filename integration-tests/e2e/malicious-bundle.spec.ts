import { resolve } from 'node:path'
import { test, expect } from './fixtures'
import { createSessionFromPage } from './helpers/journeys'

const root=resolve(process.cwd(),'../integration-tests/fixtures/uploads/malicious')

test('path traversal absolute duplicate symlink and compression-bomb ZIPs fail atomically',async({appPage})=>{
  const{page,api}=await appPage({subject:'malicious@example.com',tenant:'grid-b'})
  const before=await (await api.get('/api/storage-usage')).json()
  for(const name of ['traversal.zip','absolute.zip','duplicate.zip','symlink.zip','bomb.zip']){
    const session=await createSessionFromPage(page,`恶意归档 ${name}`)
    const responsePromise=page.waitForResponse(r=>/\/api\/sessions\/[^/]+\/upload-batches$/.test(new URL(r.url()).pathname)&&r.request().method()==='POST')
    await page.locator('input[type=file]').first().setInputFiles(resolve(root,name))
    const accepted=await (await responsePromise).json() as {uploadBatch:{id:string}}
    await expect.poll(async()=>{
      const value=await (await api.get(`/api/upload-batches/${accepted.uploadBatch.id}`)).json();return value.uploadBatch.status
    }).toBe('failed')
    const workspace=await (await api.get(`/api/sessions/${session}/workspace`)).json()
    expect(workspace.active).toBeNull();expect(workspace.draft).toBeNull()
    await expect(page.getByText(/上传批次 #.*failed/)).toBeVisible()
    const after=await (await api.get('/api/storage-usage')).json()
    expect({artifactCount:after.artifactCount,logicalBytes:after.logicalBytes}).toEqual({
      artifactCount:before.artifactCount,logicalBytes:before.logicalBytes,
    })
    expect((await api.delete(`/api/upload-batches/${accepted.uploadBatch.id}`)).status()).toBe(204)
  }
})
