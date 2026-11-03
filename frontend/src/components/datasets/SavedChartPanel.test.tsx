import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { deleteChartView, getChartCapabilities, getChartView, getChartViewData, listChartViews } from '../../api/client'
import { useSavedCharts } from '../../hooks/useSavedCharts'
import ChartViewWorkbench from './ChartViewWorkbench'
import SavedChartPanel from './SavedChartPanel'

vi.mock('../../api/client',()=>({
  ApiError:class extends Error{},listChartViews:vi.fn(),getChartView:vi.fn(),getChartViewData:vi.fn(),deleteChartView:vi.fn(),
  getChartCapabilities:vi.fn(),previewChart:vi.fn(),createChartView:vi.fn(),
}))
vi.mock('./ChartRenderer',()=>({default:()=> <div aria-label="已保存图表渲染"/>}))

const view=(id:string,extra={})=>({id,sessionId:'s',sourceDatasetId:'7',sourceName:'result',sourceDeleted:false,name:`chart-${id}`,kind:'line' as const,pointCount:3,sizeBytes:20,createdAt:'2026-11-04T00:00:00Z',...extra})

beforeEach(()=>{vi.clearAllMocks();vi.stubGlobal('confirm',vi.fn(()=>true))})

it('pages with stable-id de-duplication and clears cards immediately on session switch',async()=>{
  vi.mocked(listChartViews).mockImplementation((_id,q={})=>Promise.resolve(q.cursor?{views:[view('2'),view('3')],nextCursor:null}:{views:[view('1'),view('2')],nextCursor:'next'}))
  const hook=renderHook(({id})=>useSavedCharts(id,0),{initialProps:{id:'s'}})
  await waitFor(()=>expect(hook.result.current.views.map(item=>item.id)).toEqual(['1','2']))
  await act(async()=>hook.result.current.loadMore())
  expect(hook.result.current.views.map(item=>item.id)).toEqual(['1','2','3'])
  hook.rerender({id:'other'})
  expect(hook.result.current.views).toEqual([])
})

it('ignores a late page from the previous session',async()=>{
  let resolveOldPage!:(value:{views:ReturnType<typeof view>[];nextCursor:null})=>void
  const oldPage=new Promise<{views:ReturnType<typeof view>[];nextCursor:null}>(resolve=>{resolveOldPage=resolve})
  vi.mocked(listChartViews).mockImplementation((id,q={})=>{
    if(id==='s'&&q.cursor)return oldPage
    if(id==='other')return Promise.resolve({views:[view('other',{sessionId:'other'})],nextCursor:null})
    return Promise.resolve({views:[view('1')],nextCursor:'next'})
  })
  const hook=renderHook(({id})=>useSavedCharts(id,0),{initialProps:{id:'s'}})
  await waitFor(()=>expect(hook.result.current.views.map(item=>item.id)).toEqual(['1']))
  let pending!:Promise<void>
  act(()=>{pending=hook.result.current.loadMore()})
  hook.rerender({id:'other'})
  await waitFor(()=>expect(hook.result.current.views.map(item=>item.id)).toEqual(['other']))
  resolveOldPage({views:[view('late')],nextCursor:null})
  await act(async()=>pending)
  expect(hook.result.current.views.map(item=>item.id)).toEqual(['other'])
})

it('shows source deletion, opens by view id, and deletes with refresh',async()=>{
  vi.mocked(listChartViews).mockResolvedValue({views:[view('9',{sourceDeleted:true})],nextCursor:null})
  vi.mocked(deleteChartView).mockResolvedValue({viewId:'9',tombstone:{}})
  const open=vi.fn()
  render(<SavedChartPanel sessionId="s" revision={0} onOpen={open} notify={vi.fn()}/>)
  expect(await screen.findByText('来源已删除')).toBeInTheDocument()
  expect(screen.getByText(/result · line/)).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button',{name:'打开 chart-9'}));expect(open).toHaveBeenCalledWith('9')
  fireEvent.click(screen.getByRole('button',{name:'删除 chart-9'}))
  await waitFor(()=>expect(deleteChartView).toHaveBeenCalledWith('9'))
  await waitFor(()=>expect(listChartViews).toHaveBeenCalledTimes(2))
})

it('shows loading, retries an error, and reaches the empty state',async()=>{
  let rejectFirst!:(reason:Error)=>void
  vi.mocked(listChartViews).mockImplementationOnce(()=>new Promise((_resolve,reject)=>{rejectFirst=reject}))
    .mockResolvedValueOnce({views:[],nextCursor:null})
  render(<SavedChartPanel sessionId="s" revision={0} onOpen={vi.fn()} notify={vi.fn()}/>)
  expect(screen.getByText('加载中…')).toBeInTheDocument()
  await act(async()=>rejectFirst(new Error('暂时不可用')))
  fireEvent.click(await screen.findByRole('button',{name:'重试'}))
  expect(await screen.findByText('暂无已保存图表')).toBeInTheDocument()
})

it('loads and deletes a source-independent view without fetching its dataset',async()=>{
  vi.mocked(getChartView).mockResolvedValue({...view('9',{sourceDeleted:true}),contractVersion:'chart-model-v1',config:{kind:'histogram'},diagnostics:{}} as never)
  vi.mocked(getChartViewData).mockResolvedValue({kind:'histogram',viewId:'9',sourceDeleted:true} as never)
  vi.mocked(deleteChartView).mockResolvedValue({viewId:'9',tombstone:{}})
  render(<ChartViewWorkbench viewId="9" onClose={vi.fn()} onDeleted={vi.fn()} onOpenChart={vi.fn()} notify={vi.fn()}/>)
  expect(await screen.findByText('来源已删除')).toBeInTheDocument()
  expect(screen.getByLabelText('已保存图表渲染')).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button',{name:'删除图表'}))
  await waitFor(()=>expect(deleteChartView).toHaveBeenCalledWith('9'))
})

it('loads a live view frozen config into a copy builder',async()=>{
  const config={version:1,kind:'histogram',presentation:{legend:{show:true,position:'right',orientation:'vertical'},palette:'monochrome',title:{show:false},axis:{showGrid:false}},value:{column:'v'},bins:12} as const
  vi.mocked(getChartView).mockResolvedValue({...view('9'),contractVersion:'chart-model-v1',config,diagnostics:{}} as never)
  vi.mocked(getChartViewData).mockResolvedValue({kind:'histogram',viewId:'9',sourceDeleted:false} as never)
  vi.mocked(getChartCapabilities).mockResolvedValue({datasetId:'7',schemaHash:'h',rowCount:3,fields:[{column:'v',dtype:'Float64',nullable:false,unit:null,scalarType:'number',roles:['measure'],shape:{kind:'scalar'}}],templates:['line','bar','scatter','heatmap','boxplot','histogram'].map(kind=>({kind,enabled:true,reason:null})),limits:{maxFields:256,maxObservedPathsPerField:64,maxSeries:100,maxPoints:50000,maxCategories:5000,maxHeatmapCells:100000}} as never)
  render(<ChartViewWorkbench viewId="9" onClose={vi.fn()} onDeleted={vi.fn()} onOpenChart={vi.fn()} notify={vi.fn()}/>)
  fireEvent.click(await screen.findByRole('button',{name:'基于此图表新建'}))
  expect(await screen.findByDisplayValue('chart-9 副本')).toBeInTheDocument()
  expect(await screen.findByDisplayValue('12')).toBeInTheDocument()
})
