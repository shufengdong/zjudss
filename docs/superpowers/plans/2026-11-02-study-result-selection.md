# Study Result Selection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users change an unconfirmed study's result set from either the configuration card or a natural-language message while preserving the fixed final confirmation gate.

**Architecture:** Keep `StudyConfigurationService::select_results` and the existing HTTP route as the single mutation boundary. Render the existing result selector for both mutable configuration states, and add one narrow `update_study_results` Agent tool that verifies current-session ownership, requires the planning skill, delegates to the same service, and publishes the revised durable configuration card. Bump the embedded bundle to `v4.2.0` because the public tool registry and planning policy change.

**Tech Stack:** Rust/Axum/rig/Diesel/SQLite, React/TypeScript/Vitest, Playwright, embedded Markdown/JSON Agent bundle.

**Spec:** `docs/superpowers/specs/2026-11-02-study-result-selection-design.md`

## Global Constraints

- Work directly on `master`; do not create a feature worktree.
- Natural language may update only the ordered result names of a mutable configuration in the current session.
- Neither the tool nor the card may confirm a configuration, generate a script, or create a workflow/run.
- Every mutation uses the exact `expectedRevision`; stale and terminal configurations remain `configuration_stale`.
- Public tool arguments contain stable IDs and controlled values only; no path, SQL, command, or arbitrary patch.
- New tool contracts require runtime/manifest equality, behavior coverage, feature-catalog traceability, and current design documentation.

## Review Focus

- A `ready_to_confirm` card with default results must remain editable and return to an updated summary without losing the final confirmation button; Task 1 component test.
- A tool call for a configuration owned by the same user but a different session must be rejected without mutation; Task 2 direct tool test.
- Stale revision, unknown/duplicate/empty result sets, and terminal configurations must preserve existing stable errors and data; Task 2 direct tool/application tests.
- A successful natural-language change must publish a revised durable `study-config` card but no workflow, run, or script; Task 3 scripted-agent test.
- The new tool must be skill-gated, side-effect classified, present exactly once in registry/manifest/catalog, and included in bundle hashes/version assertions; Task 3 contract tests.

---

### Task 1: Keep result selection editable in the final configuration card

**Files:**
- Modify: `frontend/src/components/messages/StudyConfigurationCard.tsx`
- Modify: `frontend/src/components/messages/StudyConfigurationCard.test.tsx`
- Modify: `frontend/src/components/studies/ResultSelectionCard.tsx` only if an accessible label or busy-state adjustment is required by the behavior test

**Interfaces:**
- Consumes: existing `selectStudyResults(id, expectedRevision, results)` and `useStudyConfiguration.mutate` revision-refresh behavior.
- Produces: `StudyConfigurationCard` renders `ResultSelectionCard` for `collecting_inputs | ready_to_confirm`; a successful save replaces the card with the returned revision and leaves `ConfigurationSummaryCard` available when ready.

- [ ] **Step 1: Write the failing component test**

Add `ready-to-confirm configurations can change results before final confirmation`. Start with revision `7`, selected `['bus_voltage']`, and available `bus_voltage`/`summary`. Assert both checkboxes and the final-confirmation button are visible, select `summary`, click `确认结果`, and assert `selectStudyResults('41', 7, ['bus_voltage', 'summary'])`. Resolve with revision `8` and assert the summary lists both results and final confirmation remains available.

- [ ] **Step 2: Run the focused test and verify RED**

Run: `cd frontend && npm test -- --run src/components/messages/StudyConfigurationCard.test.tsx`

Expected: FAIL because `ready_to_confirm` currently hides `ResultSelectionCard`.

- [ ] **Step 3: Implement the minimal render-state change**

Render `ResultSelectionCard` whenever the status is `collecting_inputs` or `ready_to_confirm`. Keep input/resource/Struct editors restricted to `collecting_inputs`, and keep `ConfigurationSummaryCard` restricted to `ready_to_confirm`.

- [ ] **Step 4: Run the focused test and verify GREEN**

Run: `cd frontend && npm test -- --run src/components/messages/StudyConfigurationCard.test.tsx`

Expected: all tests pass.

- [ ] **Step 5: Commit**

Commit: `fix: keep study result selection editable`

---

### Task 2: Add the current-session controlled result-update tool

