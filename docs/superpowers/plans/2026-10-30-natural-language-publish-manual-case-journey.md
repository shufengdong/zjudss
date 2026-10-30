# Natural-Language Publish and Manual Case Journey Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow the conversation Agent to publish the exact current model draft on an explicit natural-language request, and add an isolated, manually invoked real-dependency journey for every importable case using the tester-guide prompts.

**Architecture:** A new `publish_model_version` Rig tool reuses `VersionService::publish`, derives its idempotency key server-side from the durable request message and exact draft ref, and remains gated by `model-intake`. A separate Playwright configuration discovers current case manifests dynamically, drives each case through UI confirmations and natural-language publish/study turns, checkpoints a redacted report, and is reachable only through a dedicated shell entrypoint.

**Tech Stack:** Rust/Axum/rig/Diesel, React/TypeScript, Playwright, Vitest, SQLite, Snappy Parquet.

**Spec:** `docs/superpowers/specs/2026-10-30-natural-language-publish-manual-case-journey-design.md`

## Global Constraints

- Work directly on `master`; do not create a feature worktree.
- Every `tensoreval` process remains behind `backend/src/adapters/`; the manual journey uses production routes and never starts the binary itself.
- The new tool accepts only `expectedDraft` and optional description; no session ID, path, command, SQL, script, version ID, or caller-provided idempotency key.
- Case application/upload confirmation and study final confirmation remain explicit UI operations.
- The manual 144-case journey is excluded from `npm run test:e2e`, `scripts/test-release-integration`, PR coverage, and release coverage.
- The manual runner defaults to an isolated temporary SQLite database and `DATA_DIR`, and retains only a redacted report.
- Prompt changes are human-reviewed repository changes; the runner never asks an LLM to rewrite prompts automatically.
- The Agent bundle version becomes `v4.1.0`; runtime, documentation, tests, and report assertions use that exact version.
- Preserve and exclude unrelated user changes from every commit.

## Review Focus

- A request containing “生成脚本/生成结果” but not an explicit model-version publication must not call `publish_model_version`; Task 2 adds a negative scripted-agent test.
- A draft changed after `inspect_workspace` must return `draft_revision_conflict` without publishing the newer revision; Task 1 adds a direct tool test.
- Replaying the same durable message must return the same version without a second publication, while a new message may publish a later draft; Task 1 tests both idempotency boundaries.
- A long run interrupted after some cases must produce valid JSON and select only prior failed/unrun stable example IDs on resume; Task 3 tests checkpoint parsing and Task 4 exercises it through the runner configuration.
- Secrets, cookies, physical paths, raw stderr, and full scripts must not enter the manual report even on failure; Task 3 adds redaction tests and Task 4 limits captured evidence to stable codes and hashes.

---

### Task 1: Exact-draft natural-language publication tool

**Files:**
- Create: `backend/src/tools/publish_model_version.rs`
- Modify: `backend/src/tools/mod.rs`
- Modify: `backend/tests/workspace_tools.rs`

**Interfaces:**
- Consumes: `VersionService::publish(CurrentUser, session_id, PublishDraftRequest)` and `ToolServices::{principal,session_id,request_message_id,versions,loaded_skills}`.
- Produces: `PublishModelVersionTool`; JSON args `{ expectedDraft: { id: i32, revision: i64 }, description?: string|null }`; JSON output `{ versionId, description, contentHash, active, deduplicated, replayed }`.

- [ ] **Step 1: Write failing direct-tool tests**

Add `publish_model_version_requires_model_intake_and_publishes_only_the_exact_draft` and `publish_model_version_replays_by_message_and_rejects_stale_or_foreign_drafts` to `backend/tests/workspace_tools.rs`. Assert the skill gate, successful publication/activation, draft removal, optional description, no artifact/path fields, same-message replay, stale revision error code `draft_revision_conflict`, and cross-session/foreign draft failure without publication.

- [ ] **Step 2: Run the focused tests and verify failure**

Run: `cd backend && cargo test --features e2e-test --test workspace_tools publish_model_version -- --nocapture`

Expected: FAIL because `publish_model_version` and its args/tool type do not exist.

- [ ] **Step 3: Implement the tool**

