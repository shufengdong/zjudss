# Bailian LLM Provider Design

## Intent

Allow the V4 conversational agent and managed-study script author to use Alibaba Cloud Model Studio (Bailian) models while preserving the existing DeepSeek configuration. The immediate target is `qwen3.6-flash`, selected through environment variables without exposing credentials.

## Goals and constraints

- Accept `LLM_PROVIDER=bailian`, `LLM_MODEL=qwen3.6-flash`, `BAILIAN_API_KEY`, and `BAILIAN_BASE_URL`.
- Use Bailian's OpenAI-compatible Chat Completions endpoint because the current rig version has no native Bailian provider and the application requires function calling.
- Keep `deepseek` as the default and retain its existing behavior.
- Require an explicit Bailian base URL so API keys are not silently sent to the wrong region; local `.env` may use the Beijing shared endpoint, while production documentation recommends a workspace-specific endpoint.
- Never include API keys in debug output, errors, reports, or logs.
- Preserve the existing tool set, prompts, retry behavior, REST/SSE contracts, and engine boundaries.

## Non-goals

- Add the native DashScope protocol, Responses API, or provider-specific model parameters.
- Change public routes, DTOs, tools, skills, database schema, or frontend behavior.
- Replace DeepSeek in release validation unconditionally; the release gate selects credentials from `LLM_PROVIDER`.

## Approaches

1. **Selected: dedicated `bailian` branch using rig `OpenAIConfig`.** Build a Chat Completions client with `BAILIAN_API_KEY` and `BAILIAN_BASE_URL`. This is explicit, keeps provider-specific environment names, and matches Bailian's compatibility contract.
2. Reuse the unfinished generic `openai` branch. This obscures which credential and endpoint are required and makes deployment diagnostics weaker.
3. Add a DashScope client dependency. This adds a second LLM transport without a capability needed by the current product.

## Design

Rename DeepSeek-specific factory and script-author implementation types to provider-neutral names. The factory continues to return the same rig Agent type; `deepseek` uses rig's native DeepSeek constructor, and `bailian` constructs an OpenAI-shaped client with an explicit base URL and calls `.chat(model)`. Missing or blank provider variables fail before any network request with a stable, secret-free configuration error.

The checked-in environment template and current deployment/testing documentation describe both providers. The real-dependency script defaults to DeepSeek for backward compatibility but respects an explicitly supplied `LLM_PROVIDER`, checks the corresponding variables, and redacts both provider credential variables from reports. No functional-catalog entry is needed because no public product contract changes.
