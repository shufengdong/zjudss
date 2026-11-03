# Managed Study Explanation and Collapsible Card Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore model-generated study-plan explanations before durable configuration cards and let users collapse long configuration cards without weakening the final confirmation gate.

**Architecture:** Extend the existing `prepare_study` tool with one bounded, required `explanation` string generated from the user request and the selected catalog facade. The tool validates it before any domain write, then persists the explanation plus a server-authored title/ID/footer immediately before the existing durable card; the ChatService continues suppressing provider tail text after successful preparation. Add presentation-only collapse state to `StudyConfigurationCard`, stored per configuration in `sessionStorage`, while configuration mutations and server state remain unchanged.

**Tech Stack:** Rust/Axum/rig, SQLite/Diesel, React/TypeScript, Vitest/Testing Library, Playwright.

**Spec:** `docs/superpowers/specs/2026-11-03-managed-study-ai-explanation-design.md`

## Global Constraints

- Work directly on `master`; do not create a feature worktree.
- All study authority remains in `StudyConfigurationService`; the explanation is display text, never a confirmation, script, run or contract mutation.
- `prepare_study.explanation` is required, trimmed, non-empty and at most 4,000 Unicode characters; reject it before creating a configuration.
- The server, not the model, appends the actual facade title, configuration ID and final-button instruction.
- Successful `prepare_study` continues to suppress later provider tail text.
- The final “确认并生成脚本” action remains a fixed UI gate.
- Collapse state is presentation-only, defaults expanded, uses `sessionStorage`, survives revision updates/session switching/page refresh in the same browser session, and auto-expands on configuration errors.
- Public tool argument, bundle version, skill version, hashes, behavior catalog and current design docs change together.

## Review Focus

- Blank and 4,001-character explanations must fail before a `study_configurations` row or message is created; Task 1 adds direct database/message assertions.
- Provider finalization failure after successful preparation must retain the detailed explanation and card without a generic failure message; Task 2 extends the scripted failure test.
- Explanation text must not be duplicated by the provider tail and must remain before the card by durable message ID; Tasks 1, 2 and 4 assert both properties.
- A stored collapsed preference must not conceal a stale/mutation error; Task 3 collapses during an in-flight mutation and verifies automatic expansion on failure.
- A collapsed ready card must not confirm or mutate anything, and expanding it must restore the current revision and final button; Tasks 3 and 4 exercise this path.

---

### Task 1: Make `prepare_study` persist a bounded model explanation before its card

**Files:**
- Modify: `backend/src/tools/prepare_study.rs`
- Modify: `backend/tests/workspace_tools.rs`

**Interfaces:**
- Consumes: `PrepareStudyTool`, `StudyConfigurationService::prepare`, `EventWriter::assistant`, `EventWriter::study_config`.
- Produces: `prepare_study::Args { facade_id, intent, derived_from_workflow_id, explanation }`; schema property `explanation` with `minLength: 1`, `maxLength: 4000`; stable direct-tool error `invalid_study_explanation` for trimmed-empty or over-limit input.

- [ ] **Step 1: Write the failing direct-tool behavior tests**

Extend `study_tools_search_explicit_functions_and_prepare_persisted_cards` with a distinctive multi-sentence explanation and assert that the assistant content is exactly:

```text
{trimmed model explanation}

已创建“牛顿-拉夫逊潮流”研究配置卡（ID: {id}）。请在卡片中检查并确认输入、参数和结果；只有点击“确认并生成脚本”后才会开始执行。
```

Assert the assistant message ID is lower than the `study-config` message ID. Add `prepare_study_rejects_invalid_explanation_before_creating_configuration` for whitespace and 4,001 characters; each call returns `invalid_study_explanation`, leaves the session with no assistant/study-config messages, and leaves the session's `study_configurations` count at zero.

- [ ] **Step 2: Run the focused test and verify RED**

Run: `cd backend && cargo test --features e2e-test --test workspace_tools prepare_study -- --nocapture`

