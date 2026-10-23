import { createServer, request } from 'node:http'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawn } from 'node:child_process'
import { createInterface } from 'node:readline'

const here = dirname(fileURLToPath(import.meta.url))
const root = resolve(here, '../..')
const backend = join(root, 'backend')
const frontend = join(root, 'frontend')
const e2eRoot = await mkdtemp(join(tmpdir(), 'zjudss-e2e-'))
const controlToken = 'e2e-control-token'
const children = new Set()
let backendProcess
let stopping = false

function run(command, args, options = {}) {
  const child = spawn(command, args, { stdio: 'inherit', ...options })
  children.add(child)
  child.once('exit', () => children.delete(child))
  return child
}

function waitForExit(child, label) {
  return new Promise((resolvePromise, reject) => {
    child.once('error', reject)
    child.once('exit', (code, signal) => {
      if (code === 0) resolvePromise()
      else reject(new Error(`${label} exited with code=${code} signal=${signal}`))
    })
  })
}

async function buildBackend() {
  await waitForExit(
    run('cargo', ['build', '--features', 'e2e-test', '--bin', 'e2e-server'], { cwd: backend }),
    'cargo build',
  )
}

async function startBackend() {
  const child = spawn(join(backend, 'target/debug/e2e-server'), [], {
    cwd: backend,
    env: {
      ...process.env,
      E2E_ROOT: e2eRoot,
      E2E_BIND: '127.0.0.1:18080',
      E2E_CONTROL_TOKEN: controlToken,
    },
    stdio: ['ignore', 'pipe', 'inherit'],
  })
  backendProcess = child
  children.add(child)
  child.once('exit', () => children.delete(child))
  const lines = createInterface({ input: child.stdout })
  await new Promise((resolvePromise, reject) => {
    const timeout = setTimeout(() => reject(new Error('e2e backend readiness timeout')), 20_000)
    child.once('error', reject)
    child.once('exit', (code) => reject(new Error(`e2e backend exited before ready (${code})`)))
    lines.on('line', (line) => {
      process.stdout.write(`[e2e-server] ${line}\n`)
      if (line.startsWith('E2E_READY ')) {
        clearTimeout(timeout)
        resolvePromise()
      }
    })
  })
}

async function stopBackend() {
  const child = backendProcess
  backendProcess = undefined
  if (!child || child.exitCode !== null) return
  const exited = new Promise((resolvePromise) => child.once('exit', resolvePromise))
  child.kill('SIGTERM')
  await exited
}

async function restartBackend() {
  await stopBackend()
  await startBackend()
}

async function resetBackend() {
  await stopBackend()
  await rm(e2eRoot, { recursive: true, force: true })
  await startBackend()
}

function waitForHttp(url, deadline = Date.now() + 20_000) {
  return new Promise((resolvePromise, reject) => {
    const probe = () => {
      const req = request(url, (response) => {
        response.resume()
        if ((response.statusCode ?? 500) < 500) resolvePromise()
        else retry()
      })
      req.once('error', retry)
      req.end()
    }
    const retry = () => {
      if (Date.now() >= deadline) reject(new Error(`HTTP readiness timeout: ${url}`))
      else setTimeout(probe, 50)
    }
    probe()
  })
}

function startHarnessControl() {
  const server = createServer(async (req, res) => {
    if (req.headers['x-zjudss-e2e-token'] !== controlToken) {
      res.writeHead(401).end()
      return
    }
    if (req.method === 'POST' && req.url === '/restart') {
      try {
        await restartBackend()
        res.writeHead(204).end()
      } catch (error) {
        res.writeHead(500, { 'content-type': 'application/json' })
        res.end(JSON.stringify({ error: String(error) }))
      }
      return
    }
    if (req.method === 'POST' && req.url === '/reset') {
      try {
        await resetBackend()
        res.writeHead(204).end()
      } catch (error) {
        res.writeHead(500, { 'content-type': 'application/json' })
        res.end(JSON.stringify({ error: String(error) }))
      }
      return
    }
    res.writeHead(404).end()
  })
  return new Promise((resolvePromise, reject) => {
    server.once('error', reject)
    server.listen(15174, '127.0.0.1', () => resolvePromise(server))
  })
}

async function shutdown(code) {
  if (stopping) return
  stopping = true
  const active = [...children]
  const exits = active.map((child) => child.exitCode === null
    ? new Promise((done) => child.once('exit', done))
    : Promise.resolve())
  for (const child of active) child.kill('SIGTERM')
  await Promise.allSettled(exits)
  await rm(e2eRoot, { recursive: true, force: true })
  process.exit(code)
}

process.once('SIGINT', () => void shutdown(130))
process.once('SIGTERM', () => void shutdown(0))
process.once('uncaughtException', (error) => {
  console.error(error)
  void shutdown(1)
})
process.once('unhandledRejection', (error) => {
  console.error(error)
  void shutdown(1)
})

await buildBackend()
await startBackend()
await startHarnessControl()
run('npm', ['run', 'dev', '--', '--host', '127.0.0.1', '--port', '15173', '--strictPort'], {
  cwd: frontend,
  env: {
    ...process.env,
    VITE_API_TARGET: 'http://127.0.0.1:18080',
  },
})
await waitForHttp('http://127.0.0.1:15173')
process.stdout.write(`E2E_HARNESS_READY ${JSON.stringify({ e2eRoot })}\n`)
await new Promise(() => {})
