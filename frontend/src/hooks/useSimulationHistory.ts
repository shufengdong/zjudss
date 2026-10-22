import{useEffect,useState}from'react';import{listTasks}from'../api/client';import type{SimulationTask}from'../types'
export function useSimulationHistory(id:string|null){const[tasks,setTasks]=useState<SimulationTask[]>([]);useEffect(()=>{setTasks([]);if(id)void listTasks(id).then(setTasks)},[id]);return tasks}