Expected: compilation/test failure because `Args.explanation`, validation and the detailed message do not exist.

- [ ] **Step 3: Implement the minimal tool contract and ordering**

Add `pub explanation: String`, expose it as a required bounded JSON-schema string, trim and count Unicode characters before `StudyConfigurationService::prepare`, and reject invalid values with `DomainToolError::new("invalid_study_explanation", ...)`. After successful preparation, write the trimmed model explanation plus the server-authored footer as one ordinary assistant message, then write the existing durable `StudyConfigCard`. Do not change ChatService tail suppression or configuration state transitions.

- [ ] **Step 4: Run focused tests and formatting**

Run:

```bash
cd backend
cargo fmt --all -- --check
cargo test --features e2e-test --test workspace_tools prepare_study -- --nocapture
```

Expected: focused tests pass and the invalid-input assertions prove no mutation.

- [ ] **Step 5: Commit**

```bash
git add backend/src/tools/prepare_study.rs backend/tests/workspace_tools.rs
git commit -m "feat: preserve detailed study explanations"
```

### Task 2: Require detailed explanations in the Agent bundle and scripted flows

**Files:**
- Modify: `backend/resources/agent/v4/system.md`
- Modify: `backend/resources/agent/v4/skills/study-planning.md`
- Modify: `backend/resources/agent/v4/skill-index.json`
- Modify: `backend/resources/agent/v4/manifest.json`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/application/managed_workflows.rs`
- Modify: `backend/src/application/workflow_worker.rs`
- Modify: `backend/src/config.rs`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/agent_knowledge.rs`
- Modify: `backend/tests/app_bootstrap.rs`
- Modify: `backend/tests/scripted_agent.rs`
- Modify: `backend/tests/tool_manifest.rs`
- Modify: `frontend/src/release/semantic-assertions.test.ts`
- Modify: `frontend/tests/release/release-config.test.ts`
- Modify: `frontend/tests/release/manual-case-journeys.test.ts`

**Interfaces:**
- Consumes: Task 1's required `prepare_study.explanation` argument and existing `TurnOutcome.configuration_prepared` tail suppression.
- Produces: embedded Agent bundle `v4.3.0`; `study-planning@1.5.0`; prompt policy requiring a factual pre-execution explanation derived only from the user request and `search_study_facades`; all scripted `prepare_study` calls carry `explanation`.

- [ ] **Step 1: Write failing bundle and scripted-agent assertions**

