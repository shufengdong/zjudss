# Run-Private Struct Parameter Overrides Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every user-editable study Struct parameter a run-private override while keeping long-lived Struct resources as explicit templates.

**Architecture:** Study configurations retain a nullable template binding and compute user overrides independently from server-owned result-selector overrides. Confirmation materializes one immutable private Struct artifact from source/defaults plus both override classes, freezes categorized and merged override metadata, and never publishes a data resource. Retry and rerun resolve only explicit replacement descendants, while source-less and historical snapshots remain replayable.

**Tech Stack:** Rust, Axum, Diesel/SQLite, Arrow/Parquet, React, TypeScript, Vitest

**Spec:** `docs/superpowers/specs/2026-11-03-run-private-struct-parameter-overrides-design.md`

## Global Constraints

- Work directly on `master` as required by `AGENTS.md`; do not create a feature worktree.
- Preserve and never stage the user's existing charting changes in `backend/src/application/charting/` or `backend/src/application/dataframe_reader.rs`.
- All Struct payloads remain schema-validated Snappy Parquet artifacts; no CSV path and no direct tensoreval invocation.
- Study confirmation must not create, replace, rename, retire, or publish any `data_resources` row for Struct parameter edits.
- Selector fields remain server-owned and cannot enter user overrides.
- Existing resource editor and upload Replace/SaveAsNew behavior remains unchanged.
- Repository access remains user-scoped; no physical paths or arbitrary payload contracts reach the browser or LLM.
- Public DTO, frontend behavior, current design docs, functional catalog, and automated behavior evidence change together.

## Review Focus

- A request that sends a forged selector value together with `max_iter` changes must preserve the server selector and record only `max_iter` as a user override; Task 1 adds this behavior test.
- JSON-equivalent numbers plus missing/nullable keys must not create spurious user overrides or lose required values; Task 1 extends semantic-diff tests.
- A same-alias resource from case switching or re-upload must not inherit frozen overrides, while an explicit multi-hop replacement descendant may; Task 3 tests both lineage classes and cycle/depth rejection.
- Repeated confirm/retry/rerun IDs under exhausted storage quota must return the existing operation before private artifact materialization; Tasks 2 and 3 pin all three paths.
- Historical snapshots containing only `effective.overrides` must remain retryable/rerunnable without category fields; Task 3 adds legacy snapshot coverage.

---

### Task 1: Make study Struct edits produce private override contracts

**Files:**
- Modify: `backend/src/application/study_configurations.rs`
- Modify: `backend/tests/study_configurations.rs`

**Interfaces:**
- Produces: `UpdateStudyStructRequest { expected_revision: i64, values: Value }` without a study replacement action.
- Produces: `PreparedStructOverlay { input_id, input_alias, source_resource_id: Option<String>, schema_ref, effective_values, user_overrides, selector_overrides, overrides }`.
- Produces: `PreparedStudyConfirmation.replacement_actions` as an empty transitional internal vector; Task 2 removes the obsolete plumbing after all consumers move to overlays.
- Produces: `struct_user_overrides(baseline: &Value, current: &Value, ignored_fields: &[String]) -> Result<Value, StudyConfigurationError>` using JSON numeric equivalence.
- Consumes: existing facade schema/defaults, resource baselines, `result_selector_fields`, and result selections.

- [x] **Step 1: Write failing configuration behavior tests**

Add or replace tests with these exact behaviors:

- `struct_edits_are_run_private_without_replacement_choice`: update `max_iter` and `tolerance` using no replacement action; configuration becomes `ready_to_confirm`, the bound resource remains live and unchanged, `replacement_actions` is empty, and the prepared overlay contains both fields in `user_overrides`.
- `selector_values_are_server_owned_while_user_overrides_remain_private`: submit a forged `result` with a normal field change; the stored/effective selector follows selected results and only the normal field enters `user_overrides`.
- `struct_without_resource_prepares_private_values_from_defaults`: with no compatible resource, required/default values can complete the card and prepare an overlay whose source ID is `None`.
- Extend the numeric-equivalence unit test so `50` versus `50.0` produces no user override and omitted nullable values are represented consistently.

- [x] **Step 2: Run the focused tests and verify RED**

Run: `cargo test --manifest-path backend/Cargo.toml --test study_configurations --features e2e-test`

Expected: FAIL because ordinary Struct edits still require replacement actions, prepared confirmations still emit replacement commands, and source-less overlays are unsupported.

- [x] **Step 3: Implement private override preparation**

In `study_configurations.rs`:

