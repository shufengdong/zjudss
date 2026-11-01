# Run Progress Failure Diagnosis Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make failed ordinary run progress cards show the same persisted, sanitized engine logs and bounded LLM analysis as managed workflow cards, using shared backend and frontend abstractions.

**Architecture:** Extract failure analysis from `ScriptAuthor` into a shared `FailureAnalyzer`, persist a typed run failure envelope before best-effort enrichment, and expose it through the existing run detail API. Extract the workflow failure UI into `FailureDiagnosisPanel`; failed run progress cards load that run detail and poll only while analysis is pending.

**Tech Stack:** Rust, Axum, Diesel/SQLite JSON1, async-trait, Tokio, React, TypeScript, Vitest/Testing Library, Playwright.

**Spec:** `docs/superpowers/specs/2026-11-01-run-progress-failure-diagnosis-design.md`

## Global Constraints

- All tensoreval calls remain inside `backend/src/adapters/`; this work consumes existing `AdapterError` diagnostics only.
- Public data contains only stable IDs and controlled DTO fields; never expose server paths, commands, SQL, secrets, audit artifact IDs, or raw provider errors.
- Engine logs use the existing 64 KiB-per-stream sanitization boundary and preserve `totalBytes`, `sentBytes`, and `truncated`.
- Failure analysis is best effort with a 30-second server timeout; run failure persistence and durable failed progress happen before the model call.
- `simulation_runs.error_json` is enriched only from `analysisStatus=pending` to `complete|unavailable`; terminal state and base logs are not rewritten.
- Existing `engineOutput` records and `service_restarted` records remain readable without migration or backfill.
- Do not add a public route or change the durable `progress` payload shape.
- Ordinary runs do not gain repair history; workflow repair behavior remains unchanged.
- Work directly on `master`; preserve and exclude unrelated user changes from commits.

## Review Focus

- Captured stderr containing non-UTF-8 bytes, secrets, control characters, and absolute paths must remain readable but sanitized; Task 2 adds an API assertion for this input.
- A model call that hangs past 30 seconds must leave the run failed, logs readable, and analysis unavailable; Task 2 adds a paused-time timeout test.
- A process failure with stdout but empty stderr must show both the fallback diagnostic and stdout without allowing unsupported evidence; Tasks 1 and 2 add request/DTO tests.
- A restored historical progress message whose run has legacy `engineOutput` must render logs and stop polling; Task 4 adds the compatibility test.
- Rapidly switching or unmounting message cards must ignore stale run-detail responses and clear pending polling; Task 4 adds request-generation and cleanup tests.

---

### Task 1: Extract the shared failure analyzer

**Files:**
- Create: `backend/src/agent/failure_analyzer.rs`
- Modify: `backend/src/agent/script_author.rs`
- Modify: `backend/src/agent/mod.rs`
- Modify: `backend/src/testing/scripted_author.rs`
- Modify: `backend/src/application/workflow_worker.rs`
- Modify: `backend/src/application/managed_workflows.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/tests/common/mod.rs`
- Modify: `backend/tests/script_repair.rs`

**Interfaces:**
- Produces: `FailureAnalysisEvidence`, `FailureAnalysis`, and `FailureAnalysisRequest { current_script: String, sanitized_stdout: Option<String>, sanitized_stderr: String, diagnostic: Option<Value>, locked_contract: Option<LockedStudyContract>, inputs: Option<WorkflowInputSnapshot> }`.
- Produces: `#[async_trait] FailureAnalyzer::analyze_failure(&self, FailureAnalysisRequest) -> Result<FailureAnalysis, FailureAnalysisError>`.
- Produces: `validate_failure_analysis(&FailureAnalysis, sanitized_stderr: &str) -> Result<(), FailureAnalysisError>` for the persistence and API boundaries to reuse.
- Produces: `LlmFailureAnalyzer::from_factory(Arc<dyn AgentFactory>, usize) -> Self` and a separate test override in `E2eAppOptions`.
- Preserves: `ScriptAuthor::generate` and `ScriptAuthor::repair`; workflow generation/repair call sites do not depend on the analyzer implementation.

- [ ] **Step 1: Write failing analyzer contract tests**

