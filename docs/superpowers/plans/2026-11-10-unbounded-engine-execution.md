# Unbounded Engine Execution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove every production `tensoreval` wall-clock deadline while adding safe, user-scoped cancellation for ordinary simulation runs and preserving the existing managed-workflow cancellation path.

**Architecture:** The process adapter will wait for engine exit or result-safety failure without a timer; dropping an in-flight adapter future remains the single process-group termination mechanism. Ordinary runs gain a durable compare-and-set cancellation transition plus an in-memory signal registry, while managed runs continue through their workflow cancellation service. The frontend routes each live run to the correct cancellation endpoint and renders user cancellation as a neutral terminal state over the existing `failed` storage status.

**Tech Stack:** Rust, Tokio, Axum, Diesel/SQLite, React, TypeScript, Vitest, Playwright, YAML feature catalog

**Spec:** `docs/superpowers/specs/2026-11-10-unbounded-engine-execution-design.md`

## Global Constraints

- Every production `tensoreval` invocation—simulation, managed workflow, case inspection/import/projection, and derived analysis—has no wall-clock deadline.
- Result file count, per-file size, total size, regular-file, symlink, and path-boundary checks remain active during and after execution.
- Test, HTTP-client, LLM, and failure-analysis watchdogs remain; they must not be passed into production engine specs.
- Ordinary and managed simulations are user-cancellable; case inspection/import/projection and derived analysis get no new cancellation API.
- All `tensoreval` process creation and process-group termination remain under `backend/src/adapters/`.
- Cross-user and missing resources return 404; cancellation exposes no PID, command, path, SQL, or raw stderr.
- Historical `engine_timeout` rows remain readable without migration, but no new production path emits that code.
- Existing user edits to `backend/resources/agent/v4/manifest.json` and `backend/resources/agent/v4/skill-index.json` must be preserved and excluded unless the task requires a conflict-resolved manifest update.

## Review Focus

- A long-running engine process that emits no output must remain alive until completion or explicit future drop; Task 1 adds the process-helper regression.
- Dropping an active run must kill descendants as well as the direct child and remove partial results; Task 1 retains the heartbeat/process-group assertions.
- Cancellation racing with atomic result publication must produce exactly one terminal state with balanced artifact references; Task 2 adds repository race coverage.
- Cancelling a queued run must release queue accounting without ever starting the engine; Task 2 adds a scheduler-backed application test.
- A managed run presented in run history must call the workflow endpoint, while direct calls to the ordinary-run endpoint return stable 409; Tasks 2 and 3 cover both sides.

---

### Task 1: Remove production engine deadlines end to end

