import{fireEvent,render,screen,within}from'@testing-library/react';import{describe,expect,it,vi}from'vitest';import type{ModelImport}from'../../types';import WorkspacePanel from'./WorkspacePanel'
vi.mock('../../api/client',()=>({listVersions:vi.fn().mockResolvedValue([]),publishDraft:vi.fn(),activateVersion:vi.fn(),previewVersionDeletion:vi.fn(),deleteVersion:vi.fn(),resolveModelImport:vi.fn(),discardModelImport:vi.fn()}))
const modelImport=(id:string,selected:boolean[]):ModelImport=>({id,sessionId:'s',clientImportId:`client-${id}`,sourceKind:'case',baseDraft:null,status:'awaiting_resolution',stages:{},diagnostics:{},candidates:selected.map((value,index)=>({id:`${id}-candidate-${index}`,kind:'table',displayName:`${id} 候选 ${index+1}`,tableName:`table_${index+1}`,schema:[],rowCount:1,selected:value})),errorCode:null,errorMessage:null})
describe('WorkspacePanel',()=>{
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
})