Implement `Args { expected_draft: DraftRef, description: Option<String> }` and `PublishModelVersionTool` in `backend/src/tools/publish_model_version.rs`. Require `model-intake`; derive `client_publish_id` as `agent-publish:<request_message_id>:<draft_id>:<revision>`; call `VersionService::publish`; map `VersionError::DraftRevisionConflict` to `draft_revision_conflict`, missing/empty draft to stable workspace errors, and all other version failures without leaking internals. Return only the interface fields above.

- [ ] **Step 4: Register the tool and rerun focused tests**

Add the module and tool to `define_tool_registry!` in `backend/src/tools/mod.rs`.

Run: `cd backend && cargo test --features e2e-test --test workspace_tools publish_model_version -- --nocapture`

Expected: PASS for both new tests.

- [ ] **Step 5: Commit the tool slice**

```bash
git add backend/src/tools/mod.rs backend/src/tools/publish_model_version.rs backend/tests/workspace_tools.rs
git commit -m "feat: publish model drafts through agent tool"
```

### Task 2: Agent policy, bundle v4.1.0, and conversational behavior

**Files:**
- Modify: `backend/resources/agent/v4/system.md`
- Modify: `backend/resources/agent/v4/skills/model-intake.md`
- Modify: `backend/resources/agent/v4/skill-index.json`
- Modify: `backend/resources/agent/v4/manifest.json`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/config.rs`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/.env.example`
- Modify: `backend/tests/agent_knowledge.rs`
- Modify: `backend/tests/app_bootstrap.rs`
- Modify: `backend/tests/tool_manifest.rs`
- Modify: `backend/tests/scripted_agent.rs`
- Modify: `frontend/tests/release/release-config.test.ts`
- Modify: `integration-tests/release/semantic-assertions.ts`
- Modify: `integration-tests/release/real-dependencies.spec.ts`
- Modify: `scripts/test-release-integration`

**Interfaces:**
- Consumes: `PublishModelVersionTool` from Task 1.
- Produces: verified bundle `v4.1.0` with 23 tools; Agent policy requiring `load_agent_skill(model-intake) -> inspect_workspace -> publish_model_version` only on explicit publication intent.

- [ ] **Step 1: Write failing bundle and scripted-agent tests**

Update `backend/tests/agent_knowledge.rs` and `backend/tests/tool_manifest.rs` to require bundle `v4.1.0`, 23 unique tools, `publish_model_version` in both manifest and `model-intake.requiredTools`, and policy text that explicit publication is required. Add scripted tests:

- `explicit_publish_request_uses_exact_draft_and_activates_the_version`;
- `publish_then_study_request_continues_to_prepare_configuration_without_bypassing_confirmation`;
- `script_or_result_generation_words_do_not_implicitly_publish_a_draft`.

Update frontend release-contract assertions and semantic tool allow/side-effect sets to include the new tool.

- [ ] **Step 2: Run focused tests and verify failure**

Run: `cd backend && cargo test --features e2e-test --test agent_knowledge --test tool_manifest --test scripted_agent publish -- --nocapture`

Run: `cd frontend && npm test -- --run tests/release/release-config.test.ts src/release/semantic-assertions.test.ts`

Expected: FAIL on the old bundle version, tool count, hashes, and missing behavior.

- [ ] **Step 3: Update Agent policy and skill contract**

Change `system.md` and `model-intake.md` so imports and ordinary edits never imply publication; only a current explicit publication request authorizes the tool; stale drafts require reinspection; a combined publish-and-study request may continue to `study-planning`; upload and study confirmations remain UI-only. Add `publish_model_version` to the skill and manifest tool lists and bump model-intake skill version.

- [ ] **Step 4: Bump and rehash bundle v4.1.0**

Set `agentBundleVersion`/`bundleVersion` to `v4.1.0`. Recompute the model-intake content hash, skill-index hash, and system hash using SHA-256 over exact file bytes, then update `manifest.json`. Replace runtime/config/report assertions that identify the current bundle with `v4.1.0`, including `application/mod.rs`, `config.rs`, `.env.example`, contract loading, release journey, and release gate.

- [ ] **Step 5: Implement scripted behavior fixtures and semantic assertions**