**Files:**
- Create: `backend/src/tools/update_study_results.rs`
- Modify: `backend/src/tools/mod.rs`
- Modify: `backend/tests/workspace_tools.rs`

**Interfaces:**
- Consumes: `StudyConfigurationService::get`, `StudyConfigurationService::select_results`, `SelectStudyResultsRequest`, `ToolServices.session_id`, `ToolServices.writer.study_config`, and the `study-planning` skill gate.
- Produces: public `update_study_results` module and `UpdateStudyResultsTool` with arguments `{configurationId: string, expectedRevision: integer, results: string[]}` and output equal to the revised `StudyConfigurationView` JSON. Task 3 registers it only after the bundle and registry contract change together.

- [ ] **Step 1: Write failing direct-tool behavior tests**

Extend the existing study-tool fixture to prepare a real configuration and assert:

- missing `study-planning` returns `skill_required`;
- selecting `['bus_voltage', 'summary']` at the exact revision increments the revision, persists ordered selections, and writes the revised `study-config` message;
- a same-user tool bound to a different session cannot mutate the configuration;
- stale revision and terminal configuration return `configuration_stale`;
- empty, duplicate, and unknown results return `result_selection_invalid` without changing the persisted selection.

- [ ] **Step 2: Run the focused Rust tests and verify RED**

Run: `cd backend && cargo test --features e2e-test --test workspace_tools study_results -- --nocapture`

Expected: FAIL because the tool/module do not exist.

- [ ] **Step 3: Implement `UpdateStudyResultsTool`**

Define public `Args` with a decimal-string stable `configuration_id`, positive `expected_revision`, and a bounded non-empty string array `results`. Parse the ID at the tool boundary and return controlled not-found/invalid feedback instead of exposing a serde String/Integer mismatch. Mark the tool as side-effecting. In `call`:

1. require `study-planning`;
2. read the configuration for the authenticated principal;
3. reject a configuration whose `session_id` differs from `ToolServices.session_id` using the same not-found public semantics;
4. call `select_results` with the exact revision and ordered names;
5. serialize the returned view and publish a `StudyConfigCard` through the current writer;
6. return the serialized view without confirming or starting anything.

Expose the module for direct typed tests, but leave runtime registry insertion to Task 3 so each task ends with a green manifest/registry contract.

- [ ] **Step 4: Run the focused Rust tests and verify GREEN**

Run: `cd backend && cargo test --features e2e-test --test workspace_tools study_results -- --nocapture`

Expected: all focused direct-tool tests pass while the existing 23-tool runtime registry remains unchanged.

- [ ] **Step 5: Commit**

Commit: `feat: add controlled study result tool`

---

### Task 3: Teach the Agent to use the tool and publish bundle v4.2.0

**Files:**
- Modify: `backend/resources/agent/v4/system.md`
- Modify: `backend/resources/agent/v4/skills/study-planning.md`
- Modify: `backend/resources/agent/v4/skill-index.json`
- Modify: `backend/resources/agent/v4/manifest.json`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/config.rs`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/.env.example`
- Modify: `backend/tests/agent_knowledge.rs`
- Modify: `backend/tests/agent_factory.rs`
- Modify: `backend/tests/app_bootstrap.rs`
- Modify: `backend/tests/contract_manifest.rs`
- Modify: `backend/tests/tool_manifest.rs`
- Modify: `backend/tests/scripted_agent.rs`
- Modify: `frontend/tests/release/release-config.test.ts`
- Modify: `frontend/tests/release/manual-case-journeys.test.ts`
- Modify: `integration-tests/release/semantic-assertions.ts`
- Modify: `scripts/test-release-integration`

**Interfaces:**
- Consumes: Task 2 `update_study_results` registry entry and its exact parameter names.
- Produces: verified embedded bundle `v4.2.0` with 24 tools; `study-planning` version `1.4.0` requires `update_study_results`; system/skill policy tells the Agent to load the skill, use the exact current card ID/revision, and never confirm by tool.

- [ ] **Step 1: Write failing bundle and scripted-agent tests**

Update bundle assertions to require version `v4.2.0`, 24 tools, planning skill `1.4.0`, and the new tool in both runtime registry, manifest and planning-skill requirements. Add `natural_language_result_selection_updates_the_durable_card_without_starting_work`:

