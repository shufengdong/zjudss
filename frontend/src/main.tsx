import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import './index.css'

console.info(`[version] zjudss frontend git = ${import.meta.env.VITE_GIT_HASH}`)

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