Use `ScriptedAgentFactory` turns to call the registered tool with the inspected draft ref, verify exact tool ordering and persisted workspace state, continue into `prepare_study` only for the combined request, and prove the negative wording case leaves the draft unchanged. Mark `publish_model_version` as a side-effect tool in release semantic assertions so repeated publication is rejected unless explicitly expected.

- [ ] **Step 6: Run focused bundle and conversation tests**

Run: `cd backend && cargo test --features e2e-test --test agent_knowledge --test tool_manifest --test scripted_agent -- --nocapture`

Run: `cd frontend && npm test -- --run tests/release/release-config.test.ts src/release/semantic-assertions.test.ts`

Expected: PASS.

- [ ] **Step 7: Commit the bundle slice**

```bash
git add backend/resources/agent/v4 backend/src/application/mod.rs backend/src/config.rs backend/src/contract_manifest.rs backend/.env.example backend/tests/agent_knowledge.rs backend/tests/app_bootstrap.rs backend/tests/tool_manifest.rs backend/tests/scripted_agent.rs frontend/tests/release/release-config.test.ts frontend/src/release/semantic-assertions.test.ts integration-tests/release/semantic-assertions.ts integration-tests/release/real-dependencies.spec.ts scripts/test-release-integration
git commit -m "feat: authorize explicit natural language model publication"
```

### Task 3: Canonical tester-guide prompts and resumable report primitives

**Files:**
- Create: `integration-tests/manual-case-journeys/prompts.ts`
- Create: `integration-tests/manual-case-journeys/report.ts`
- Create: `frontend/tests/release/manual-case-journeys.test.ts`
- Modify: `docs/v4-tester-guide.md`

**Interfaces:**
- Produces: `CASE_PROMPTS: Readonly<Record<SupportedCaseFacade,string>>`; `promptForFacade(functionName)`; `selectExampleIds(examples, filters, resumeReport)`; `ManualCaseReport` and `checkpointCase(reportPath, result)`; `sanitizeCaseFailure(error, redactions)`.
- Consumes: stable example summaries/details and target functions `ds_newton_raphson_pf`, `ds_fault_study`, `ds_network_dynamics`, `ds_radial_execution_plan`, `ds_newton_raphson_execution_plan`, `ds_harmonic_execution_plan`.

- [ ] **Step 1: Write failing Vitest contract tests**

Create tests that require exactly six facade prompts, exact equality with the six fenced prompts in `docs/v4-tester-guide.md`, rejection of unsupported target functions, facade/example filtering, resume selection of only failed/unrun IDs, atomic checkpoint JSON that remains parseable after every result, and redaction of secrets, cookies, absolute paths, raw stderr, and script bodies.

- [ ] **Step 2: Run tests and verify failure**

Run: `cd frontend && npm test -- --run tests/release/manual-case-journeys.test.ts`

Expected: FAIL because the prompt/report modules do not exist.

- [ ] **Step 3: Implement prompt mapping and guide consistency**

Copy prompts 2, 6, 14, 16, 17, and 18 exactly from the guide into `prompts.ts`, keyed by target function. Add stable machine-readable markers around those six guide blocks so the test extracts only their text. Do not copy the 144 paths into code. Preserve `projection_diagnostic` initially because this task establishes the requested guide baseline; only change it after a real failure demonstrates the known catalog conflict.

- [ ] **Step 4: Implement selection, checkpoint, and sanitization helpers**

Use stable example IDs and target facade names for filters. Write checkpoints to a sibling temporary file and rename over the report. Store prompt SHA-256, stable error code, phase, duration, and counts; never store prompt response bodies, full scripts, raw stderr, credentials, cookie headers, or physical paths.

- [ ] **Step 5: Run the focused Vitest tests**

Run: `cd frontend && npm test -- --run tests/release/manual-case-journeys.test.ts`

Expected: PASS.

- [ ] **Step 6: Commit prompt and report primitives**

```bash
git add integration-tests/manual-case-journeys/prompts.ts integration-tests/manual-case-journeys/report.ts frontend/tests/release/manual-case-journeys.test.ts docs/v4-tester-guide.md
git commit -m "test: define manual case journey prompt contract"
```

### Task 4: Isolated manually invoked all-case Playwright journey

