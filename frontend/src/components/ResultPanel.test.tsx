import { fireEvent,render,screen } from '@testing-library/react'
import { expect,it,vi } from 'vitest'
import ResultPanel from './ResultPanel'

vi.mock('./workspace/WorkspacePanel',()=>({default:()=>null}))
vi.mock('./resources/DataResourcePanel',()=>({default:()=>null}))
vi.mock('./runs/RunHistory',()=>({default:()=>null}))
vi.mock('./datasets/SavedChartPanel',()=>({default:()=> <div>已保存图表面板</div>}))

it('exposes a fourth charts tab and opens the saved chart panel',()=>{
  const onTab=vi.fn()
  const props={open:true,overlay:false,onTab,onClose:vi.fn(),sessionId:'s',batches:[],batchActions:{} as never,runHistory:{} as never,datasetRevision:0,chartRevision:0,
    onReload:vi.fn(),notify:vi.fn(),onOpenDataset:vi.fn(),onOpenChart:vi.fn(),onLocateScript:vi.fn(),onRunDeleted:vi.fn(),onOpenDataFrame:vi.fn(),onInvalidateDataFrame:vi.fn()}
  const {rerender}=render(<ResultPanel {...props} tab="workspace"
  />)
  fireEvent.click(screen.getByRole('button',{name:'图表'}))
  expect(onTab).toHaveBeenCalledWith('charts')
  rerender(<ResultPanel {...props} tab="charts"/>)
  expect(screen.getByText('已保存图表面板')).toBeInTheDocument()
})
