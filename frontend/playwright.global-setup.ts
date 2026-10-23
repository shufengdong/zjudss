import { mkdir, writeFile } from 'node:fs/promises'
import { resolve } from 'node:path'

export default async function globalSetup() {
  const directory = resolve(process.cwd(), '../target/functional-coverage')
  await mkdir(directory, { recursive: true })
  await writeFile(resolve(directory, 'e2e-routes.ndjson'), '')
}