**Files:**
- Create: `integration-tests/manual-case-journeys/all-cases.spec.ts`
- Create: `integration-tests/manual-case-journeys/journey.ts`
- Create: `frontend/playwright.manual-cases.config.ts`
- Create: `scripts/test-manual-case-journeys`
- Modify: `frontend/tests/release/release-config.test.ts`

**Interfaces:**
- Consumes: Task 3 prompt/filter/report functions; production `/api/examples`, example detail/application, upload-batch, workspace, messages, study configuration, workflow, run, and dataset contracts.
- Produces: explicit command `./scripts/test-manual-case-journeys`; optional `CASE_JOURNEY_EXAMPLE_ID`, `CASE_JOURNEY_FACADE`, `CASE_JOURNEY_RESUME_REPORT`; report `target/manual-case-journeys/report.json`.

- [ ] **Step 1: Write failing configuration-isolation tests**

Extend `release-config.test.ts` to assert the normal and release Playwright configs cannot match `integration-tests/manual-case-journeys/all-cases.spec.ts`, the manual config matches only that directory with one worker and a multi-hour timeout, and neither `frontend/package.json` nor existing gate scripts invoke `test-manual-case-journeys`.

- [ ] **Step 2: Run the configuration test and verify failure**

Run: `cd frontend && npm test -- --run tests/release/release-config.test.ts`

Expected: FAIL because the manual config and entrypoint do not exist.

- [ ] **Step 3: Implement the dedicated shell entrypoint and Playwright config**

The shell script loads `backend/.env` unless disabled, validates the selected LLM variables plus executable `TENSOREVAL_BIN` and directory `SPARROWZZ_DIR`, creates an exact `mktemp` run directory beneath `target/manual-case-journeys`, assigns two free loopback ports, exports isolated `DATABASE_URL`/`DATA_DIR`, starts production backend and Vite through the manual Playwright config, and removes only that validated run directory on exit. Keep the incremental redacted report outside the removed directory.

- [ ] **Step 4: Implement case discovery and per-case UI state machine**

In `journey.ts`, implement these named phases with bounded waits and phase-specific stable failures: `discover`, `apply_example`, `confirm_candidates`, `publish_by_agent`, `prepare_study_by_agent`, `complete_configuration`, `confirm_workflow`, `wait_workflow`, `verify_result`, `cleanup_session`. For every importable example, fetch detail to read `projection.targetFunction`, open a fresh session, click the case UI action, explicitly confirm candidates, send the publication prompt and assert `load_agent_skill`, `inspect_workspace`, and `publish_model_version`, send the exact Task 3 study prompt and assert the requested facade plus `prepare_study`, complete unambiguous resource/Struct/result cards, click final confirmation, wait for a successful workflow, and open/read at least one result. Clean up successful temporary sessions only after checkpointing evidence.

- [ ] **Step 5: Implement resilient suite orchestration**

`all-cases.spec.ts` must catch each case failure, write an atomic checkpoint, continue to later selected cases, and fail once at the end with the failed stable IDs. A resume report selects previous failed and unrun IDs in a new isolated environment. The final report includes discovered/selected/passed/failed counts, per-facade totals, bundle version/hash, engine snapshot, prompt hashes, phase durations, and stable failure codes only.

- [ ] **Step 6: Verify discovery without running real dependencies**

Run: `cd frontend && NODE_PATH=./node_modules ./node_modules/.bin/playwright test --config=playwright.manual-cases.config.ts --list`

Expected: one manually named all-case test is listed; no server or case journey is executed.

- [ ] **Step 7: Run fast configuration and helper tests**

Run: `cd frontend && npm test -- --run tests/release/release-config.test.ts tests/release/manual-case-journeys.test.ts`

Expected: PASS.

- [ ] **Step 8: If real dependencies are configured, run one representative case only**

Run with the stable ID discovered for one representative radial case:

```bash
CASE_JOURNEY_EXAMPLE_ID=<stable-example-id> ./scripts/test-manual-case-journeys
```

Expected: one case passes and the report contains one selected/passed result. If its guide prompt fails because `projection_diagnostic` is not publishable, remove that resource from prompts 16/17 in both the canonical mapping and tester guide, rerun the focused consistency test, then rerun only the failed representative. Do not start all 144 cases without a separate explicit user request.

