#!/usr/bin/env node

import fs from 'node:fs/promises'
import path from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

import YAML from 'yaml'

const REQUIRED_ARRAYS = ['layers', 'lanes', 'tests', 'assertions', 'fixtures']

export async function loadCatalog(catalogPath) {
  const value = YAML.parse(await fs.readFile(catalogPath, 'utf8'))
  if (!Array.isArray(value)) throw new Error('functional coverage catalog must be an array')
  return value
}

function headingAnchor(heading) {
  return heading
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s_-]/gu, '')
    .replace(/\s+/g, '-')
}

async function sourceExists(repoRoot, source) {
  const [relative, anchor] = String(source).split('#', 2)
  let content
  try {
    content = await fs.readFile(path.join(repoRoot, relative), 'utf8')
  } catch {
    return false
  }
  if (!anchor) return true
  return content
    .split(/\r?\n/)
    .filter((line) => /^#{1,6}\s+/.test(line))
    .map((line) => headingAnchor(line.replace(/^#{1,6}\s+/, '')))
    .includes(anchor)
}

async function testSymbolExists(repoRoot, test) {
  if (!test || typeof test.file !== 'string' || typeof test.name !== 'string') return false
  try {
    const content = await fs.readFile(path.join(repoRoot, test.file), 'utf8')
    return content.includes(test.name)
  } catch {
    return false
  }
}

export async function validateCatalog(entries, repoRoot, manifest = null) {
  const counts = new Map()
  for (const entry of entries) counts.set(entry.id, (counts.get(entry.id) ?? 0) + 1)
  const duplicateIds = [...counts.entries()]
    .filter(([, count]) => count > 1)
    .map(([id]) => id)
    .sort()
  const duplicateSet = new Set(duplicateIds)
  const missing = []
  let validEntries = 0

  for (const entry of entries) {
    const errors = []
    if (!entry || typeof entry.id !== 'string' || entry.id.length === 0) {
      errors.push('id must not be empty')
    }
    if (typeof entry?.source !== 'string' || !(await sourceExists(repoRoot, entry.source))) {
      errors.push(`source missing ${entry?.source ?? ''}`.trim())
    }
    if (typeof entry?.title !== 'string' || entry.title.trim().length === 0) {
      errors.push('title must not be empty')
    }
    for (const field of REQUIRED_ARRAYS) {
      if (!Array.isArray(entry?.[field])) errors.push(`${field} must be an array`)
    }
    for (const field of ['layers', 'lanes', 'tests', 'assertions']) {
      if (Array.isArray(entry?.[field]) && entry[field].length === 0) {
        errors.push(`${field} must not be empty`)
      }
    }
    if (Array.isArray(entry?.tests)) {
      for (const test of entry.tests) {
        if (!(await testSymbolExists(repoRoot, test))) {
          errors.push(`test symbol missing ${test?.file ?? ''}::${test?.name ?? ''}`)
        }
      }
    }
    const id = typeof entry?.id === 'string' && entry.id ? entry.id : '<missing-id>'
    missing.push(...errors.map((error) => `${id}: ${error}`))
    if (errors.length === 0 && !duplicateSet.has(id)) validEntries += 1
  }

  return {
    catalogEntries: entries.length,
    validEntries,
    missing,
    duplicateIds,
    contractTotal: manifest ? manifest.total : null,
    contractCovered: manifest ? manifest.covered : null,
    coveragePercent: manifest
      ? (manifest.total === 0 ? 100 : (manifest.covered / manifest.total) * 100)
      : null,
  }
}

async function main() {
  const here = path.dirname(fileURLToPath(import.meta.url))
  const repoRoot = path.resolve(here, '../..')
  const catalogPath = process.argv[2] ?? path.join(repoRoot, 'integration-tests/catalog/features.yaml')
  const report = await validateCatalog(await loadCatalog(catalogPath), repoRoot)
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`)
  if (report.missing.length > 0 || report.duplicateIds.length > 0) process.exitCode = 1
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main()
}
