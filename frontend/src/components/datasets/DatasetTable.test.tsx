import{render,screen}from'@testing-library/react';import{describe,expect,it,vi}from'vitest';import DatasetTable from'./DatasetTable'
describe('DatasetTable',()=>{it('uses schema order and renders null explicitly',()=>{
  render(<DatasetTable metadata={{id:'d',taskId:'t',analysisRunId:null,name:'result',rowCount:1,sizeBytes:10,columns:[{name:'b',dtype:'Utf8',nullable:true},{name:'a',dtype:'Int64',nullable:false}],views:[]}}
    page={{datasetId:'d',columns:['a','b'],rows:[[7,null]],nextCursor:null,diagnostics:{}}} loading={false} onMore={vi.fn()}/>)
  const headers=screen.getAllByRole('columnheader').map(v=>v.textContent);expect(headers).toEqual(['b','a']);expect(screen.getByText('—')).toBeInTheDocument()
})})
