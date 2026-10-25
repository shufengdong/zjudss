import{fireEvent,render,screen,within}from'@testing-library/react';import{beforeEach,describe,expect,it,vi}from'vitest';import type{ModelImport,ModelVersion}from'../../types';import WorkspacePanel from'./WorkspacePanel';import{listVersions}from'../../api/client'
vi.mock('../../api/client',()=>({listVersions:vi.fn().mockResolvedValue([]),publishDraft:vi.fn(),activateVersion:vi.fn(),previewVersionDeletion:vi.fn(),deleteVersion:vi.fn(),resolveModelImport:vi.fn(),discardModelImport:vi.fn()}))
const modelImport=(id:string,selected:boolean[]):ModelImport=>({id,sessionId:'s',clientImportId:`client-${id}`,sourceKind:'case',baseDraft:null,status:'awaiting_resolution',stages:{},diagnostics:{},candidates:selected.map((value,index)=>({id:`${id}-candidate-${index}`,kind:'table',displayName:`${id} 候选 ${index+1}`,tableName:`table_${index+1}`,schema:[],rowCount:1,selected:value})),errorCode:null,errorMessage:null})
describe('WorkspacePanel',()=>{
  beforeEach(()=>vi.mocked(listVersions).mockResolvedValue([]))
  it('enables explicit version generation only for a non-empty draft',()=>{const base={id:'w',sessionId:'s',active:null,activeModelImports:[]};const{rerender}=render(<WorkspacePanel sessionId="s" workspace={{...base,draft:null}} imports={[]} onReload={vi.fn()} notify={vi.fn()}/>);expect(screen.queryByText('生成版本')).not.toBeInTheDocument();rerender(<WorkspacePanel sessionId="s" workspace={{...base,draft:{id:'d',workspaceId:'w',baseVersionId:null,revision:1,diagnostics:{},tables:[{id:'t',tableName:'bus',artifactId:'a',schema:[],rowCount:1,contentHash:'h'}],createdAt:'x',updatedAt:'x'}}} imports={[]} onReload={vi.fn()} notify={vi.fn()}/>);expect(screen.getByText('生成版本')).toBeEnabled()})
  it('selects and clears every candidate for one import without changing another import',()=>{
    render(<WorkspacePanel sessionId="s" imports={[modelImport('first',[true,false]),modelImport('second',[true,true])]} onReload={vi.fn()} notify={vi.fn()}/>)
    const first=screen.getByText(/模型导入 #first/).closest('section')!
    const second=screen.getByText(/模型导入 #second/).closest('section')!
    const firstSelectAll=within(first).getByRole('checkbox',{name:'全选'}) as HTMLInputElement
    const secondSelectAll=within(second).getByRole('checkbox',{name:'全选'}) as HTMLInputElement

    expect(firstSelectAll).not.toBeChecked()
    expect(firstSelectAll.indeterminate).toBe(true)
    expect(secondSelectAll).toBeChecked()

    fireEvent.click(firstSelectAll)
    expect(within(first).getAllByRole('checkbox').every(input=>(input as HTMLInputElement).checked)).toBe(true)
    expect(secondSelectAll).toBeChecked()

    fireEvent.click(firstSelectAll)
    expect(within(first).getAllByRole('checkbox').every(input=>!(input as HTMLInputElement).checked)).toBe(true)
    expect(secondSelectAll).toBeChecked()
  })
  it('opens every active and draft table through named buttons',()=>{
    const open=vi.fn(),base={id:'w',sessionId:'s',activeModelImports:[]}
    render(<WorkspacePanel sessionId="s" workspace={{...base,active:{id:'v1',workspaceId:'w',contentHash:'vh',sourceSummary:{},engineContractVersion:'e',createdAt:'x',tables:[{id:'a1',tableName:'bus',artifactId:'a',schema:[{}],rowCount:2,contentHash:'h'}]},draft:{id:'d1',workspaceId:'w',baseVersionId:'v1',revision:3,diagnostics:{},createdAt:'x',updatedAt:'x',tables:[{id:'d2',tableName:'load',artifactId:'a',schema:[{},{}],rowCount:4,contentHash:'h'}]}}} imports={[]} onReload={vi.fn()} notify={vi.fn()} onOpenDataFrame={open}/>)
    fireEvent.click(screen.getByRole('button',{name:'查看 bus'}))
    fireEvent.click(screen.getByRole('button',{name:'查看 load'}))
    expect(open).toHaveBeenNthCalledWith(1,{kind:'model-version',dataframeId:'a1',versionId:'v1',tableName:'bus'})
    expect(open).toHaveBeenNthCalledWith(2,{kind:'model-draft',dataframeId:'d2',draftId:'d1',revision:3,tableName:'load'})
  })
  it('expands historical versions for viewing without activation',async()=>{
    const version:ModelVersion={id:'v9',workspaceId:'w',contentHash:'vh',sourceSummary:{},engineContractVersion:'e',createdAt:'x',tables:[{id:'h1',tableName:'line',artifactId:'a',schema:[],rowCount:1,contentHash:'h'}]}
    vi.mocked(listVersions).mockResolvedValue([version])
    const open=vi.fn()
    render(<WorkspacePanel sessionId="s" workspace={{id:'w',sessionId:'s',active:null,draft:null,activeModelImports:[]}} imports={[]} onReload={vi.fn()} notify={vi.fn()} onOpenDataFrame={open}/>)
    const expand=await screen.findByRole('button',{name:'展开版本 v9 的表'})
    fireEvent.click(expand)
    const view=await screen.findByRole('button',{name:'查看 line'})
    fireEvent.click(view)
    expect(open).toHaveBeenCalledWith({kind:'model-version',dataframeId:'h1',versionId:'v9',tableName:'line'})
    expect(screen.queryByText('版本 v9 · 1 表')).not.toBeNull()
  })
})