- Remove `replacement_action` from `UpdateStudyStructRequest` and eliminate study-card validation/storage of that action.
- Keep `baselineValues` metadata only; clear legacy replacement metadata whenever the Struct card is read, rebound, or updated.
- Replace replacement-gated readiness with valid values plus resolved resource selection.
- Compute `user_overrides`, `selector_overrides`, merged `overrides`, and complete `effective_values` for every Struct input.
- Prepare an overlay whenever there is no source or merged overrides are non-empty; reuse is decided during materialization.
- Stop producing confirmation-time Struct resource commands; leave the now-always-empty internal vector/type in place until Task 2 removes its consumers.

- [x] **Step 4: Run Task 1 regression tests**

Run: `cargo test --manifest-path backend/Cargo.toml --test study_configurations --features e2e-test`

Expected: PASS with all configuration tests green.

- [x] **Step 5: Commit Task 1 only**

Stage only the two Task 1 files and commit:

`feat: make study struct edits run private`

---

### Task 2: Materialize and freeze initial private Struct inputs

**Files:**
- Modify: `backend/src/application/data_resources.rs`
- Modify: `backend/src/application/managed_workflows.rs`
- Modify: `backend/src/db/repositories/managed_workflow.rs`
- Modify: `backend/tests/managed_workflows.rs`

**Interfaces:**
- Consumes: Task 1 `PreparedStructOverlay` and nullable frozen source identity.
- Produces: `WorkflowStructOverlayMaterial { input_id, source_resource_id: Option<String>, source_artifact_id: Option<i32>, source_content_hash: Option<String>, source_schema_hash: Option<String>, artifact_id, content_hash, schema_ref, effective_values_json, user_overrides_json, selector_overrides_json, overrides_json, owns_artifact_reference }`.
- Produces: `materialize_study_struct_overlays(user, prepared) -> Result<Vec<WorkflowStructOverlayMaterial>, DataResourceError>` with no replacement-material argument.
- Produces: `ManagedWorkflowRepo::confirm_and_start(user, prepared, overlays, ...)` with no `WorkflowReplacementMaterial` argument.

- [x] **Step 1: Write failing managed-confirmation tests**

Add or revise tests with these behaviors:

- `ordinary_struct_overrides_freeze_without_publishing_a_resource`: confirm modified `max_iter`, `tolerance`, and selected results; resource count/identity/hash remain unchanged, effective Parquet contains all changes, and snapshot contains `userOverrides`, `selectorOverrides`, merged `overrides`, and `effective.values`.
- `source_less_struct_confirmation_owns_a_private_artifact`: confirm defaults/user values without a resource, verify `workflow_input_snapshots.resource_id IS NULL`, execution uses the private artifact, and session/run deletion releases it exactly once.
- `byte_identical_private_values_reuse_the_source_artifact`: unchanged values and no selector difference reuse the source artifact rather than creating storage.
- Extend confirmation idempotency under a quota that permits no second materialization; the same `clientOperationId` must replay successfully.

- [x] **Step 2: Run managed workflow tests and verify RED**

Run: `cargo test --manifest-path backend/Cargo.toml --test managed_workflows --features e2e-test`

Expected: FAIL because materialization requires a source/replacement and the repository publishes replacement resources.

- [x] **Step 3: Generalize Struct overlay materialization**

Update `WorkflowStructOverlayMaterial` and `materialize_study_struct_overlays` to:

- validate optional source ID/hash/schema when present;
- build source-less Parquet from effective values when absent;
- reuse a byte-identical source artifact when possible;
- serialize categorized and merged overrides;
- release every newly owned artifact on partial failure.

Delete the managed-study-only `materialize_study_replacements` path and its internal replacement material type; retain standalone structured resource preview/confirmation code.

- [x] **Step 4: Freeze nullable sources transactionally**

Refactor `confirm_and_start` and input insertion so every prepared Struct overlay is matched exactly once. Persist nullable `resource_id`, a nullable `source` object, complete `effective.values`, categorized overrides, and merged `effective.overrides`. Validate source/resource identity only when present, keep the transaction race check, and maintain artifact reference ownership on success/failure/replay.

- [x] **Step 5: Run Task 2 regression tests**

Run: `cargo test --manifest-path backend/Cargo.toml --test managed_workflows --features e2e-test`

Expected: PASS with no Struct resource publication and correct source-less cleanup.

- [x] **Step 6: Commit Task 2 only**

Stage only the four Task 2 files and commit:

`feat: freeze private study parameter artifacts`

---

### Task 3: Preserve private parameters across retry and rerun safely

