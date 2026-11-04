import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { createChartView, getChartCapabilities, previewChart } from '../../api/client'
import { chartCapabilities } from '../../test/fixtures'
import ChartBuilder from './ChartBuilder'
import { useChartCapabilities } from '../../hooks/useChartCapabilities'
import { useChartPreview } from '../../hooks/useChartPreview'
import type { ChartConfig } from '../../types'

vi.mock('../../api/client', () => ({
  ApiError: class extends Error {}, getChartCapabilities: vi.fn(), previewChart: vi.fn(), createChartView: vi.fn(),
}))
vi.mock('./ChartPreview', () => ({ default: ({error}:{error:string|null}) => error?<div role="alert">{error}</div>:<div aria-label="图表预览"/> }))

const capabilities = chartCapabilities()

beforeEach(() => {
  vi.clearAllMocks()
  vi.mocked(getChartCapabilities).mockResolvedValue(capabilities as never)
  vi.mocked(previewChart).mockResolvedValue({ kind: 'line', viewId: null } as never)
  vi.mocked(createChartView).mockResolvedValue({ view: { id: '9' }, replayed: false } as never)
})

it('builds list components, gates save on a fresh preview, and invalidates it on style changes', async () => {
  const saved = vi.fn()
  render(<ChartBuilder datasetId="7" sourceName="result" onSaved={saved} notify={vi.fn()} />)
  expect(await screen.findByRole('option', { name: 'v_mag[0] · A' })).toBeInTheDocument()
  expect(screen.getByRole('option', { name: /wide/ })).toBeDisabled()
  expect(screen.getByRole('button', { name: '保存图表' })).toBeDisabled()
  await waitFor(() => expect(screen.getByRole('button', { name: '生成预览' })).toBeEnabled())
  fireEvent.click(screen.getByRole('button', { name: '生成预览' }))
  await waitFor(() => expect(previewChart).toHaveBeenCalled())
  expect(screen.getByRole('button', { name: '保存图表' })).toBeEnabled()
  fireEvent.click(screen.getByText('显示设置'))
  fireEvent.change(screen.getByLabelText('调色板'), { target: { value: 'monochrome' } })
  expect(screen.getByRole('button', { name: '保存图表' })).toBeDisabled()
})

it('hides line-only settings for histogram and keeps preview errors editable', async () => {
  vi.mocked(previewChart).mockRejectedValue(new Error('没有可展示数据'))
  render(<ChartBuilder datasetId="7" sourceName="result" onSaved={vi.fn()} notify={vi.fn()} />)
  await screen.findByRole('option', { name: 'v_mag[0] · A' })
  fireEvent.change(screen.getByLabelText('图表类型'), { target: { value: 'histogram' } })
  expect(screen.queryByLabelText('线宽')).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: '生成预览' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('没有可展示数据')
  expect(screen.getByLabelText('图表名称')).toBeEnabled()
})

it('keeps scatter X independent from its bounded Y measures', async () => {
  render(<ChartBuilder datasetId="7" sourceName="result" onSaved={vi.fn()} notify={vi.fn()} />)
  await screen.findByRole('option', { name: 'v_mag[0] · A' })
  fireEvent.change(screen.getByLabelText('图表类型'), { target: { value: 'scatter' } })
  await waitFor(() => expect(screen.getByRole('button', { name: '生成预览' })).toBeEnabled())
  fireEvent.click(screen.getByRole('button', { name: '生成预览' }))
  await waitFor(() => expect(previewChart).toHaveBeenCalled())
  const calls = vi.mocked(previewChart).mock.calls
  const config = calls[calls.length - 1]?.[1]
  expect(config).toMatchObject({ kind: 'scatter', x: { column: 'v_mag', path: [0] },
    measures: [{ column: 'v_mag', path: [0] }, { column: 'v_mag', path: [1] }, { column: 'v_mag', path: [2] }] })
})