Move the existing analysis parsing tests to `failure_analyzer.rs` and add `accepts_optional_workflow_context_for_ordinary_runs`. Assert valid evidence must start with `stderr:` and its excerpt must occur in `sanitized_stderr`; stdout-only text cannot be cited as stderr.

- [ ] **Step 2: Run the focused unit tests and verify the new module is missing**

Run: `cd backend && cargo test --features e2e-test failure_analyzer --lib`

Expected: FAIL because `failure_analyzer` and `FailureAnalyzer` are not defined.

- [ ] **Step 3: Implement the analyzer module and narrow `ScriptAuthor`**

Move the prompt, bounded JSON parser, evidence validation, retry loop, analysis DTOs, and analysis-specific error into `backend/src/agent/failure_analyzer.rs`. Keep script normalization/generation/repair in `script_author.rs`; do not duplicate `AgentFactory` calls or validation rules.

- [ ] **Step 4: Inject `Arc<dyn FailureAnalyzer>` independently**

Add `failure_analyzer` to `AppState`, `E2eAppOptions`, managed workflow construction, and `WorkflowWorker`. Production creates `LlmFailureAnalyzer` from the same `AgentFactory`; tests may provide the existing `ScriptedAuthor` as both interfaces. Update workflow requests to wrap `locked_contract` and `inputs` in `Some`, include sanitized stdout when available, and apply the same 30-second best-effort timeout without changing repair behavior.

- [ ] **Step 5: Run analyzer and workflow regression tests**

Run: `cd backend && cargo test --features e2e-test failure_analyzer --lib && cargo test --features e2e-test --test script_repair`

Expected: PASS, including existing generation/repair ordering and evidence validation.

- [ ] **Step 6: Commit the shared analyzer extraction**

```bash
git add backend/src/agent backend/src/application/workflow_worker.rs backend/src/application/managed_workflows.rs backend/src/application/mod.rs backend/src/testing/scripted_author.rs backend/tests/common/mod.rs backend/tests/script_repair.rs
git commit -m "refactor: share engine failure analysis"
```

### Task 2: Persist and enrich ordinary run failures

**Files:**
- Modify: `backend/src/application/diagnostics.rs`
- Modify: `backend/src/application/runs.rs`
- Modify: `backend/src/application/workflow_worker.rs`
- Modify: `backend/src/db/repositories/run.rs`
- Modify: `backend/src/db/mod.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/testing/scenario_engine.rs`
- Modify: `backend/tests/common/mod.rs`
- Modify: `backend/tests/script_repair.rs`
- Modify: `backend/tests/simulation_runs.rs`

**Interfaces:**
- Consumes: `FailureAnalyzer::analyze_failure(FailureAnalysisRequest)` from Task 1.
- Produces: `EngineLog { stream, text, total_bytes, sent_bytes, truncated }`, `FailureAnalysisStatus`, and `RunFailureDiagnosis { code, reason, stage, duration_ms, engine_logs, analysis_status, analysis }` in `application::diagnostics` with camelCase serde.
- Produces: `RunRepo::complete_failure_analysis(&UserId, run_id: i32, status: FailureAnalysisStatus, analysis: Option<&FailureAnalysis>) -> RepoResult<()>`, restricted to failed, live runs whose stored status is `pending`.
- Produces: `RunService` dependency `failure_analyzer: Arc<dyn FailureAnalyzer>`.

- [ ] **Step 1: Extend the failed-run test with the persisted base envelope**

In `failed_run_publishes_a_terminal_failed_progress_event`, use a captured stderr stream and assert the run becomes `failed` and its failed progress message exists before a blocked analyzer is released. Assert stored `error_json` already contains sanitized `engineLogs` and `analysisStatus: "pending"`.

- [ ] **Step 2: Add enrichment success, unavailable, timeout, and recovery tests**

Add tests named:

- `failed_run_persists_structured_analysis_after_logs_and_terminal_progress`
- `failed_run_keeps_logs_when_failure_analysis_is_unavailable`
- `failed_run_times_out_analysis_without_reopening_the_terminal_run`
- `restart_recovery_resolves_pending_run_analysis_without_changing_logs`

The timeout test uses Tokio paused time rather than a 30-second wall-clock wait. The recovery test inserts a failed pending envelope, invokes `recover_interrupted_runs`, and asserts only `analysisStatus` changes to `unavailable`.

