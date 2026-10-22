import{useEffect,useState}from'react';import{listTaskDatasets}from'../api/client';import type{DatasetMetadata}from'../types'
export function useTaskDatasets(id:string|null){const[datasets,setDatasets]=useState<DatasetMetadata[]>([]);useEffect(()=>{if(id)void listTaskDatasets(id).then(setDatasets)},[id]);return datasets}
