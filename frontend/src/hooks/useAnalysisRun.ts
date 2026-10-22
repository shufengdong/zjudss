import{useEffect,useState}from'react';import{getAnalysis}from'../api/client';import type{AnalysisRun}from'../types'
export function useAnalysisRun(id:string|null){const[value,setValue]=useState<AnalysisRun|null>(null);useEffect(()=>{if(id)void getAnalysis(id).then(setValue)},[id]);return value}