- [ ] **Step 3: Run the new integration tests and verify they fail**

Run: `cd backend && cargo test --features e2e-test --test simulation_runs failed_run -- --nocapture`

Expected: FAIL because ordinary runs do not yet persist engine logs or invoke the analyzer.

- [ ] **Step 4: Implement shared log assembly and write-once enrichment**

Build `engineLogs` with one shared helper around `sanitize_adapter_stderr` and `sanitize_adapter_stdout`; use the error message fallback only when the adapter did not capture stderr. Replace the workflow worker's local log-array assembly with this helper. Implement the repository update with a transaction or guarded SQLite JSON update requiring `status='failed'`, a live user-owned run, and `json_extract(error_json,'$.analysisStatus')='pending'`.

- [ ] **Step 5: Change ordinary failure ordering**

In `RunService::execute`, persist the base envelope and emit terminal failed progress first. For adapter failures, read the frozen script artifact, construct an ordinary-run analysis request with `locked_contract: None` and `inputs: None`, then call the analyzer inside `tokio::time::timeout(Duration::from_secs(30), ...)`. Complete enrichment as `complete` or `unavailable`; never change the returned run error or progress outcome based on analyzer failure.

- [ ] **Step 6: Normalize pending analysis during startup recovery**

Extend `recover_interrupted_runs` so already-failed ordinary runs with `analysisStatus=pending` become `unavailable`, preserving all other JSON fields. Do not touch legacy errors without that field.

- [ ] **Step 7: Run the ordinary run suite**

Run: `cd backend && cargo test --features e2e-test --test simulation_runs && cargo test --features e2e-test --test script_repair`

Expected: PASS; the terminal progress test still observes failed progress before analysis completes.

- [ ] **Step 8: Commit the run failure pipeline**

```bash
git add backend/src/application/diagnostics.rs backend/src/application/runs.rs backend/src/application/workflow_worker.rs backend/src/application/mod.rs backend/src/db/mod.rs backend/src/db/repositories/run.rs backend/src/testing/scenario_engine.rs backend/tests/common/mod.rs backend/tests/script_repair.rs backend/tests/simulation_runs.rs
git commit -m "feat: persist ordinary run failure diagnosis"
```

### Task 3: Expose the bounded run diagnosis DTO

**Files:**
- Modify: `backend/src/api/runs.rs`
- Modify: `frontend/src/types/run.ts`
- Create: `frontend/src/types/failureDiagnosis.ts`
- Modify: `frontend/src/types/workflow.ts`
- Modify: `frontend/src/types/index.ts`
- Modify: `frontend/src/components/runs/runPresentation.ts`
- Modify: `frontend/src/components/runs/runPresentation.test.ts`
- Modify: `backend/tests/simulation_runs.rs`

**Interfaces:**
- Consumes: stored `RunFailureDiagnosis` from Task 2.
- Produces: public run error `{ code, message, stage?, engineLogs?, analysisStatus?, analysis?, engineOutput? }`, with `engineOutput` accepted only for legacy records.
- Produces: TypeScript `FailureAnalysis`, `FailureAnalysisStatus`, `EngineLog`, `FailureDiagnosis`, and `parseRunFailureDiagnosis(run: SimulationRun): FailureDiagnosis`.

- [ ] **Step 1: Write API whitelist and compatibility tests**

Extend `api::runs` unit tests and `simulation_runs.rs` to assert structured logs and valid analysis are returned, while full hashes, diagnostics, paths, secrets, control characters, oversized text, and unknown JSON fields are absent. Keep the legacy `engineOutput` and `service_restarted` expectations.

- [ ] **Step 2: Run the API tests and verify the new fields are missing**

Run: `cd backend && cargo test --features e2e-test api::runs && cargo test --features e2e-test --test simulation_runs failed_run`

Expected: FAIL on `engineLogs`, `analysisStatus`, and `analysis` assertions.

- [ ] **Step 3: Implement the public whitelist**

Parse the stored typed envelope, remap `reason` to the existing Chinese public message, and serialize only the controlled diagnosis fields. Re-sanitize every outgoing log and revalidate the analysis through the shared bounded type; malformed optional enrichment becomes `unavailable` while base logs remain visible.

