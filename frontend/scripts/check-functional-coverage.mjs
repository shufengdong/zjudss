#!/usr/bin/env node

import fs from 'node:fs/promises'
import path from 'node:path'
import process from 'node:process'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { fileURLToPath } from 'node:url'

import YAML from 'yaml'

const REQUIRED_ARRAYS = ['layers', 'lanes', 'tests', 'assertions', 'fixtures']
export const REQUIRED_FEATURE_SCOPES = [
  'BOOT', 'AUTH', 'SESSION', 'MESSAGE', 'SSE', 'SCHED', 'UPLOAD',
  'INPUT-RSS', 'INPUT-BUNDLE', 'INPUT-DF', 'INFERENCE', 'WORKSPACE',
  'EXAMPLE', 'AGENT', 'STUDY', 'TASK', 'DATASET', 'ANALYSIS', 'VIEW',
  'COMPARE', 'HISTORY', 'STORAGE', 'UI', 'LEGACY',
]
const execFileAsync = promisify(execFile)
const ENUMERATION_ONLY_TESTS = new Set([
  'production_router_and_manifest_share_one_declaration',
  'manifest_allowed_tools_equal_runtime_registry',
  'frontend public enum contracts',
  'embedded_bundle_is_hash_verified_and_contains_only_compact_policy',
])

export function contractKeys(manifest) {
  return [
    ...(manifest.routes ?? []).map(({ method, path }) => `route:${method} ${path}`),
    ...(manifest.messageTypes ?? []).map((value) => `message:${value}`),
    ...(manifest.toolNames ?? []).map((value) => `tool:${value}`),
    ...(manifest.skillIds ?? []).map((value) => `skill:${value}`),
    ...(manifest.uploadSources ?? []).map((value) => `upload:${value}`),
    ...(manifest.visualizationKinds ?? []).map((value) => `view:${value}`),
    ...(manifest.stableErrorCodes ?? []).map((value) => `error:${value}`),
  ].sort()
}

export async function loadProductionManifest(repoRoot) {
  const { stdout } = await execFileAsync('cargo', [
    'run', '--quiet', '--manifest-path', path.join(repoRoot, 'backend/Cargo.toml'),
    '--bin', 'contract-manifest',
  ], { cwd: repoRoot, maxBuffer: 4 * 1024 * 1024 })
  return JSON.parse(stdout)
}

export async function loadCatalog(catalogPath) {
  const value = YAML.parse(await fs.readFile(catalogPath, 'utf8'))
  if (!Array.isArray(value)) throw new Error('functional coverage catalog must be an array')
  return value
}

export async function loadRouteEvidence(evidencePath) {
  try {
    return (await fs.readFile(evidencePath, 'utf8'))
      .split(/\r?\n/)
      .filter(Boolean)
      .map((line) => JSON.parse(line))
  } catch (error) {
    if (error?.code === 'ENOENT') return []
    throw error
  }
}

function routeMatches(contract, request) {
  const match = /^route:([A-Z]+) (.+)$/.exec(contract)
  if (!match || request?.method !== match[1] || request.status < 200 || request.status >= 400) return false
  const pattern = match[2]
    .split('/')
    .map((part) => /^\{[^}]+\}$/.test(part) ? '[^/]+' : part.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
    .join('/')
  return new RegExp(`^${pattern}$`).test(request.path)
}

function hasRouteEvidence(entry, contract, evidence) {
  return entry.tests.some((test) => test.runner === 'playwright' && evidence.some((result) =>
    result.passed === true
      && result.file === test.file
      && result.name === test.name
      && result.requests?.some?.((request) => routeMatches(contract, request))))
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
    const escaped = test.name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
    if (test.runner === 'rust') {
      return new RegExp(`(?:async\\s+)?fn\\s+${escaped}\\s*\\(`).test(content)
    }
    if (test.runner === 'release') {
      return new RegExp(`^#\\s*${escaped}\\s*$`, 'm').test(content)
    }
    return new RegExp(`(?:test|it)(?:\\.each\\([\\s\\S]{0,500}?\\))?\\s*\\(\\s*['\"\\x60]${escaped}`).test(content)
  } catch {
    return false
  }
}

async function testSourceHasUnexpectedSkip(repoRoot, test) {
  try {
    const content = await fs.readFile(path.join(repoRoot, test.file), 'utf8')
    return /#\s*\[ignore(?:\s*=|\s*\])|\b(?:test|it|describe)\.(?:skip|fixme)\s*\(/.test(content)
  } catch {
    return false
  }
}

