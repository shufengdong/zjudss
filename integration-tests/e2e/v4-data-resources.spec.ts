import { test, expect } from './fixtures'

test('unified upload publishes and manages independent data resources through public routes',async({appPage})=>{
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
  const decision={candidateId:created.uploadBatch.candidates[0].id,purpose:'data_resource',selected:true,displayName:'数据',alias:'data',tableName:null}
  expect((await api.put(`/api/upload-batches/${created.uploadBatch.id}/candidates`,{data:{decisions:[decision]}})).status()).toBe(200)
  const confirmed=await api.post(`/api/upload-batches/${created.uploadBatch.id}/confirm`,{data:{decisions:[decision],clientOperationId:'e2e-confirm-data'}})
  expect(confirmed.status()).toBe(200)
  const confirmedBody=await confirmed.json() as {uploadConfirmation:{resources:Array<{id:string}>}}
  const resourceId=confirmedBody.uploadConfirmation.resources[0].id
  expect((await api.get(`/api/sessions/${session.id}/data-resources`)).status()).toBe(200)
  expect((await api.get(`/api/data-resources/${resourceId}`)).status()).toBe(200)
  expect((await api.patch(`/api/data-resources/${resourceId}`,{data:{displayName:'更新后的数据',alias:'renamed_data'}})).status()).toBe(200)
  expect((await api.get(`/api/data-resources/${resourceId}/rows?limit=1&columns=x`)).status()).toBe(200)
  expect((await api.get(`/api/data-resources/${resourceId}/download?kind=normalized`)).status()).toBe(200)
  expect((await api.get(`/api/data-resources/${resourceId}/download?kind=original`)).status()).toBe(200)

  const replacement=await api.post(`/api/data-resources/${resourceId}/replacement-batches`,{multipart:multipart('e2e-replacement','3,4')})
  expect(replacement.status()).toBe(202)
  const replacementBody=await replacement.json() as {uploadBatch:{id:string;candidates:Array<{id:string}>}}
  const replacementDecision={candidateId:replacementBody.uploadBatch.candidates[0].id,purpose:'data_resource',selected:true,displayName:'替换数据',alias:'replacement_data',tableName:null}
  const replaced=await api.post(`/api/upload-batches/${replacementBody.uploadBatch.id}/confirm`,{data:{decisions:[replacementDecision],replacementAction:'replace',clientOperationId:'e2e-replace-data'}})
  expect(replaced.status()).toBe(200)
  const replacedBody=await replaced.json() as {uploadConfirmation:{resources:Array<{id:string}>}}
  const replacementId=replacedBody.uploadConfirmation.resources[0].id
  expect((await api.get(`/api/data-resources/${resourceId}`)).status()).toBe(404)
  const preview=await api.post(`/api/data-resources/${replacementId}/deletion-preview`)
  expect(preview.status()).toBe(200)
  const previewBody=await preview.json() as {impactToken:string}
  expect((await api.delete(`/api/data-resources/${replacementId}?impactToken=${previewBody.impactToken}`)).status()).toBe(200)

  const staticUpload=await api.post(`/api/sessions/${session.id}/upload-batches`,{multipart:{
    clientBatchId:'e2e-struct-upload',
    'file.master':{name:'master.dss',mimeType:'text/plain',buffer:Buffer.from('New Circuit.E2E\n')},
  }})
  expect(staticUpload.status()).toBe(202)
  const staticBody=await staticUpload.json() as {uploadBatch:{id:string;candidates:Array<{id:string;proposedAlias:string|null}>}}
  const staticDecisions=staticBody.uploadBatch.candidates.map((candidate)=>{
    const isPara=candidate.proposedAlias==='para'
    return {candidateId:candidate.id,purpose:isPara?'data_resource':'skip',selected:isPara,displayName:isPara?'参数':null,alias:isPara?'para':null,tableName:null}
  })
  const staticConfirmed=await api.post(`/api/upload-batches/${staticBody.uploadBatch.id}/confirm`,{data:{decisions:staticDecisions,clientOperationId:'e2e-confirm-struct'}})
  expect(staticConfirmed.status()).toBe(200)
  const staticConfirmation=await staticConfirmed.json() as {uploadConfirmation:{resources:Array<{id:string;contentHash:string}>}}
  const para=staticConfirmation.uploadConfirmation.resources[0]
  const structuredPreview=await api.post(`/api/data-resources/${para.id}/structured-replacement-previews`,{data:{
    expectedContentHash:para.contentHash,
    schemaRef:'frames.para',
    values:{base_frequency_hz:50,max_iter:20},
  }})
  const structuredPreviewText=await structuredPreview.text()
  expect(structuredPreview.status(),structuredPreviewText).toBe(200)
  const structuredPreviewBody=JSON.parse(structuredPreviewText) as {previewToken:string}
  expect((await api.post(`/api/data-resource-replacement-previews/${structuredPreviewBody.previewToken}/confirm`,{data:{action:'save_as_new'}})).status()).toBe(200)

  const discardable=await api.post(`/api/sessions/${session.id}/upload-batches`,{multipart:multipart('e2e-discard','5,6')})
  const discardableBody=await discardable.json() as {uploadBatch:{id:string}}
  expect((await api.delete(`/api/upload-batches/${discardableBody.uploadBatch.id}`)).status()).toBe(204)
})
