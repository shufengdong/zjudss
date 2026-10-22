import { useEffect,useState } from 'react'
import { ApiError,getDatasetView } from '../api/client';import type{DatasetViewData}from'../types'
export function useDatasetView(id:string|null){const[data,setData]=useState<DatasetViewData|null>(null),[error,setError]=useState<string|null>(null)
  useEffect(()=>{setData(null);setError(null);if(id)void getDatasetView(id).then(setData).catch(e=>{const detail=e instanceof ApiError&&e.details?`（${JSON.stringify(e.details)}）`:'';setError((e instanceof Error?e.message:'视图加载失败')+detail)})},[id]);return{data,error}}
