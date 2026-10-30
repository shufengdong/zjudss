import fs from 'node:fs'

const reportPath = process.env.RELEASE_LLM_REPORT
if (!reportPath || !fs.existsSync(reportPath)) process.exit(0)

const credentials = [...new Set(
  [process.env.DEEPSEEK_API_KEY, process.env.BAILIAN_API_KEY]
    .flatMap((value) => {
      if (!value) return []
      const trimmed = value.trim()
      return trimmed && trimmed !== value ? [value, trimmed] : [value]
    }),
)]

function redact(value) {
  if (typeof value === 'string') {
    return credentials.reduce(
      (text, credential) => text.split(credential).join('<redacted>'),
      value,
    )
  }
  if (Array.isArray(value)) return value.map(redact)
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, redact(item)]))
  }
  return value
}

try {
  const report = JSON.parse(fs.readFileSync(reportPath, 'utf8'))
  fs.writeFileSync(reportPath, `${JSON.stringify(redact(report), null, 2)}\n`)
} catch {
  fs.rmSync(reportPath, { force: true })
  console.error('release_integration_llm_report_sanitization_failed')
  process.exit(1)
}