it('isolates late capability and preview responses after source or config changes', async () => {
  let resolveOldCapability!:(value:unknown)=>void
  const oldCapability=new Promise(resolve=>{resolveOldCapability=resolve})
  vi.mocked(getChartCapabilities).mockImplementation(id=>id==='old'?oldCapability as never:Promise.resolve({...capabilities,datasetId:'new'} as never))
  const capabilityHook=renderHook(({id})=>useChartCapabilities(id),{initialProps:{id:'old'}})
  capabilityHook.rerender({id:'new'})
  await waitFor(()=>expect(capabilityHook.result.current.data?.datasetId).toBe('new'))
  await act(async()=>resolveOldCapability({...capabilities,datasetId:'old'}))
  expect(capabilityHook.result.current.data?.datasetId).toBe('new')

  let resolveOldPreview!:(value:unknown)=>void
  const oldPreview=new Promise(resolve=>{resolveOldPreview=resolve})
  vi.mocked(previewChart).mockImplementation((_id,config)=>config.title==='old'?oldPreview as never:Promise.resolve({kind:'histogram',title:'new'} as never))
  const previewHook=renderHook(({id})=>useChartPreview(id),{initialProps:{id:'7'}})
  const base={version:1,kind:'histogram',presentation:{legend:{show:true,position:'auto',orientation:'auto'},palette:'colorblind',title:{show:true},axis:{showGrid:true}},value:{column:'v'},bins:10} as const
  await act(async()=>{void previewHook.result.current.run({...base,title:'old'});await previewHook.result.current.run({...base,title:'new'})})
  await waitFor(()=>expect(previewHook.result.current.model?.title).toBe('new'))
  await act(async()=>resolveOldPreview({kind:'histogram',title:'old'}))
  expect(previewHook.result.current.model?.title).toBe('new')
  previewHook.rerender({id:'8'})
  expect(previewHook.result.current.model).toBeNull()
  expect(previewHook.result.current.previewHash).toBeNull()
})

it('copies a frozen config as a new name and requires a new preview and operation id',async()=>{
  const config: ChartConfig={version:1,kind:'histogram',title:'原图',presentation:{legend:{show:true,position:'right',orientation:'vertical'},palette:'monochrome',title:{show:false},axis:{showGrid:false}},value:{column:'v_mag',path:[0],label:'自定义 A 相'},bins:12}
  vi.mocked(createChartView).mockRejectedValueOnce(new Error('网络中断')).mockRejectedValueOnce(new Error('网络中断')).mockResolvedValueOnce({view:{id:'9'},replayed:false} as never)
  render(<ChartBuilder datasetId="7" sourceName="result" initialConfig={config} initialName="原图 副本" onSaved={vi.fn()} notify={vi.fn()}/>)
  expect(await screen.findByDisplayValue('原图 副本')).toBeInTheDocument()
  expect(screen.getByRole('button',{name:'保存图表'})).toBeDisabled()
  await waitFor(()=>expect(screen.getByRole('button',{name:'生成预览'})).toBeEnabled())
  fireEvent.click(screen.getByRole('button',{name:'生成预览'}))
  await waitFor(()=>expect(screen.getByRole('button',{name:'保存图表'})).toBeEnabled())
  fireEvent.click(screen.getByRole('button',{name:'保存图表'}))
  await waitFor(()=>expect(createChartView).toHaveBeenCalledTimes(1))
  await waitFor(()=>expect(screen.getByRole('button',{name:'保存图表'})).toBeEnabled())
  fireEvent.click(screen.getByRole('button',{name:'保存图表'}))
  await waitFor(()=>expect(createChartView).toHaveBeenCalledTimes(2))
  await waitFor(()=>expect(screen.getByRole('button',{name:'保存图表'})).toBeEnabled())
  fireEvent.change(screen.getByLabelText('图表名称'),{target:{value:'原图 新副本'}})
  fireEvent.click(screen.getByRole('button',{name:'保存图表'}))
  await waitFor(()=>expect(createChartView).toHaveBeenCalledTimes(3))
  const requests=vi.mocked(createChartView).mock.calls.map(call=>call[1])
  expect(requests[0]).toEqual(expect.objectContaining({name:'原图 副本',clientOperationId:expect.any(String),config:expect.objectContaining({kind:'histogram',bins:12,value:expect.objectContaining({label:'自定义 A 相'}),presentation:expect.objectContaining({title:{show:false}})})}))
  expect(requests[1].clientOperationId).toBe(requests[0].clientOperationId)
  expect(requests[2].name).toBe('原图 新副本')
  expect(requests[2].clientOperationId).not.toBe(requests[0].clientOperationId)
})