- [ ] **Step 4: Add shared frontend types and parser**

Move workflow analysis/log interfaces out of `workflow.ts`. Implement `parseRunFailureDiagnosis` so modern envelopes use `engineLogs`, legacy `engineOutput` becomes one stderr log with unknown byte counts, timestamps produce duration through the existing helper, and absent analysis maps to `unavailable` rather than `pending`.

- [ ] **Step 5: Run API and presentation tests**

Run: `cd backend && cargo test --features e2e-test api::runs && cargo test --features e2e-test --test simulation_runs failed_run`

Run: `cd frontend && npm test -- --run src/components/runs/runPresentation.test.ts`

Expected: PASS.

- [ ] **Step 6: Commit the public DTO**

```bash
git add backend/src/api/runs.rs backend/tests/simulation_runs.rs frontend/src/types frontend/src/components/runs/runPresentation.ts frontend/src/components/runs/runPresentation.test.ts
git commit -m "feat: expose bounded run failure diagnosis"
```

### Task 4: Reuse one failure diagnosis panel in both progress cards

**Files:**
- Create: `frontend/src/components/messages/FailureDiagnosisPanel.tsx`
- Create: `frontend/src/components/messages/FailureDiagnosisPanel.test.tsx`
- Create: `frontend/src/hooks/useRunFailureDiagnosis.ts`
- Create: `frontend/src/hooks/useRunFailureDiagnosis.test.tsx`
- Modify: `frontend/src/components/messages/WorkflowProgressCard.tsx`
- Modify: `frontend/src/components/messages/WorkflowProgressCard.test.tsx`
- Modify: `frontend/src/components/messages/ProgressCard.tsx`
- Create: `frontend/src/components/messages/ProgressCard.test.tsx`

**Interfaces:**
- Consumes: `FailureDiagnosis` and shared analysis/log types from Task 3.
- Produces: `FailureDiagnosisPanel({ diagnosis, downloadPrefix }: { diagnosis: FailureDiagnosis; downloadPrefix: string })`.
- Produces: `useRunFailureDiagnosis(runId: string | null, enabled: boolean)` returning `{ diagnosis, loading, error, retry }`; it polls every 750 ms only while `analysisStatus === 'pending'` and ignores stale responses after run ID changes or unmount.

- [ ] **Step 1: Write the shared panel tests**

Cover formatted stdout/stderr, sanitized download names/content, truncation notice, summary/facts/possible causes/unknowns/recommendations, `pending`, `unavailable`, and no-log copy. Mock `URL.createObjectURL` and verify the downloaded Blob contains exactly the displayed sanitized text.

- [ ] **Step 2: Run the panel test and verify the component is missing**

Run: `cd frontend && npm test -- --run src/components/messages/FailureDiagnosisPanel.test.tsx`

Expected: FAIL because the component does not exist.

- [ ] **Step 3: Implement the panel and refactor the workflow card**

Move the complete failure diagnosis JSX and download logic out of `WorkflowProgressCard`. Map the selected workflow attempt to `FailureDiagnosis`; keep workflow error text, attempt number, retry, cancellation, and collapsed automatic repair records in the workflow card.

- [ ] **Step 4: Run workflow and panel regressions**

Run: `cd frontend && npm test -- --run src/components/messages/FailureDiagnosisPanel.test.tsx src/components/messages/WorkflowProgressCard.test.tsx`

Expected: PASS with unchanged workflow-visible copy and repair-history behavior.

- [ ] **Step 5: Write failed run progress hook/card tests**

Add tests named:

- `loads_and_shows_failed_run_logs_and_analysis_inline`
- `polls_only_while_analysis_is_pending`
- `renders_legacy_engine_output_without_polling`
- `keeps_stage_list_when_run_detail_loading_fails_and_retries`
- `never_loads_run_detail_for_non_run_or_nonfailed_progress`
- `ignores_stale_detail_responses_and_clears_polling_on_unmount`

- [ ] **Step 6: Run the progress tests and verify they fail**

Run: `cd frontend && npm test -- --run src/hooks/useRunFailureDiagnosis.test.tsx src/components/messages/ProgressCard.test.tsx`

Expected: FAIL because failed progress does not call `getRun` or render the shared panel.

