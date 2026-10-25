import type{DatasetMetadata,DatasetPage}from'../../types'
import DataFrameTable from'../dataframes/DataFrameTable'
export default function DatasetTable({metadata,page,loading,onMore}:{metadata:DatasetMetadata;page:DatasetPage|null;loading:boolean;onMore:()=>void}){
  return <DataFrameTable columns={metadata.columns} page={page} loading={loading} onMore={onMore}/>
}
