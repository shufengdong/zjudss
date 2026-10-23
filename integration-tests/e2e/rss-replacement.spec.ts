import { resolve } from 'node:path'
import type { APIRequestContext,Page } from '@playwright/test'
import {test,expect}from'./fixtures'
import {createSessionFromPage}from'./helpers/journeys'
const root=resolve(process.cwd(),'../integration-tests/fixtures/uploads/rss')
async function ws(api:APIRequestContext,id:string){return (await (await api.get(`/api/sessions/${id}/workspace`)).json()) as any}
const current=(value:any)=>value.draft??value.active
async function upload(page:Page,path:string){await page.locator('input[type=file]').first().setInputFiles(path);await expect(page.getByLabel(/^上传 .*：accepted$/)).toBeVisible()}
async function send(page:Page,message:string){await page.getByRole('textbox',{name:'消息'}).fill(message);await page.getByRole('button',{name:'发送消息'}).click()}

test('re-uploaded RSS creates immutable replacements and version pointers remain reversible',async({appPage,control})=>{
  const{page,api}=await appPage({subject:'rss-replace@example.com',tenant:'grid-b'})
  const session=await createSessionFromPage(page,'RSS 重传')
  await upload(page,resolve(root,'broken.rss'))
  await expect.poll(async()=>current(await ws(api,session))?.status).toBe('collecting')
  const broken=current(await ws(api,session))
  await upload(page,resolve(root,'replacement.rss'))
  await expect.poll(async()=>{const value=current(await ws(api,session));return value?.versionId===broken.versionId?'':value?.versionId??''}).not.toBe('')
  const uploaded=current(await ws(api,session))
  expect(uploaded.parentVersionId).toBe(broken.versionId)
  expect(uploaded.inputs).toHaveLength(2)

  await control.enqueueEngine({kind:'success',data:[{name:'summary',rows:[['ok',1]],valueUnit:'1'}]})
  await control.enqueueLlm({id:'old-version-task',expectedPrompt:'在当前版本运行潮流',turns:[
    {kind:'tool_call',call_id:'study-skill',name:'load_agent_skill',args:{skill_id:'study-planning'}},
    {kind:'tool_call',call_id:'rss-skill',name:'load_agent_skill',args:{skill_id:'rss-generation'}},
    {kind:'tool_call',call_id:'old-study',name:'run_study',args:{modelVersionId:Number(uploaded.versionId),studyType:'power_flow',studySpec:{},script:'result = 1\n'}},
    {kind:'text',text:'旧版本潮流完成'},
  ]})
  await send(page,'在当前版本运行潮流')
  await expect(page.getByText('旧版本潮流完成',{exact:true})).toBeVisible()
  let oldTask:any
  await expect.poll(async()=>{const tasks=await (await api.get(`/api/sessions/${session}/tasks`)).json();oldTask=tasks[0];return oldTask?.status}).toBe('succeeded')

  await upload(page,resolve(root,'modified.rss'))
  await expect.poll(async()=>(await ws(api,session)).draft?.parentVersionId).toBe(uploaded.versionId)
  const withExtra=(await ws(api,session)).draft
  const extra=withExtra.inputs.find((input:any)=>input.fileName==='modified.rss')
  await control.enqueueLlm({id:'remove-rss',expectedPrompt:'移除刚才误传的文件',turns:[
    {kind:'tool_call',call_id:'remove-rss',name:'remove_model_input',args:{inputId:Number(extra.inputId),expectedDraftVersionId:Number(withExtra.versionId)}},
    {kind:'text',text:'误传文件已移除'},
  ]})
  await send(page,'移除刚才误传的文件')
  await expect(page.getByText('误传文件已移除',{exact:true})).toBeVisible()
  await expect.poll(async()=>{const draft=(await ws(api,session)).draft;return draft?.versionId===withExtra.versionId?'':draft?.versionId??''}).not.toBe('')
  const removed=(await ws(api,session)).draft
  expect(removed.inputs).toHaveLength(2)

  const oldInput=removed.inputs.find((input:any)=>input.fileName==='broken.rss')
  const replacementInput=removed.inputs.find((input:any)=>input.fileName==='replacement.rss')
  await control.enqueueLlm({id:'replace-rss',expectedPrompt:'用新文件替换错误文件',turns:[
    {kind:'tool_call',call_id:'replace-rss',name:'replace_model_input',args:{targetInputId:Number(oldInput.inputId),replacementInputId:Number(replacementInput.inputId),expectedDraftVersionId:Number(removed.versionId)}},
    {kind:'text',text:'错误文件已被新文件替换'},
  ]})
  await send(page,'用新文件替换错误文件')
  await expect(page.getByText('错误文件已被新文件替换',{exact:true})).toBeVisible()
  await expect.poll(async()=>{const draft=(await ws(api,session)).draft;return draft?.versionId===removed.versionId?'':draft?.versionId??''}).not.toBe('')
  const replaced=(await ws(api,session)).draft
  expect(replaced.inputs).toHaveLength(1)
  expect(replaced.parentVersionId).toBe(removed.versionId)

  const editable=replaced.inputs[0]
  await control.enqueueLlm({id:'modify-rss',expectedPrompt:'把结果参数从1改成2',turns:[
    {kind:'tool_call',call_id:'modify-skill',name:'load_agent_skill',args:{skill_id:'rss-generation'}},
    {kind:'tool_call',call_id:'modify-rss',name:'modify_model',args:{inputId:Number(editable.inputId),patch:{tableKind:'rss_parameter',rowKey:{name:'result'},changes:{value:{from:1,to:2,unit:'1'}}},expectedDraftVersionId:Number(replaced.versionId)}},
    {kind:'text',text:'结果参数已修改'},
  ]})
  await send(page,'把结果参数从1改成2')
  await expect(page.getByText('结果参数已修改',{exact:true})).toBeVisible()
  await expect.poll(async()=>{const draft=(await ws(api,session)).draft;return draft?.versionId===replaced.versionId?'':draft?.versionId??''}).not.toBe('')
  const modified=(await ws(api,session)).draft
  expect(modified.parentVersionId).toBe(replaced.versionId)

  await control.enqueueLlm({id:'activate-rss',expectedPrompt:'激活替换版本',turns:[
    {kind:'tool_call',call_id:'activate-rss',name:'activate_model_version',args:{versionId:Number(modified.versionId),expectedDraftVersionId:Number(modified.versionId)}},
    {kind:'text',text:'替换版本已激活'},
  ]})
  await send(page,'激活替换版本')
  await expect.poll(async()=>(await ws(api,session)).active?.versionId).toBe(modified.versionId)
  await control.enqueueLlm({id:'rollback-rss',expectedPrompt:'回退到最初版本',turns:[
    {kind:'tool_call',call_id:'rollback-rss',name:'rollback_model_version',args:{versionId:Number(uploaded.versionId),expectedDraftVersionId:Number(modified.versionId)}},
    {kind:'text',text:'已回退模型指针'},
  ]})
  await send(page,'回退到最初版本')
  await expect.poll(async()=>(await ws(api,session)).active?.versionId).toBe(uploaded.versionId)
  expect((await (await api.get(`/api/tasks/${oldTask.id}`)).json()).modelVersionId).toBe(uploaded.versionId)
  const versions=await (await api.get(`/api/sessions/${session}/versions`)).json()
  expect(versions.versions).toHaveLength(6)
  const versionResponse=await api.get(`/api/versions/${uploaded.versionId}`)
  expect(versionResponse.status()).toBe(200)
  expect((await versionResponse.json()).versionId).toBe(uploaded.versionId)
})