function runnerCoversLane(runner, lane) {
  return lane === 'release' ? runner === 'release' : runner !== 'release'
}

export async function validateCatalog(entries, repoRoot, manifest = null, options = {}) {
  const requiredLane = options.requiredLane ?? null
  const routeEvidence = options.routeEvidence ?? []
  const counts = new Map()
  for (const entry of entries) counts.set(entry.id, (counts.get(entry.id) ?? 0) + 1)
  const duplicateIds = [...counts.entries()]
    .filter(([, count]) => count > 1)
    .map(([id]) => id)
    .sort()
  const duplicateSet = new Set(duplicateIds)
  const missing = []
  const unexpectedSkips = []
  let validEntries = 0
  let applicableEntries = 0
  let coveredEntries = 0

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
    if (entry?.contracts !== undefined && !Array.isArray(entry.contracts)) {
      errors.push('contracts must be an array')
    }
    if (entry?.status === 'manual-only' || entry?.lanes?.includes?.('manual-only')) {
      errors.push('manual-only entries are forbidden')
    }
    if (entry?.skip) {
      const reason = typeof entry.skip === 'object' ? entry.skip.reason : entry.skipReason
      if (typeof reason !== 'string' || reason.trim().length === 0) {
        errors.push('skip requires a non-empty reason')
      } else {
        errors.push(`skipped entry is uncovered: ${reason.trim()}`)
      }
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
        if (requiredLane && await testSourceHasUnexpectedSkip(repoRoot, test)) {
          unexpectedSkips.push(`${test.file}::${test.name}`)
        }
        if (entry?.contracts?.length && ENUMERATION_ONLY_TESTS.has(test.name)) {
          errors.push(`enumeration-only test cannot prove contract behavior ${test.file}::${test.name}`)
        }
      }
    }
    if (options.requireRouteEvidence && Array.isArray(entry?.contracts)) {
      for (const contract of entry.contracts.filter((value) => value.startsWith('route:'))) {
        if (!hasRouteEvidence(entry, contract, routeEvidence)) {
          errors.push(`route contract lacks successful runtime evidence ${contract}`)
        }
      }
    }
    const applies = requiredLane !== null
      && Array.isArray(entry?.lanes)
      && entry.lanes.includes(requiredLane)
    if (applies) {
      applicableEntries += 1
      if (!entry.tests?.some?.((test) => runnerCoversLane(test.runner, requiredLane))) {
        errors.push(`${requiredLane} lane requires a ${requiredLane === 'release' ? 'release' : 'PR'} runner test`)
      }
    }
    const id = typeof entry?.id === 'string' && entry.id ? entry.id : '<missing-id>'
    missing.push(...errors.map((error) => `${id}: ${error}`))
    if (errors.length === 0 && !duplicateSet.has(id)) {
      validEntries += 1
      if (applies) coveredEntries += 1
    }
  }

  const allContracts = manifest ? contractKeys(manifest) : null
  const missingFeatureScopes = requiredLane
    ? REQUIRED_FEATURE_SCOPES.filter((scope) => !entries.some((entry) => entry?.id?.startsWith(`${scope}-`)))
    : []
  const coveredFeatureScopes = requiredLane
    ? REQUIRED_FEATURE_SCOPES.length - missingFeatureScopes.length
    : null
  const contractCounts = new Map()
  for (const contract of entries.flatMap((entry) => entry?.contracts ?? [])) {
    contractCounts.set(contract, (contractCounts.get(contract) ?? 0) + 1)
  }
  const declaredContracts = new Set(contractCounts.keys())
  const missingContracts = allContracts
    ? allContracts.filter((contract) => !declaredContracts.has(contract))
    : null
  const allContractSet = new Set(allContracts ?? [])
  const duplicateContracts = [...contractCounts.entries()]
    .filter(([, count]) => count > 1)
    .map(([contract]) => contract)
    .sort()
  const unknownContracts = allContracts
    ? [...declaredContracts].filter((contract) => !allContractSet.has(contract)).sort()
    : []
  const contractCovered = allContracts ? allContracts.length - missingContracts.length : null
  const strictTotal = requiredLane
    ? applicableEntries + (allContracts?.length ?? 0) + REQUIRED_FEATURE_SCOPES.length
    : null
  const strictCovered = requiredLane
    ? coveredEntries + (contractCovered ?? 0) + coveredFeatureScopes
    : null
  const total = strictTotal ?? allContracts?.length ?? null
  const covered = strictCovered ?? contractCovered
  const coveragePercent = total === null
    ? null
    : total === 0 ? 100 : (covered / total) * 100
  const uniqueUnexpectedSkips = [...new Set(unexpectedSkips)].sort()
  const gatePassed = missing.length === 0
    && duplicateIds.length === 0
    && (missingContracts?.length ?? 0) === 0
    && duplicateContracts.length === 0
    && unknownContracts.length === 0
    && missingFeatureScopes.length === 0
    && uniqueUnexpectedSkips.length === 0
    && (!requiredLane || coveragePercent === 100)

  return {
    catalogEntries: entries.length,
    validEntries,
    missing,
    duplicateIds,
    contractTotal: allContracts?.length ?? null,
    applicableEntries: requiredLane ? applicableEntries : null,
    coveredEntries: requiredLane ? coveredEntries : null,
    total,
    covered,
    contractCovered,
    missingContracts,
    duplicateContracts,
    unknownContracts,
    requiredFeatureScopes: requiredLane ? REQUIRED_FEATURE_SCOPES : null,
    missingFeatureScopes,
    unexpectedSkips: uniqueUnexpectedSkips,
    coveragePercent,
    requiredLane,
    gatePassed,
  }
}