Update `agent_knowledge` expectations to `v4.3.0` / `study-planning@1.5.0` and assert both system and skill text require `prepare_study.explanation`, its five content topics, and no claim of confirmation/execution. In `study_power_flow_searches_then_prepares_a_configuration_without_a_script`, pass a distinctive explanation and assert exactly one assistant message contains it before the card while the scripted provider tail is absent. Extend `prepared_configuration_survives_provider_finalization_failure` to assert the same detailed explanation and card survive with no generic failure message. Add `explanation` to every other scripted `prepare_study` call so unrelated scenarios exercise the new public argument.

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
cd backend
cargo test --features e2e-test --test agent_knowledge embedded_bundle_is_hash_verified_and_contains_only_compact_policy -- --nocapture
cargo test --features e2e-test --test scripted_agent study_power_flow_searches_then_prepares_a_configuration_without_a_script -- --nocapture
cargo test --features e2e-test --test scripted_agent prepared_configuration_survives_provider_finalization_failure -- --nocapture
```

Expected: failures on the old bundle versions/policy and missing explanation behavior.

- [ ] **Step 3: Update policy, versions and every runtime reference**

Tell the model to generate `explanation` after selecting a unique facade and before calling `prepare_study`. It must cover user goal, selected method/reason, inputs/parameters to inspect, planned results and the still-required final button; it must not claim the configuration, script or run already exists. Bump bundle `v4.2.0 → v4.3.0` and planning skill `1.4.0 → 1.5.0` in runtime defaults, embedded loaders, managed-workflow provenance, release fixtures and assertions. Keep the allowed tool count at 24.

- [ ] **Step 4: Recompute and install exact bundle hashes**

Compute SHA-256 for `skills/study-planning.md`, update its skill-index entry, hash the complete updated `skill-index.json`, hash `system.md`, and update `manifest.json`. Do not hand-edit approximate hashes.

- [ ] **Step 5: Run focused backend and release tests**

Run:

```bash
cd backend
cargo fmt --all -- --check
cargo test --features e2e-test --test workspace_tools prepare_study -- --nocapture
cargo test --features e2e-test --test scripted_agent -- --nocapture
cargo test --features e2e-test --test agent_knowledge -- --nocapture
cargo test --features e2e-test --test app_bootstrap -- --nocapture
cargo test --features e2e-test --test tool_manifest -- --nocapture
cd ../frontend
npm test -- --run src/release/semantic-assertions.test.ts tests/release/release-config.test.ts tests/release/manual-case-journeys.test.ts
```

Expected: all focused tests pass with bundle `v4.3.0`, planning skill `1.5.0` and 24 tools.

- [ ] **Step 6: Commit**

```bash
git add backend/resources/agent/v4 backend/src/application/mod.rs backend/src/application/managed_workflows.rs backend/src/application/workflow_worker.rs backend/src/config.rs backend/src/contract_manifest.rs backend/tests/agent_knowledge.rs backend/tests/app_bootstrap.rs backend/tests/scripted_agent.rs backend/tests/tool_manifest.rs frontend/src/release/semantic-assertions.test.ts frontend/tests/release/release-config.test.ts frontend/tests/release/manual-case-journeys.test.ts
git commit -m "feat: require model generated study explanations"
```

### Task 3: Make mutable study configuration cards collapsible and recoverable

**Files:**
- Modify: `frontend/src/components/messages/StudyConfigurationCard.tsx`
- Modify: `frontend/src/components/messages/StudyConfigurationCard.test.tsx`

**Interfaces:**
- Consumes: existing `StudyConfigMessage`, `useStudyConfiguration` configuration/error state and current mutation components.
- Produces: accessible title-bar toggle with `aria-expanded`; `sessionStorage` key `study-configuration:${configuration.id}:collapsed`; compact collapsed hint; automatic expansion and stored-preference clearing when `error` becomes non-null.

- [ ] **Step 1: Write failing component tests**

Clear `sessionStorage` in `beforeEach`. Add tests that assert:

- first render is expanded and the “收起研究配置” button has `aria-expanded="true"`;
- clicking it hides configuration controls, shows “已收起，展开后可继续配置或确认”, persists the key, and “展开研究配置” restores the same current revision/final button;
- unmount/remount with a higher message revision stays collapsed in the same browser session;
- a mutation rejected as `configuration_stale` after the user collapses during the request automatically reopens the card, exposes the alert and removes the collapsed preference;
- `confirmed` and `cancelled` compact terminal cards expose no collapse toggle.

- [ ] **Step 2: Run the component test and verify RED**

Run: `cd frontend && npm test -- --run src/components/messages/StudyConfigurationCard.test.tsx`

Expected: failures because the toggle, persistence and auto-expansion do not exist.

- [ ] **Step 3: Implement the minimal collapsible presentation**

Initialize collapse state from `sessionStorage`, defaulting to expanded. Render title, revision, a readable Chinese state label and the toggle in the header; render the existing error and action body only when expanded, except that an error effect first clears the stored collapsed flag and expands the body. Toggle writes/removes only its per-configuration key. Preserve the current terminal-card branch and all mutation callbacks unchanged.

- [ ] **Step 4: Run focused and full frontend unit tests**

Run:

```bash
cd frontend
npm test -- --run src/components/messages/StudyConfigurationCard.test.tsx
npm test -- --run
npm run build
```

Expected: all Vitest files pass and production TypeScript/Vite build succeeds.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/messages/StudyConfigurationCard.tsx frontend/src/components/messages/StudyConfigurationCard.test.tsx
git commit -m "feat: collapse study configuration cards"
```

