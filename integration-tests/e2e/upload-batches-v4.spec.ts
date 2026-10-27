import { test, expect } from './fixtures'

test('unified upload routes preserve recoverable decisions and owned replacement context',async({appPage,control})=>{
  const {api}=await appPage({subject:'alice@example.com'})
  const sessionResponse=await api.post('/api/sessions',{data:{title:'V4 upload routes'}})
  expect(sessionResponse.status()).toBe(201)
  const session=await sessionResponse.json() as {id:string}
  const multipart=(clientBatchId:string,value:string)=>({
    clientBatchId,
    'file.data':{name:'data.csv',mimeType:'text/csv',buffer:Buffer.from(`x,y\n${value}\n`)},
  })
  const createdResponse=await api.post(`/api/sessions/${session.id}/upload-batches`,{multipart:multipart('e2e-upload','1,2')})
  expect(createdResponse.status()).toBe(202)
  const created=await createdResponse.json() as {uploadBatch:{id:string;candidates:Array<{id:string}>}}
  expect((await api.get(`/api/upload-batches/${created.uploadBatch.id}`)).status()).toBe(200)
  expect((await api.put(`/api/upload-batches/${created.uploadBatch.id}/parse-settings`,{data:{delimiter:44}})).status()).toBe(200)
  expect((await api.put(`/api/upload-batches/${created.uploadBatch.id}/candidates`,{data:{decisions:[{candidateId:created.uploadBatch.candidates[0].id,purpose:'data_resource',selected:true,displayName:'数据',alias:'data',tableName:null}]}})).status()).toBe(200)
  await control.seedDataResource({userId:'alice@example.com',sessionId:Number(session.id),uploadBatchId:Number(created.uploadBatch.id),resourceId:'e2e-resource'})
  const replacement=await api.post('/api/data-resources/e2e-resource/replacement-batches',{multipart:multipart('e2e-replacement','3,4')})
  expect(replacement.status()).toBe(202)
  const replacementBody=await replacement.json() as {uploadBatch:{id:string}}
  expect((await api.delete(`/api/upload-batches/${replacementBody.uploadBatch.id}`)).status()).toBe(204)
  expect((await api.delete(`/api/upload-batches/${created.uploadBatch.id}`)).status()).toBe(204)
})
