import{useEffect,useState}from'react';import{listRunDatasets}from'../api/client';import type{DatasetMetadata}from'../types'
export function useTaskDatasets(id:string|null){const[datasets,setDatasets]=useState<DatasetMetadata[]>([]);useEffect(()=>{if(id)void listRunDatasets(id).then(setDatasets)},[id]);return datasets}