- [ ] **Step 9: Commit the manual journey slice**

```bash
git add integration-tests/manual-case-journeys frontend/playwright.manual-cases.config.ts frontend/tests/release/release-config.test.ts scripts/test-manual-case-journeys docs/v4-tester-guide.md
git commit -m "test: add opt-in all-case agent journey"
```

### Task 5: Public contract, current documentation, and functional traceability

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `docs/development-status.md`
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/api-design.md`
- Modify: `docs/frontend-design.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/v4-deployment.md`
- Modify: `docs/v4-tester-guide.md`
- Modify: `README.md`

**Interfaces:**
- Consumes: final tool and manual-runner behavior from Tasks 1–4.
- Produces: current documentation for bundle `v4.1.0`, 23 tools, natural-language exact-draft publication, manual invocation/resume/report semantics, and the distinction between release projection coverage and opt-in all-case semantic journeys.

- [ ] **Step 1: Add the public behavior evidence before editing prose**

In `features.yaml`, add `tool:publish_model_version` to the workspace/model lifecycle feature and point it to the exact-draft Rust and scripted-agent tests. Do not add the manual 144-case runner to `lanes: [pr]` or `lanes: [release]`, and do not use it to satisfy missing automated behavior coverage.

- [ ] **Step 2: Update authoritative current docs**

Document that publication is now allowed only via explicit natural-language Agent intent or the existing button, always uses the exact current draft revision, and auto-activates. Update tool lists/counts, bundle/default version, prompt/confirmation boundaries, deployment command, isolated storage behavior, filters, resume, report location, and the rule that a full 144-case success claim requires an explicitly completed manual run.

- [ ] **Step 3: Run documentation and coverage consistency checks**

Run: `rg -n "v4\.0\.0|22 个工具|22 个对话工具|22 个受控工具" README.md backend docs frontend integration-tests scripts -g '!docs/superpowers/plans/**' -g '!docs/superpowers/specs/**'`

Expected: no stale current-contract references outside historical plans/specs.

Run: `./scripts/check-functional-coverage --require-pr`

Expected: PASS with 100% PR traceability.

- [ ] **Step 4: Commit documentation and traceability**

```bash
git add integration-tests/catalog/features.yaml docs/development-status.md docs/agent-prompt-skills-design.md docs/api-design.md docs/frontend-design.md docs/integration-testing-design.md docs/v4-deployment.md docs/v4-tester-guide.md README.md
git commit -m "docs: describe natural language publish and manual case journey"
```

### Task 6: Full verification and completion evidence

**Files:**
- Modify only if verification exposes a defect in files owned by Tasks 1–5.

**Interfaces:**
- Consumes: all prior tasks.
- Produces: deterministic verification evidence; optional one-case real-dependency evidence; no claim that all 144 passed unless the opt-in runner actually completed.

- [ ] **Step 1: Run backend formatting and focused/full tests**

Run:

```bash
cd backend
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --features e2e-test
```

Expected: all commands exit 0.

- [ ] **Step 2: Run frontend tests and build**

Run:

```bash
cd frontend
npm test -- --run
npm run build
```

Expected: all tests pass and production build completes.

- [ ] **Step 3: Verify ordinary browser regression and manual-suite isolation**

Run: `cd frontend && npm run test:e2e`

Expected: ordinary E2E passes and does not execute any manual-case journey.

Run: `cd frontend && NODE_PATH=./node_modules ./node_modules/.bin/playwright test --config=playwright.manual-cases.config.ts --list`

Expected: manual suite lists exactly its opt-in test without executing it.

- [ ] **Step 4: Run coverage and diff checks**

Run:

```bash
./scripts/check-functional-coverage --require-pr
git diff --check
git status --short
```

Expected: coverage passes, no whitespace errors, and only intended changes remain.

- [ ] **Step 5: Request final code review and address findings**

Use `superpowers:requesting-code-review` against the implementation range. Fix all blocking findings, rerun the affected focused tests, then repeat Steps 1–4 if production code or shared test infrastructure changed.

- [ ] **Step 6: Commit verification fixes if any**

```bash
git add <only-files-changed-for-review-findings>
git commit -m "fix: address natural language publish review"
```