**Files:**
- Modify: `backend/src/adapters/mod.rs`
- Modify: `backend/src/adapters/import.rs`
- Modify: `backend/src/adapters/process/mod.rs`
- Modify: `backend/src/adapters/process/control.rs`
- Modify: `backend/src/adapters/process/execution.rs`
- Modify: `backend/src/adapters/process/import.rs`
- Modify: `backend/src/application/analysis.rs`
- Modify: `backend/src/application/upload_processing.rs`
- Modify: `backend/src/application/runs.rs`
- Modify: `backend/src/application/run_execution.rs`
- Modify: `backend/src/application/workflow_worker.rs`
- Modify: `backend/src/application/managed_workflows.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/config.rs`
- Modify: `backend/src/main.rs`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/src/testing/app.rs`
- Modify: `backend/src/testing/scenario_engine.rs`
- Test: `backend/tests/process_engine_v4.rs`
- Test: `backend/tests/app_bootstrap.rs`
- Test: `backend/tests/model_import_adapter.rs`
- Test: `backend/tests/release_engine_cases.rs`
- Test: `backend/tests/script_repair.rs`

**Interfaces:**
- Consumes: Existing `SimEngine` methods, `ProcessGroupGuard`, result monitor, and application composition root.
- Produces: `InspectSpec`, `RunSpec`, `ImportInspectSpec`, `ImportSpec`, and `CaseProjectionSpec` without `timeout`; `EngineConfig::probe(bin: PathBuf)`; `control::run_command(command, retained_limit, monitored_result_dir)` with no deadline parameter.

- [ ] **Step 1: Rewrite the process lifecycle regression to specify unbounded execution**

In `backend/tests/process_engine_v4.rs`, replace `timeout_and_future_cancellation_terminate_the_whole_process_group` with `unbounded_execution_waits_and_future_cancellation_terminates_the_whole_process_group`. Start the existing parent/child heartbeat helper, assert through an outer `tokio::time::timeout` probe that the engine future is still pending, then drop/abort that future and assert the heartbeat stops and the result directory is removed. Keep the monitor-failure process-group test unchanged.

- [ ] **Step 2: Update configuration and workflow tests to express the removed contract**

Remove assertions and fixtures for `RuntimeConfig::tensoreval_timeout_ms`, `TENSOREVAL_TIMEOUT_MS`, `EngineScenario::Timeout`, and workflow timeout repair behavior from `backend/tests/app_bootstrap.rs` and `backend/tests/script_repair.rs`. Update adapter/import/release spec constructors to compile without timeout fields; keep outer test watchdogs such as `tokio::time::timeout`.

- [ ] **Step 3: Run the focused tests to verify they fail against the old interfaces**

Run:

```bash
cd backend
cargo test --features e2e-test --test process_engine_v4 unbounded_execution_waits_and_future_cancellation_terminates_the_whole_process_group
cargo test --features e2e-test --test app_bootstrap
```

Expected: FAIL because production specs/config still require or expose timeout and `run_command` still terminates by elapsed time.

- [ ] **Step 4: Remove timeout from the adapter contracts and process controller**

Delete `timeout: Duration` from `InspectSpec`, `RunSpec`, `ImportInspectSpec`, `ImportSpec`, and `CaseProjectionSpec`; delete `EngineConfig::default_timeout`; change `EngineConfig::probe(bin: PathBuf)` and all constructors accordingly. Change `control::run_command` to select only between child exit and result monitoring, delete the elapsed branch that creates `engine_timeout`, and retain process-group cleanup for monitor errors and future drop.

- [ ] **Step 5: Remove timeout from every production call site and composition option**

Delete `RuntimeConfig::tensoreval_timeout_ms`, `AppStateOptions::managed_engine_timeout`, `RunServiceDependencies::engine_timeout`, `ManagedWorkflowDependencies::engine_timeout`, and matching service fields. Remove timeout arguments from analysis, upload inspection/import/projection, ordinary runs, managed workflows, production bootstrap, test app assembly, and release-case spec construction. `ProcessEngine::from_env` must ignore an obsolete `TENSOREVAL_TIMEOUT_MS` instead of rejecting startup.

- [ ] **Step 6: Remove the unreachable production timeout error contract**

Delete `ErrorCategory::Timeout` if no non-engine caller remains, remove `engine_timeout` from `backend/src/contract_manifest.rs`, delete the workflow repairability special case and `EngineScenario::Timeout`, and retain generic reading/sanitization of historical error JSON in `api/runs.rs`.

- [ ] **Step 7: Run adapter, import, workflow, configuration, and release compile coverage**

Run:

```bash
cd backend
cargo test --features e2e-test --test process_engine_v4
cargo test --features e2e-test --test model_import_adapter
cargo test --features e2e-test --test script_repair
cargo test --features e2e-test --test app_bootstrap
cargo test --features e2e-test --test contract_manifest
cargo test --features e2e-test --test release_engine_cases --no-run
```

Expected: PASS; no production spec contains a timeout and cancellation/monitor failures still kill the process group.

- [ ] **Step 8: Commit the unbounded engine boundary**

```bash
git add backend/src backend/tests
git commit -m "refactor: remove production engine timeouts"
```

### Task 2: Add durable ordinary-run cancellation

**Files:**
- Modify: `backend/src/db/repositories/run.rs`
- Modify: `backend/src/db/repositories/mod.rs`
- Modify: `backend/src/application/runs.rs`
- Modify: `backend/src/application/run_execution.rs`
- Modify: `backend/src/api/runs.rs`
- Modify: `backend/src/api/error.rs`
- Modify: `backend/src/api/mod.rs`
- Test: `backend/tests/simulation_runs.rs`
- Test: `backend/tests/auth_scope.rs`
- Test: `backend/tests/failure_atomicity.rs`
- Test: `backend/tests/contract_manifest.rs`

**Interfaces:**
- Consumes: Task 1's unbounded `RunSpec`, existing `RunScheduler::run_cancellable`, `ProcessGroupGuard`, `RunFailureDiagnosis`, and workflow cancellation route.
- Produces: `RunRepo::cancel(&UserId, i32, i64) -> Result<SimulationRun, RunRepoError>`; `RunService::cancel(&CurrentUser, i32) -> Result<RunSummary, RunError>`; `POST /api/runs/{id}/cancel`; stable API error `managed_run_cancel_via_workflow`; public terminal error `cancelled_by_user`.

- [ ] **Step 1: Write repository tests for cancellation state and publication races**

Add tests that assert: queued and running ordinary runs transition to `status=failed`, `stage=cancelled_by_user`, `error.code=cancelled_by_user`, and a non-null finish time; repeated cancellation and cancellation of an already terminal ordinary run return that terminal row unchanged; managed runs reject direct cancellation; a cancellation that wins before `complete` prevents dataset insertion/success overwrite; a completion that wins first remains succeeded.

- [ ] **Step 2: Write API/application tests for active, queued, scoped, and analysis-free cancellation**

In `simulation_runs.rs`, use a controllable engine barrier to prove queued cancellation never calls the engine, active cancellation returns promptly and drops the engine future, scheduler counts are released, no result dataset is published, no failure analyzer runs, and repeat cancellation is idempotent. In `auth_scope.rs`, assert another user receives 404. Assert direct cancellation of a run with `managedWorkflowId` returns 409 `managed_run_cancel_via_workflow`.

- [ ] **Step 3: Run the new cancellation tests to verify they fail**

Run:

```bash
cd backend
cargo test --features e2e-test --test simulation_runs cancel
cargo test --features e2e-test --test auth_scope cancel_run
cargo test --features e2e-test --test failure_atomicity cancel
```

Expected: FAIL because the route, service cancellation registry, and compare-and-set repository transition do not exist.

- [ ] **Step 4: Implement atomic repository cancellation and terminal compare-and-set**

Add `RunRepoError::ManagedRunCancelViaWorkflow` and `RunRepo::cancel`. Inside one immediate transaction, verify user ownership/live session, reject `managed_workflow_id IS NOT NULL`, conditionally update only `queued`/`running` ordinary rows to the exact cancellation diagnosis, and otherwise return the existing terminal row. Tighten `set_running`, `fail`, `complete`, and `complete_with_artifact_outputs` so terminal cancellation cannot be overwritten and drafts/artifact refs are released when publication loses the race; execution treats the resulting terminal-state compare-and-set miss as cancellation rather than a second failure.

- [ ] **Step 5: Add the ordinary-run cancellation registry and active select**

Add an `Arc<Mutex<BTreeMap<i32, watch::Sender<bool>>>>` owned by `RunService`, with private register/signal/remove helpers. Register before spawning a new non-replayed run; forward session cancellation into the same sender; change execution to select the active `execute_inner` future against `cancel.changed()` as well as using `RunScheduler::run_cancellable` for queued cancellation; always remove the sender on exit. `RunService::cancel` must commit the repository transition before signaling the worker.

- [ ] **Step 6: Add the documented API route and public error mapping**

Add `runs::cancel` with a `///` contract comment and route `POST /api/runs/{id}/cancel`. Return the authoritative `run_json(RunSummary)` on 200. Map `RunError::ManagedRunCancelViaWorkflow` to 409 `managed_run_cancel_via_workflow`; expose stored `cancelled_by_user` as a stable public code/message and skip failure analysis for it.

