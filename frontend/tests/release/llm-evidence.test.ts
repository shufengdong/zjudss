import { describe, expect, test } from 'vitest'

import { resolveLlmEvidence } from '../../../integration-tests/release/llm-evidence'

describe('release LLM evidence', () => {
  test.each([
    [{}, { provider: 'deepseek', model: 'deepseek-v4-pro' }],
    [{ LLM_PROVIDER: 'deepseek', LLM_MODEL: '   ' }, { provider: 'deepseek', model: 'deepseek-v4-pro' }],
    [{ LLM_PROVIDER: 'bailian', LLM_MODEL: 'qwen3.6-flash' }, { provider: 'bailian', model: 'qwen3.6-flash' }],
    [{ LLM_PROVIDER: ' bailian ', LLM_MODEL: '   ' }, { provider: 'bailian', model: 'qwen3.6-flash' }],
  ])('normalizes provider and model from %#', (environment, expected) => {
    expect(resolveLlmEvidence(environment)).toEqual(expected)
  })

  test('rejects an unsupported provider instead of mislabeling evidence', () => {
    expect(() => resolveLlmEvidence({ LLM_PROVIDER: 'other' })).toThrow('unsupported_llm_provider:other')
  })
})