1. prepare a real ready configuration;
2. enqueue a turn that loads `study-planning` then calls `update_study_results` with its exact ID/revision and `['bus_voltage', 'summary']`;
3. post `bus_voltage，summary`;
4. assert the latest durable card has the incremented revision and both results;
5. assert there are no script, workflow, or run records and the assistant tells the user final confirmation is still required.

Add negative assertions that the tool is skill-gated and classified as a side-effect tool in release trace validation.

- [ ] **Step 2: Run focused tests and verify RED**

Run: `cd backend && cargo test --features e2e-test --test agent_knowledge --test scripted_agent --test contract_manifest --test agent_factory --test app_bootstrap -- --nocapture`

Expected: FAIL against bundle `v4.1.0`, 23 tools, and missing natural-language behavior.

- [ ] **Step 3: Update policy, bundle version, registry expectations, and hashes**

Add the narrow result-update rule to `system.md` and `study-planning.md`; do not grant input/parameter/final-confirm authority. Register Task 2's tool exactly once, add it to `study-planning.requiredTools` and `allowedTools`, set bundle `v4.2.0` and skill `1.4.0`, then recompute exact SHA-256 values for the planning skill, skill index, and system prompt. Update runtime defaults, embedded loaders, release assertions, semantic side-effect classification, and tool-count assertions consistently.

- [ ] **Step 4: Run focused backend and release-config tests and verify GREEN**

Run: `cd backend && cargo test --features e2e-test --test agent_knowledge --test scripted_agent --test contract_manifest --test agent_factory --test app_bootstrap --test tool_manifest -- --nocapture`

Run: `cd frontend && npm test -- --run tests/release/release-config.test.ts tests/release/manual-case-journeys.test.ts src/release/semantic-assertions.test.ts`

Expected: all focused tests pass with the verified `v4.2.0` bundle and 24-tool registry.

- [ ] **Step 5: Commit**

Commit: `feat: support conversational study result selection`

---

### Task 4: Add browser behavior coverage and synchronize current documentation

**Files:**
- Modify: `integration-tests/e2e/v4-managed-studies.spec.ts`
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `docs/frontend-design.md`
- Modify: `docs/api-design.md`
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/v4-deployment.md`
- Modify: `docs/development-status.md`

**Interfaces:**
- Consumes: Task 1 ready-state result editor and Task 3 scripted `update_study_results` behavior.
- Produces: PR traceability for both deterministic card editing and natural-language editing, plus current documentation for bundle `v4.2.0` and 24 tools.

- [ ] **Step 1: Write the failing Playwright behavior**

In the managed-study journey, once the configuration is `ready_to_confirm`, assert the `summary` checkbox and final-confirmation button coexist. Select `summary` through the card, submit `确认结果`, and assert the response/card revision increments and the summary shows `bus_voltage、summary`. In a fresh unconfirmed configuration, enqueue the scripted Agent call to `update_study_results`, send `bus_voltage，summary`, and assert the durable card updates while the session still has zero workflows/runs/scripts.

- [ ] **Step 2: Run the focused Playwright test and verify RED/GREEN state**

Run: `cd frontend && npm run test:e2e -- ../integration-tests/e2e/v4-managed-studies.spec.ts`

Expected before Tasks 1–3: FAIL at the missing ready-state checkbox/tool. Expected after Tasks 1–3: PASS.

- [ ] **Step 3: Update feature traceability and current docs**

Register `tool:update_study_results` exactly once under managed study configuration, list the Vitest/Rust/scripted-agent/Playwright behavior symbols, and update assertions for mutable ready cards and no natural-language final confirmation. Document the current two-entry interaction, error semantics, tool authority, bundle `v4.2.0`, 24 tools, and verification evidence.

- [ ] **Step 4: Run deterministic project verification**

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
```

Expected: every command passes without warnings or skipped required coverage.

- [ ] **Step 5: Commit**

Commit: `test: cover study result selection paths`

---

## Final Review

- Review the complete diff against the spec, especially current-session scoping, durable-card revision ordering, and the final-confirmation boundary.
- Fix every Critical/Important finding with a new RED→GREEN test, then rerun the full deterministic verification.
- Do not run the real-provider release lane or opt-in all-case semantic journey unless explicitly requested; their version/config assertions must still compile and remain current.
