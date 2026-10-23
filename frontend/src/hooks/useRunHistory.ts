import{useEffect,useRef,useState}from'react'
import{listRuns}from'../api/client'
import type{SimulationRun}from'../types'
export function useRunHistory(id:string|null,revision=0){const[runs,setRuns]=useState<SimulationRun[]>([]),generation=useRef(0);useEffect(()=>{const request=++generation.current;if(!id){setRuns([]);return}void listRuns(id).then(value=>{if(generation.current===request)setRuns(value)})},[id,revision]);return runs}