- [ ] **Step 7: Run backend cancellation and regression tests**

Run:

```bash
cd backend
cargo test --features e2e-test --test simulation_runs
cargo test --features e2e-test --test auth_scope
cargo test --features e2e-test --test failure_atomicity
cargo test --features e2e-test --test contract_manifest
```

Expected: PASS; cancellation is scoped/idempotent, races are atomic, and managed runs direct clients to workflow cancellation.

- [ ] **Step 8: Commit the backend cancellation API**

```bash
git add backend/src backend/tests
git commit -m "feat: allow users to cancel ordinary runs"
```

### Task 3: Add run-history cancellation controls

**Files:**
- Modify: `frontend/src/api/client.ts`
- Modify: `frontend/src/hooks/useRunHistory.ts`
- Modify: `frontend/src/hooks/useRunHistory.test.ts`
- Modify: `frontend/src/components/runs/RunHistory.tsx`
- Modify: `frontend/src/components/runs/RunHistory.test.tsx`
- Modify: `frontend/src/components/runs/runPresentation.ts`
- Modify: `frontend/src/components/runs/runPresentation.test.ts`

**Interfaces:**
- Consumes: Task 2's `POST /api/runs/{id}/cancel`, existing `cancelManagedWorkflow`, `SimulationRun.managedWorkflowId`, and public `cancelled_by_user` error.
- Produces: `cancelRun(id: string) -> Promise<SimulationRun>` in the API client; `RunHistoryState.cancel(run: SimulationRun) -> Promise<void>` in the history hook; `isRunCancelled(run: SimulationRun) -> boolean` and `runStatusLabel(run: SimulationRun) -> string` in presentation helpers.

- [ ] **Step 1: Write hook tests for ordinary/managed endpoint routing**

Mock both cancellation clients. Assert `RunHistoryState.cancel` calls `cancelRun(run.id)` for `managedWorkflowId === null`, calls `cancelManagedWorkflow(run.managedWorkflowId)` otherwise, never calls both, and refreshes/replaces history only after success. Assert a rejected request leaves the prior row intact and propagates the error.

- [ ] **Step 2: Write component and presentation tests for cancel UX**