function htmlReport(report) {
  const escape = (value) => String(value)
    .replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')
  const gaps = [
    ...report.missing,
    ...(report.missingContracts ?? []).map((value) => `missing contract: ${value}`),
    ...report.duplicateContracts.map((value) => `duplicate contract: ${value}`),
    ...report.unknownContracts.map((value) => `unknown contract: ${value}`),
    ...report.missingFeatureScopes.map((value) => `missing feature scope: ${value}`),
    ...report.unexpectedSkips.map((value) => `unexpected skip: ${value}`),
  ]
  return `<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>功能覆盖报告</title><body><h1>功能覆盖报告</h1><dl><dt>Lane</dt><dd>${escape(report.requiredLane ?? 'catalog')}</dd><dt>Coverage</dt><dd>${escape(report.coveragePercent ?? 'n/a')}%</dd><dt>Covered</dt><dd>${escape(report.covered ?? 'n/a')} / ${escape(report.total ?? 'n/a')}</dd><dt>Gate</dt><dd>${report.gatePassed ? 'PASS' : 'FAIL'}</dd></dl><h2>缺口</h2><ul>${gaps.map((gap) => `<li>${escape(gap)}</li>`).join('')}</ul></body></html>\n`
}

async function main() {
  const here = path.dirname(fileURLToPath(import.meta.url))
  const repoRoot = path.resolve(here, '../..')
  const args = process.argv.slice(2)
  const requiredLane = args.includes('--require-release') ? 'release'
    : args.includes('--require-pr') ? 'pr' : null
  if (args.includes('--require-pr') && args.includes('--require-release')) {
    throw new Error('--require-pr and --require-release are mutually exclusive')
  }
  const checkManifest = args.includes('--check-manifest') || requiredLane !== null
  const catalogArg = args.find((arg) => !arg.startsWith('--'))
  const catalogPath = catalogArg ?? path.join(repoRoot, 'integration-tests/catalog/features.yaml')
  const manifest = checkManifest ? await loadProductionManifest(repoRoot) : null
  const report = await validateCatalog(
    await loadCatalog(catalogPath), repoRoot, manifest, {
      requiredLane,
      requireRouteEvidence: requiredLane === 'pr',
      routeEvidence: requiredLane === 'pr'
        ? await loadRouteEvidence(path.join(repoRoot, 'target/functional-coverage/e2e-routes.ndjson'))
        : [],
    },
  )
  const jsonPath = args.find((arg) => arg.startsWith('--report-json='))?.slice('--report-json='.length)
  const htmlPath = args.find((arg) => arg.startsWith('--report-html='))?.slice('--report-html='.length)
  if (jsonPath) {
    await fs.mkdir(path.dirname(path.resolve(jsonPath)), { recursive: true })
    await fs.writeFile(path.resolve(jsonPath), `${JSON.stringify(report, null, 2)}\n`)
  }
  if (htmlPath) {
    await fs.mkdir(path.dirname(path.resolve(htmlPath)), { recursive: true })
    await fs.writeFile(path.resolve(htmlPath), htmlReport(report))
  }
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`)
  if (!report.gatePassed) process.exitCode = 1
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main()
}
