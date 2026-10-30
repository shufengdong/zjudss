# Bailian LLM Provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Bailian `qwen3.6-flash` support through its OpenAI-compatible Chat Completions endpoint while preserving DeepSeek.

**Architecture:** A provider-neutral factory selects DeepSeek or Bailian from `LlmConfig`. Bailian reads its own credential and explicit base URL, constructs rig's `OpenAIConfig`, and always chooses the Chat Completions wire; deployment and release validation select provider-specific environment requirements.

**Tech Stack:** Rust 2024, rig 0.45, shell, Vitest

**Spec:** `docs/superpowers/specs/2026-10-29-bailian-llm-provider-design.md`

## Global Constraints

- Work directly on `master`; do not create a worktree.
- Preserve all unrelated staged and unstaged user changes.
- Do not commit unless the user explicitly requests a commit.
- Do not log or serialize LLM credentials.
- Do not change public routes, DTOs, tools, skills, database schema, frontend behavior, or engine boundaries.

## Review Focus

- A blank `BAILIAN_API_KEY` must fail locally before a request and must not appear in the error.
- A blank `BAILIAN_BASE_URL` must fail locally before a request.
- Bailian must use Chat Completions rather than rig's OpenAI Responses default.
- DeepSeek must remain the default and keep its existing model default.
- Release reports and failure output must not expose either provider credential.

---

### Task 1: Provider-neutral agent construction

**Files:**
- Modify: `backend/src/agent/mod.rs`
- Modify: `backend/src/agent/factory.rs`
- Modify: `backend/src/agent/script_author.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/main.rs`
- Modify: `backend/tests/agent_factory.rs`
- Modify: `backend/tests/common/mod.rs`

**Interfaces:**
- Produces: `LlmAgentFactory::new(LlmConfig)`, `LlmScriptAuthor`, and a `bailian` provider branch using `BAILIAN_API_KEY` plus `BAILIAN_BASE_URL`.
- Preserves: `AgentFactory`, `ScriptAuthor`, `LlmConfig`, and all application behavior.

- [x] **Step 1: Write failing factory tests**

Add behavior tests that construct a Bailian agent with dummy credentials, reject missing/blank Bailian variables without leaking their values, and continue rejecting an unknown provider.

- [x] **Step 2: Run the focused test and verify RED**

Run: `cd backend && cargo test --test agent_factory bailian -- --test-threads=1`

Expected: FAIL because `bailian` is unsupported and provider-neutral types do not exist.

- [x] **Step 3: Implement the minimal provider branch and neutral names**

Use `rig::providers::openai::OpenAIConfig::new(key).with_base_url(url).client().chat(model)`. Validate non-empty values before construction. Rename only provider-specific implementation types; do not change public application protocols.

- [x] **Step 4: Run focused and backend agent tests**

Run: `cd backend && cargo test --test agent_factory -- --test-threads=1`

Expected: PASS.

### Task 2: Configuration and release gate

**Files:**
- Modify: `backend/.env`
- Modify: `backend/.env.example`
- Modify: `scripts/test-release-integration`
- Modify: `.github/workflows/release-integration.yml`
- Modify: `frontend/tests/ci/workflows.test.ts`

**Interfaces:**
- Consumes: provider names and variables from Task 1.
- Produces: provider-aware local configuration and release validation with credential redaction.

- [x] **Step 1: Write failing workflow tests**

Update the CI behavior test to require provider selection plus both provider-specific secrets/base URL inputs instead of a DeepSeek-only contract.

- [x] **Step 2: Run the focused workflow test and verify RED**

Run: `cd frontend && npm test -- --run tests/ci/workflows.test.ts`

Expected: FAIL because the workflow is still DeepSeek-only.

- [x] **Step 3: Implement provider-aware configuration**

Set local `.env` to Bailian and add the shared Beijing compatible endpoint. Document both providers in `.env.example`. Make the release script default to DeepSeek but respect `LLM_PROVIDER`, validate the selected provider's variables, and redact both possible secrets. Expose the Bailian secret/base URL to the release workflow without printing them.

- [x] **Step 4: Run focused workflow tests and shell syntax validation**

Run: `cd frontend && npm test -- --run tests/ci/workflows.test.ts && cd .. && bash -n scripts/test-release-integration`

Expected: PASS.

### Task 3: Current documentation and full verification

**Files:**
- Modify: `README.md`
- Modify: `docs/development-status.md`
- Modify: `docs/v4-deployment.md`
- Modify: `docs/integration-testing-design.md`

**Interfaces:**
- Consumes: the provider/environment contract from Tasks 1 and 2.
- Produces: current operational documentation for DeepSeek and Bailian.

- [x] **Step 1: Update current documentation**

Describe supported providers, required variables, regional endpoint matching, and the provider-selected real-dependency gate. Do not edit historical implementation plans.

- [ ] **Step 2: Run formatting and complete test gates**

Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --features e2e-test`

Run: `cd frontend && npm test -- --run && npm run build`

Run: `git diff --check`

Expected: all commands PASS with no warnings.
