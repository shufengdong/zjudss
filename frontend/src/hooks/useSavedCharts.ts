import { useCallback, useEffect, useRef, useState } from 'react'
import { listChartViews } from '../api/client'
import type { ChartViewSummary } from '../types'

const mergeViews=(current:ChartViewSummary[],next:ChartViewSummary[])=>{
  const byId=new Map(current.map(view=>[view.id,view]));next.forEach(view=>byId.set(view.id,view));return[...byId.values()]
}

export function useSavedCharts(sessionId:string|null,externalRevision=0){
  const[state,setState]=useState<{sessionId:string|null;views:ChartViewSummary[];cursor:string|null;error:string|null}>({sessionId:null,views:[],cursor:null,error:null})
  const[loading,setLoading]=useState(false),[revision,setRevision]=useState(0),generation=useRef(0),paging=useRef(false)
  const reload=useCallback(()=>setRevision(value=>value+1),[])
  useEffect(()=>{const request=++generation.current;setState({sessionId,views:[],cursor:null,error:null});setLoading(Boolean(sessionId));paging.current=false
    if(!sessionId)return
    void listChartViews(sessionId).then(page=>{if(generation.current===request)setState({sessionId,views:page.views,cursor:page.nextCursor,error:null})})
      .catch(value=>{if(generation.current===request)setState({sessionId,views:[],cursor:null,error:value instanceof Error?value.message:'图表列表加载失败'})})
      .finally(()=>{if(generation.current===request)setLoading(false)})
  },[sessionId,externalRevision,revision])
  const loadMore=useCallback(async()=>{if(!sessionId||state.sessionId!==sessionId||!state.cursor||paging.current)return
    const request=generation.current,cursor=state.cursor;paging.current=true;setLoading(true)
    try{const page=await listChartViews(sessionId,{cursor});if(generation.current===request)setState(current=>({...current,views:mergeViews(current.views,page.views),cursor:page.nextCursor,error:null}))}
    catch(value){if(generation.current===request)setState(current=>({...current,error:value instanceof Error?value.message:'图表列表加载失败'}))}
    finally{if(generation.current===request){setLoading(false);paging.current=false}}
  },[sessionId,state.sessionId,state.cursor])
  const visible=state.sessionId===sessionId
  return{views:visible?state.views:[],nextCursor:visible?state.cursor:null,error:visible?state.error:null,loading:loading&&visible,reload,loadMore}
}
