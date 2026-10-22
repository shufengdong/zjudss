import{useEffect,useState}from'react';import{getStorageUsage}from'../api/client';import type{StorageUsage}from'../types'
export function useStorageUsage(revision=0){const[value,setValue]=useState<StorageUsage|null>(null);useEffect(()=>{let live=true;void getStorageUsage().then(next=>{if(live)setValue(next)}).catch(()=>{});return()=>{live=false}},[revision]);return value}
