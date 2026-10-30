export interface LlmEvidence {
  provider: 'deepseek' | 'bailian'
  model: string
}

export function resolveLlmEvidence(
  environment: Record<string, string | undefined>,
): LlmEvidence {
  const provider = environment.LLM_PROVIDER?.trim() || 'deepseek'
  if (provider !== 'deepseek' && provider !== 'bailian') {
    throw new Error(`unsupported_llm_provider:${provider}`)
  }
  const defaultModel = provider === 'bailian' ? 'qwen3.6-flash' : 'deepseek-v4-pro'
  return {
    provider,
    model: environment.LLM_MODEL?.trim() || defaultModel,
  }
}