Assert queued/running rows show a “取消运行 {id}” button; click changes it to “取消中…” and disables it; success removes the action after the supplied run becomes terminal; failure restores the button and shows a bounded error. Assert terminal rows have no action. Add a `cancelled_by_user` failed fixture and assert its badge/detail say “已取消” with neutral styling and duration/timestamps, not generic failure copy.

- [ ] **Step 3: Run focused frontend tests to verify they fail**

Run:

```bash
cd frontend
npm test -- --run src/hooks/useRunHistory.test.ts src/components/runs/RunHistory.test.tsx src/components/runs/runPresentation.test.ts
```

Expected: FAIL because the API client/hook action and cancellation presentation do not exist.

- [ ] **Step 4: Implement API and hook routing**

Add `cancelRun` to `client.ts`. Add `cancel(run)` to `RunHistoryState`; use the ordinary endpoint for null `managedWorkflowId`, the workflow endpoint otherwise, and trigger a generation-safe reload after success without optimistic terminal mutation.

- [ ] **Step 5: Implement the run-history action and cancellation presentation**

Track cancelling run IDs and per-run cancellation errors in `RunHistory`. Render the action only for queued/running rows, disable it while pending, call `history.cancel(run)`, and restore on failure. Add `isRunCancelled(run)` and change `runStatusLabel` to accept the full run so `cancelled_by_user` yields “已取消”; use the same predicate for neutral detail styling without changing the `SimulationRun['status']` union.

- [ ] **Step 6: Run frontend unit tests and build**

Run:

```bash
cd frontend
npm test -- --run
npm run build
```

Expected: PASS; TypeScript compiles and all existing run-history behaviors remain intact.

- [ ] **Step 7: Commit frontend cancellation**

```bash
git add frontend/src
git commit -m "feat: add run history cancellation controls"
```

### Task 4: Synchronize contracts, documentation, and release gates

**Files:**
- Modify: `docs/development-status.md`
- Modify: `docs/api-design.md`
- Modify: `docs/v4-deployment.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `integration-tests/catalog/features.yaml`
- Modify if required by generated contract validation: `backend/resources/agent/v4/manifest.json`
- Preserve without unrelated edits: `backend/resources/agent/v4/skill-index.json`
- Test: backend/frontend suites and `scripts/check-functional-coverage`

**Interfaces:**
- Consumes: Tasks 1–3's final API route, stable codes, unbounded engine behavior, and UI action.
- Produces: Current authoritative docs and feature traceability for unbounded production execution and explicit cancellation.

- [ ] **Step 1: Update API and deployment contracts**

Document `POST /api/runs/{id}/cancel`, its 200/404/409/idempotency behavior, `cancelled_by_user`, and `managed_run_cancel_via_workflow` in `docs/api-design.md`. Remove `TENSOREVAL_TIMEOUT_MS` from deployment configuration and state that every production engine call is unbounded while result safety and scheduler limits remain.

- [ ] **Step 2: Update status and testing design**

Replace claims that production timeout is a process-boundary feature with future-drop/user cancellation semantics. Keep test watchdog language explicit. Record that imports/projections/analysis have no user cancellation and end only through completion, active failure, result-safety rejection, or service stop.

- [ ] **Step 3: Update machine-readable behavior coverage**

Remove `timeout_is_not_repaired`, `error:engine_timeout`, and production process-timeout assertions from `integration-tests/catalog/features.yaml`. Add behavior cases for unbounded engine execution, queued ordinary cancellation, active process-group cancellation, cross-user 404, managed-run 409 routing, idempotency, and neutral frontend display. Update manifest error enumeration only if the contract test requires the already user-modified manifest; preserve unrelated manifest and skill-index changes.

- [ ] **Step 4: Run formatting, lint, tests, builds, and traceability gates**

Run:

```bash
cd backend
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --features e2e-test
cd ../frontend
npm test -- --run
npm run build
cd ..
./scripts/check-functional-coverage --require-pr
```

Expected: every command exits 0. Tests may use bounded watchdogs, but no production engine spec/config exposes a wall-clock timeout.

- [ ] **Step 5: Review the final diff for user-change preservation and contract completeness**

Run:

```bash
git status --short
git diff --check
git diff --stat
rg -n "TENSOREVAL_TIMEOUT_MS|engine_timeout|timeout_is_not_repaired" backend/src docs integration-tests/catalog/features.yaml
```

Expected: only intentional historical-compatibility/test-watchdog references remain; `skill-index.json` is untouched; any manifest hunk is limited to the required error enumeration.

- [ ] **Step 6: Commit docs and release traceability**

```bash
git add docs integration-tests/catalog/features.yaml backend/resources/agent/v4/manifest.json
git commit -m "docs: define unbounded engine execution"
```
