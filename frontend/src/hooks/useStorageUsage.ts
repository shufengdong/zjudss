import{useEffect,useState}from'react';import{getStorageUsage}from'../api/client';import type{StorageUsage}from'../types'
export function useStorageUsage(){const[value,setValue]=useState<StorageUsage|null>(null);useEffect(()=>{void getStorageUsage().then(setValue)},[]);return value}