it('removes a frozen custom label when the copied label input is cleared',async()=>{
  const config:ChartConfig={version:1,kind:'histogram',presentation:{legend:{show:true,position:'auto',orientation:'auto'},palette:'colorblind',title:{show:false},axis:{showGrid:true}},value:{column:'v_mag',path:[0],label:'旧标签'},bins:12}
  render(<ChartBuilder datasetId="7" sourceName="result" initialConfig={config} initialName="副本" onSaved={vi.fn()} notify={vi.fn()}/>)
  const label=await screen.findByLabelText('v_mag[0] 展示名')
  fireEvent.change(label,{target:{value:''}})
  await waitFor(()=>expect(screen.getByRole('button',{name:'生成预览'})).toBeEnabled())
  fireEvent.click(screen.getByRole('button',{name:'生成预览'}))
  await waitFor(()=>expect(previewChart).toHaveBeenCalled())
  const calls=vi.mocked(previewChart).mock.calls
  const request=calls[calls.length-1]?.[1]
  expect(request).toMatchObject({kind:'histogram',value:{column:'v_mag',path:[0]}})
  expect((request as Extract<ChartConfig,{kind:'histogram'}>).value).not.toHaveProperty('label')
})

it('shows only the effective measure label after switching to a single-measure kind',async()=>{
  render(<ChartBuilder datasetId="7" sourceName="result" onSaved={vi.fn()} notify={vi.fn()}/>)
  await screen.findByLabelText('v_mag[2] 展示名')
  fireEvent.change(screen.getByLabelText('图表类型'),{target:{value:'histogram'}})
  expect(screen.getByLabelText('v_mag[0] 展示名')).toBeInTheDocument()
  expect(screen.queryByLabelText('v_mag[1] 展示名')).not.toBeInTheDocument()
  expect(screen.queryByLabelText('v_mag[2] 展示名')).not.toBeInTheDocument()
})

it('keeps distinct frozen labels when scatter axes reference the same field',async()=>{
  const config: ChartConfig={version:1,kind:'scatter',presentation:{legend:{show:true,position:'auto',orientation:'auto'},palette:'colorblind',title:{show:false},axis:{showGrid:true}},x:{column:'v_mag',path:[0],label:'横轴'},measures:[{column:'v_mag',path:[0],label:'纵轴'}]}
  render(<ChartBuilder datasetId="7" sourceName="result" initialConfig={config} initialName="散点副本" onSaved={vi.fn()} notify={vi.fn()}/>)
  await waitFor(()=>expect(screen.getByRole('button',{name:'生成预览'})).toBeEnabled())
  fireEvent.click(screen.getByRole('button',{name:'生成预览'}))
  await waitFor(()=>expect(screen.getByRole('button',{name:'保存图表'})).toBeEnabled())
  fireEvent.click(screen.getByRole('button',{name:'保存图表'}))
  await waitFor(()=>expect(createChartView).toHaveBeenCalled())
  const calls=vi.mocked(createChartView).mock.calls
  expect(calls[calls.length-1]?.[1].config).toMatchObject({x:{label:'横轴'},measures:[{label:'纵轴'}]})
})

it('edits selected measure display names without changing physical field references',async()=>{
  render(<ChartBuilder datasetId="7" sourceName="result" onSaved={vi.fn()} notify={vi.fn()}/>)
  await screen.findByRole('option',{name:'v_mag[0] · A'})
  fireEvent.change(await screen.findByLabelText('v_mag[0] 展示名'),{target:{value:'A相'}})
  fireEvent.change(screen.getByLabelText('v_mag[1] 展示名'),{target:{value:'B相'}})
  fireEvent.change(screen.getByLabelText('v_mag[2] 展示名'),{target:{value:'C相'}})
  fireEvent.click(screen.getByRole('button',{name:'生成预览'}))
  await waitFor(()=>expect(screen.getByRole('button',{name:'保存图表'})).toBeEnabled())
  fireEvent.click(screen.getByRole('button',{name:'保存图表'}))
  await waitFor(()=>expect(createChartView).toHaveBeenCalled())
  const calls=vi.mocked(createChartView).mock.calls
  expect(calls[calls.length-1]?.[1].config).toMatchObject({measures:[
    {column:'v_mag',path:[0],label:'A相'},{column:'v_mag',path:[1],label:'B相'},{column:'v_mag',path:[2],label:'C相'},
  ]})
})
