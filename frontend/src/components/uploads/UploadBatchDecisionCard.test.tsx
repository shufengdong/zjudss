import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import type { UploadBatchActions, UploadBatchView } from '../../types'
import UploadBatchDecisionCard from './UploadBatchDecisionCard'

it('confirms a whole case switch with activation and history guidance', async () => {
  const batch: UploadBatchView = { id:'5',sessionId:'1',clientBatchId:'case',sourceKind:'case',status:'ready_to_confirm',modelPublishStatus:'ready',resourcePublishStatus:'ready',parseSettings:{},diagnostics:{},reservedBytes:0,expiresAtMs:Date.now()+10000,revision:1,
    candidates:[{id:'para',displayName:'para',proposedAlias:'para',purpose:'data_resource',selected:true,evidence:{}}] }
  const actions: UploadBatchActions = {updateParseSettings:vi.fn(),updateCandidates:vi.fn().mockResolvedValue({}),confirm:vi.fn().mockResolvedValue({}),discard:vi.fn(),addBatch:vi.fn(),watch:vi.fn()}
  const notify = vi.fn()
  render(<UploadBatchDecisionCard batch={batch} actions={actions} notify={notify} />)
  expect(screen.getByText(/历史模型版本、运行和结果保留/)).toBeInTheDocument()
  expect(screen.getByRole('combobox')).toBeDisabled()
  expect(screen.getByRole('textbox', {name:'para alias'})).toBeDisabled()
  fireEvent.click(screen.getByRole('button', {name:'确认切换案例'}))
  await waitFor(() => expect(actions.confirm).toHaveBeenCalledWith('5',expect.arrayContaining([expect.objectContaining({selected:true,purpose:'data_resource'})]),undefined))
  expect(notify).toHaveBeenCalledWith('案例已切换，模型版本已激活')
})

it('shows switch failures instead of claiming activation', async () => {
  const batch: UploadBatchView = { id:'6',sessionId:'1',clientBatchId:'case',sourceKind:'case',status:'ready_to_confirm',modelPublishStatus:'ready',resourcePublishStatus:'ready',parseSettings:{},diagnostics:{caseProjection:{title:'新案例'},caseSwitch:{previousCase:{batchId:4,title:'旧案例'},retiringResources:[{id:'r1',displayName:'参数',normalizedAlias:'para'}]}},reservedBytes:0,expiresAtMs:Date.now()+10000,revision:1,candidates:[] }
  const actions: UploadBatchActions = {updateParseSettings:vi.fn(),updateCandidates:vi.fn().mockResolvedValue({}),confirm:vi.fn().mockResolvedValue({status:'ready_to_confirm',diagnostics:[{message:'资源 para 与用户数据重名'}]}),discard:vi.fn(),addBatch:vi.fn(),watch:vi.fn()}
  const notify=vi.fn()
  render(<UploadBatchDecisionCard batch={batch} actions={actions} notify={notify} />)
  expect(screen.getByText('上一案例：旧案例')).toBeInTheDocument()
  expect(screen.getByText('将替换 1 个上一案例的配套数据资源。')).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button',{name:'确认切换案例'}))
  await waitFor(() => expect(notify).toHaveBeenCalledWith('资源 para 与用户数据重名'))
  expect(notify).not.toHaveBeenCalledWith('案例已切换，模型版本已激活')
})
