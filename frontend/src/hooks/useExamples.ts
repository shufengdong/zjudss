import { useEffect, useState } from 'react'
import { listExamples } from '../api/client'
import type { ExampleSummary } from '../types'
export function useExamples(query:string){
  const [examples,setExamples]=useState<ExampleSummary[]>([])
  const [engineResourceSnapshot,setEngineResourceSnapshot]=useState('')
  const [loading,setLoading]=useState(false)
  useEffect(()=>{let live=true;setLoading(true);const timer=setTimeout(()=>void listExamples({query}).then(r=>{if(live){setExamples(r.examples);setEngineResourceSnapshot(r.engineResourceSnapshot)}}).finally(()=>{if(live)setLoading(false)}),200);return()=>{live=false;clearTimeout(timer)}},[query])
  return{examples,loading,engineResourceSnapshot}
}