**Files:**
- Create: `backend/src/db/repositories/resource_lineage.rs`
- Modify: `backend/src/db/repositories/mod.rs`
- Modify: `backend/src/db/repositories/managed_workflow.rs`
- Modify: `backend/src/db/repositories/run.rs`
- Modify: `backend/src/application/data_resources.rs`
- Modify: `backend/src/application/managed_workflows.rs`
- Modify: `backend/src/application/runs.rs`
- Modify: `backend/tests/workflow_retries.rs`
- Modify: `backend/tests/managed_workflows.rs`

**Interfaces:**
- Consumes: Task 2 nullable-source snapshot and categorized/merged overrides.
- Produces: `ResolvedStructResource { id: String, artifact_id: i32, alias: String, schema_ref: String, schema_hash: String, content_hash: String }`.
- Produces: `resolve_live_struct_successor(conn, session_id, frozen_resource_id, resource_alias, schema_ref) -> Result<Option<ResolvedStructResource>, RepoError>` accepting only the original live resource or an explicit `lineage.replacesResourceId` descendant, with cycle detection and a maximum ancestry depth of 64.
- Produces: retry/rerun overlay preparation that uses categorized overrides when present and falls back to legacy `effective.overrides`.
- Produces: source-less retry/rerun inputs that reuse frozen effective artifact without searching by alias.

- [x] **Step 1: Write failing retry and lineage tests**

In `workflow_retries.rs`, add:

- `retry_replays_user_and_selector_overrides_on_explicit_replacement_descendant` including a two-hop replacement chain.
- `retry_rejects_same_alias_resource_without_replacement_lineage` covering delete/re-upload or case-style publication.
- `source_less_overlay_retry_reuses_frozen_effective_input`.
- `legacy_overlay_snapshot_retries_with_merged_overrides`.
- A malformed/cyclic or over-depth replacement lineage returns `configuration_stale` rather than looping or selecting a resource.

- [x] **Step 2: Run retry tests and verify RED**

Run: `cargo test --manifest-path backend/Cargo.toml --test workflow_retries --features e2e-test`

Expected: FAIL because current retry lookup accepts arbitrary same-alias live resources and requires a source resource.

- [x] **Step 3: Implement provenance-safe retry resolution**

Add the shared repository helper and route managed retry preparation through it. For a valid changed descendant, read the latest template and apply merged overrides in deterministic user-then-selector order; for no source, reuse the frozen artifact. Keep early `clientRetryId` replay before lineage lookup or materialization and retain transaction-level race checks.

- [x] **Step 4: Write failing generated-script rerun tests**

In `managed_workflows.rs`, extend the generated-script rerun case to cover:

- explicit replacement descendant receives frozen user and selector overrides;
- same alias without replacement lineage is rejected;
- source-less and legacy snapshots rerun using their frozen effective input;
- repeated `clientRunId` succeeds under zero additional artifact quota.

- [x] **Step 5: Run rerun tests and verify RED**

Run: `cargo test --manifest-path backend/Cargo.toml --test managed_workflows --features e2e-test`

Expected: FAIL because current run queries inner-join by alias and cannot represent source-less inputs or lineage restrictions.

- [x] **Step 6: Implement provenance-safe rerun resolution**

Use the shared lineage helper from `run.rs`; keep `RetryStructOverlayRequest` source-bound for changed replacement descendants, while source-less inputs bypass overlay rematerialization and retain their frozen artifact. Update input validation for both cases. Keep `clientRunId` replay ahead of scheduler checks, lineage lookup, and artifact creation.

