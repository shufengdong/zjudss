import { defineConfig, loadEnv } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { apiProxyPath, normalizeApiBasePath } from './config/api-config.js'

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, '.', '')
  const apiBasePath = normalizeApiBasePath(env.VITE_API_BASE_PATH)
  const apiProxy: Record<string, unknown> = {
    target: env.VITE_API_TARGET || 'http://127.0.0.1:8080',
    changeOrigin: true,
    rewrite: (path: string) => apiProxyPath(path, apiBasePath),
  }
  if (env.VITE_E2E_PROXY_TOKEN) {
    apiProxy.configure = (proxy: any) => {
      proxy.on('proxyReq', (proxyRequest: any, request: any) => {
        const cookies: Record<string, string> = {}
        String(request.headers.cookie ?? '').split(';').forEach((part) => {
          const separator = part.indexOf('=')
          if (separator > 0) cookies[part.slice(0, separator).trim()] = decodeURIComponent(part.slice(separator + 1))
        })
        proxyRequest.setHeader('x-zjudss-proxy-token', env.VITE_E2E_PROXY_TOKEN)
        if (cookies['zjudss-e2e-subject']) {
          proxyRequest.setHeader('x-zjudss-subject', cookies['zjudss-e2e-subject'])
        }
        if (cookies['zjudss-e2e-tenant']) {
          proxyRequest.setHeader('x-zjudss-tenant', cookies['zjudss-e2e-tenant'])
        }
      })
    }
  }
  return {
    base: env.VITE_BASE_PATH || '/',
    plugins: [react(), tailwindcss()],
    server: {
      port: 5173,
      proxy: {
        [apiBasePath]: apiProxy,
      },
    },
  }
})