- [ ] **Step 7: Implement pending-only loading and inline diagnosis**

Use `getRun(operation.id)` only for a failed run operation. Keep the existing stage list visible through loading/error states, show an explicit retry action for detail load errors, and mount `FailureDiagnosisPanel` below the stages once data arrives. Stop polling on complete, unavailable, error, run ID change, or unmount.

- [ ] **Step 8: Run all affected frontend tests**

Run: `cd frontend && npm test -- --run src/components/messages/FailureDiagnosisPanel.test.tsx src/components/messages/WorkflowProgressCard.test.tsx src/components/messages/ProgressCard.test.tsx src/hooks/useRunFailureDiagnosis.test.tsx src/components/runs/runPresentation.test.ts`

Expected: PASS.

- [ ] **Step 9: Commit shared frontend diagnosis UI**

```bash
git add frontend/src/types frontend/src/components/messages frontend/src/components/runs/runPresentation.ts frontend/src/components/runs/runPresentation.test.ts frontend/src/hooks/useRunFailureDiagnosis.ts frontend/src/hooks/useRunFailureDiagnosis.test.tsx
git commit -m "feat: show diagnosis in run progress cards"
```

### Task 5: Add browser coverage, synchronize contracts, and verify release gates

**Files:**
- Modify: `integration-tests/e2e/v3-workflows.spec.ts`
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `docs/api-design.md`
- Modify: `docs/db-design.md`
- Modify: `docs/frontend-design.md`
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/development-status.md`
- Include: `docs/superpowers/specs/2026-11-01-run-progress-failure-diagnosis-design.md`
- Include: `docs/superpowers/plans/2026-11-01-run-progress-failure-diagnosis.md`

**Interfaces:**
- Consumes: completed backend and frontend behavior from Tasks 1–4.
- Produces: RUN-001 traceability for inline failed progress diagnosis, persistent refresh behavior, analyzer degradation, and shared component coverage.

- [ ] **Step 1: Extend the deterministic browser failure journey**

Update the existing failed-run journey to assert the failed run progress card itself shows the sanitized multiline tensoreval output without opening run history, survives a page refresh, and never exposes the fixture absolute path. Do not require a live LLM response in deterministic Playwright.

- [ ] **Step 2: Run the focused browser journey**

Run: `cd frontend && npm run test:e2e -- --grep "failed run"`

Expected: PASS.

- [ ] **Step 3: Synchronize design and feature contracts**

Document the existing route's optional diagnosis fields, the guarded terminal enrichment, shared analyzer prompt boundary, pending-only frontend polling, and the shared panel. Add the new Rust/Vitest/Playwright evidence names to RUN-001 and keep the public route count unchanged.

- [ ] **Step 4: Run backend formatting, lint, and full tests**

Run: `cd backend && cargo fmt --all -- --check`

Run: `cd backend && cargo clippy --all-targets --all-features -- -D warnings`

Run: `cd backend && cargo test --features e2e-test`

Expected: all commands exit 0.

- [ ] **Step 5: Run frontend full tests and production build**

Run: `cd frontend && npm test -- --run`

Run: `cd frontend && npm run build`

Expected: all tests pass and Vite production build succeeds.

- [ ] **Step 6: Run full deterministic browser and coverage gates**

Run: `cd frontend && npm run test:e2e`

Run: `./scripts/check-functional-coverage --require-pr`

Expected: all Playwright journeys pass and functional coverage remains 100%.

- [ ] **Step 7: Review the final diff and commit documentation/coverage**

Run: `git status --short && git diff --check`

Expected: only intended files remain, no whitespace errors, and no user-owned changes are staged.

```bash
git add integration-tests/e2e/v3-workflows.spec.ts integration-tests/catalog/features.yaml docs/api-design.md docs/db-design.md docs/frontend-design.md docs/agent-prompt-skills-design.md docs/integration-testing-design.md docs/development-status.md docs/superpowers/specs/2026-11-01-run-progress-failure-diagnosis-design.md docs/superpowers/plans/2026-11-01-run-progress-failure-diagnosis.md
git commit -m "docs: trace shared run failure diagnosis"
```

- [ ] **Step 8: Run final commit-state verification**

Run: `git status --short && git log -5 --oneline`

Expected: clean worktree and the five planned commits present.