- [x] **Step 7: Run retry, rerun, and run lifecycle regressions**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml --test workflow_retries --features e2e-test
cargo test --manifest-path backend/Cargo.toml --test managed_workflows --features e2e-test
cargo test --manifest-path backend/Cargo.toml --test simulation_runs --features e2e-test
```

Expected: all three targets PASS.

- [x] **Step 8: Commit Task 3 only**

Stage only Task 3 files and commit:

`fix: retain private parameters through safe reruns`

---

### Task 4: Present run-private parameter semantics in the study card

**Files:**
- Modify: `frontend/src/types/studyConfiguration.ts`
- Modify: `frontend/src/components/studies/StructParameterCard.tsx`
- Modify: `frontend/src/components/studies/ConfigurationSummaryCard.tsx`
- Modify: `frontend/src/components/messages/StudyConfigurationCard.tsx`
- Modify: `frontend/src/components/messages/StudyConfigurationCard.test.tsx`

**Interfaces:**
- Consumes: Task 1 backend request contract without `replacementAction`, plus existing `baselineValues` and `resultSelectorFields`.
- Produces: `StructParameterCard.onSave(values)` and study update requests containing only revision plus values.
- Leaves: upload/resource replacement UI and general `ReplacementAction` types unchanged.

- [x] **Step 1: Write failing component tests**

Add or revise tests asserting:

- a bound template shows no save-mode selector, reports `本次运行覆盖：max_iter、tolerance；不会修改参数模板`, and submits no `replacementAction`;
- unchanged values say `当前值来自参数模板；本次研究不会修改模板`;
- no resource says `使用目录默认值；保存后仅用于本次研究`;
- the action button is `应用参数`;
- selector fields remain hidden and never enter changed-field copy or the request;
- the ready-to-confirm summary labels Struct differences as `运行覆盖` and never says `覆盖` or `另存`.

- [x] **Step 2: Run focused frontend tests and verify RED**

Run: `cd frontend && npm test -- --run src/components/messages/StudyConfigurationCard.test.tsx`

Expected: FAIL because the save-mode selector and replacement action still exist.

- [x] **Step 3: Implement the study-card copy and request changes**

Change `StudyStructUpdate` to `{ expectedRevision: number; values: Record<string, unknown> }`; remove the study-card `ReplacementAction` imports/state/props, compute changed fields against baseline while excluding selector fields, render the three approved messages, use `应用参数`, and update the summary to describe template identity plus run overrides. Do not change resource-workbench controls or the general `ReplacementAction` type.

- [ ] **Step 4: Run frontend regression checks**

Run:

```bash
cd frontend
npm test -- --run src/components/messages/StudyConfigurationCard.test.tsx
npm test -- --run
npm run build
```

Expected: focused and full Vitest suites PASS; production build PASS.

Progress: focused `StudyConfigurationCard.test.tsx` passes 20/20. Full Vitest and production build are intentionally deferred until the user requests full verification because other processes are modifying the workspace.

- [x] **Step 5: Commit Task 4 only**

Stage only Task 4 files and commit:

`feat: explain run-private study parameters`

---

### Task 5: Update contracts, traceability, and run full verification

**Files:**
- Modify: `docs/api-design.md`
- Modify: `docs/db-design.md`
- Modify: `docs/development-status.md`
- Modify: `docs/frontend-design.md`
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `docs/superpowers/plans/2026-11-03-run-private-struct-parameter-overrides.md`

**Interfaces:**
- Consumes: all final behavior and exact automated test names from Tasks 1–4.
- Produces: current public contract and functional evidence for run-private Struct parameters, source-less inputs, and provenance-safe replay.

- [x] **Step 1: Update authoritative documents**

Document that study Struct edits never publish resources, source/default/effective snapshot shapes, explicit replacement-lineage following, same-alias rejection, source-less retry/rerun, legacy snapshot fallback, and the new frontend copy. Keep standalone resource replacement semantics unchanged.

- [x] **Step 2: Update functional evidence**

Add the exact backend and frontend behavior test names to the relevant AGENT/RUN/UI contract entries in `features.yaml`. Do not use registry-enumeration tests as evidence.

- [ ] **Step 3: Run formatting and static checks**

Run:

```bash
cd backend
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: both commands exit 0 with no warnings.

- [ ] **Step 4: Run the full deterministic suite**

Run:

```bash
cd backend
cargo test --features e2e-test
cd ../frontend
npm test -- --run
npm run build
cd ..
./scripts/check-functional-coverage --require-pr
```

Expected: backend and frontend suites PASS, build PASS, coverage gate reports 100% with no missing contracts.

Progress: formatting/static checks and the full deterministic suite are intentionally deferred until the user requests them because other processes are modifying the workspace.

- [x] **Step 5: Review the final diff and dirty-worktree boundary**

Run `git diff --check`, inspect every task commit, and verify no pre-existing charting file or hunk was staged. Confirm no study confirmation path writes a Struct data resource and all private artifacts have consume/release coverage.

Result: scoped `git diff --check` passed; Tasks 1–4 contain only their intended backend/frontend files, no charting file was staged, and all concurrent chart-workbench edits remain outside this task. Full static and deterministic verification remains deferred as noted above.

Independent review found and the implementation now fixes two replay edges: Struct snapshots without an `effective` overlay also require explicit replacement lineage, and stale retry-overlay materialization reports `configuration_stale` instead of `invalid_run`. The same-alias/no-overlay regression passed its red-green run; mapping and >64-depth tests were added. Their focused execution is pending because concurrent chart-schema edits currently leave unrelated `DatasetView` imports uncompilable. Follow-up read-only review reports no remaining critical, important, or minor findings.

- [x] **Step 6: Commit documentation and traceability only**

Stage only Task 5 files and commit:

`docs: document run-private study parameters`
