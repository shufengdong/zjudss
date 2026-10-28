import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getDataResource, getDataResourceRows } from '../../api/client'
import DataResourceWorkbench from './DataResourceWorkbench'

vi.mock('../../api/client',()=>({getDataResource:vi.fn(),getDataResourceRows:vi.fn()}))

describe('DataResourceWorkbench',()=>{
  beforeEach(()=>{
    vi.mocked(getDataResource).mockResolvedValue({id:'r',sessionId:'s',displayName:'负荷预测',alias:'forecast',status:'live',declaredSchemaRef:null,sourceKind:'upload',schema:[{name:'value',dtype:'Float64',nullable:false}],schemaHash:'schema',contentHash:'hash',rowCount:2,sizeBytes:20,originalAvailable:true,sourceBatchId:'b',sourceModelVersionId:null,replacedByResourceId:null,lineage:{},createdAtMs:1})
    vi.mocked(getDataResourceRows).mockResolvedValueOnce({resourceId:'r',columns:['value'],rows:[[1]],nextCursor:'next',diagnostics:{}}).mockResolvedValueOnce({resourceId:'r',columns:['value'],rows:[[2]],nextCursor:null,diagnostics:{}})
  })
  it('reads metadata and appends opaque cursor pages for the stable resource id',async()=>{
    render(<DataResourceWorkbench resourceId="r" onClose={vi.fn()}/>)
    expect(await screen.findByText('负荷预测')).toBeInTheDocument()
    expect(screen.getByText('1')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button',{name:'加载更多'}))
    await waitFor(()=>expect(getDataResourceRows).toHaveBeenLastCalledWith('r',{cursor:'next',limit:100}))
    expect(screen.getByText('2')).toBeInTheDocument()
  })
})