### Task 4: Prove the combined conversation order in a real browser and publish current contracts

**Files:**
- Modify: `integration-tests/e2e/helpers/managedStudies.ts`
- Modify: `integration-tests/e2e/v4-managed-studies.spec.ts`
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `README.md`
- Modify: `backend/.env.example`
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/api-design.md`
- Modify: `docs/development-status.md`
- Modify: `docs/frontend-design.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/v4-deployment.md`

**Interfaces:**
- Consumes: Tasks 1–3's explanation/tool/bundle/collapse behavior.
- Produces: deterministic browser evidence and current docs/catalog for the public `prepare_study` argument, message ordering, bundle `v4.3.0`, `study-planning@1.5.0` and collapsible UI.

- [ ] **Step 1: Write/extend the failing Playwright journey**

Give every E2E `prepare_study` scenario a distinctive multi-sentence `explanation`. In the managed Newton journey, assert the explanation assistant DOM node precedes the study card, the provider tail is absent, and the card is initially expanded. Collapse it, assert configuration inputs/final button are hidden but the compact hint remains, refresh and switch away/back to prove the current browser session remembers it, then expand it and continue the existing result-edit/confirm/repair/result journey. Keep the QSTS mapping journey aligned with the new required argument.

- [ ] **Step 2: Run the focused Playwright file and verify RED**

Run: `cd frontend && npx playwright test ../integration-tests/e2e/v4-managed-studies.spec.ts`

Expected: scripted tool calls fail without the new argument and collapse/order assertions fail before implementation is complete.

- [ ] **Step 3: Update functional traceability and current docs**

Under `AGENT-001` and `MANAGED-STUDY-CONFIGURATION-001`, register the exact Rust/Vitest/Playwright symbols for bounded model explanation, durable ordering, provider-failure preservation and collapse recovery. Replace the old “deterministic acknowledgement” wording with the model-generated explanation plus server-authored footer contract. Add `error:invalid_study_explanation` exactly once if Task 1 exposes it as a stable code. Document bundle `v4.3.0`, skill `1.5.0`, required `explanation`, unchanged final-confirm authority, and `sessionStorage` collapse behavior. Update exact test/evidence counts only from command output.

- [ ] **Step 4: Run focused browser and functional coverage checks**

Run:

```bash
cd frontend
npx playwright test ../integration-tests/e2e/v4-managed-studies.spec.ts
cd ..
./scripts/check-functional-coverage --require-pr
```

Expected: managed-study browser tests and 100% PR coverage pass.

- [ ] **Step 5: Run full verification**

Run:

```bash
cd backend
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --features e2e-test
cd ../frontend
npm test -- --run
npm run build
npm run test:e2e
cd ..
./scripts/check-functional-coverage --require-pr
git diff --check
```

Expected: all Rust tests, Vitest, production build, all Playwright journeys and the 100% functional-coverage gate pass.

- [ ] **Step 6: Commit**

```bash
git add integration-tests/e2e/helpers/managedStudies.ts integration-tests/e2e/v4-managed-studies.spec.ts integration-tests/catalog/features.yaml README.md backend/.env.example docs/agent-prompt-skills-design.md docs/api-design.md docs/development-status.md docs/frontend-design.md docs/integration-testing-design.md docs/v4-deployment.md
git commit -m "test: cover managed study explanations and collapse"
```

- [ ] **Step 7: Restart the current local services and verify health**

Build `backend/target/release/zjudss`, resolve the exact existing repository-owned listeners before terminating them, restart the release backend and Vite frontend, then verify backend `/health` returns `ok`, frontend `/` returns HTTP 200, and backend startup reports the new commit and Agent bundle `v4.3.0`.
