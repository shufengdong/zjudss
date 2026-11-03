# Run-private Result Selector Overlay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make result-selector fields such as `para.result` server-managed, freeze them only in workflow/run-private Struct artifacts, and stop result selection from creating ambiguous long-lived resources.

**Architecture:** Split long-lived Struct values from effective execution values during study confirmation. A shared Struct overlay helper materializes immutable Snappy Parquet artifacts for managed confirmation, retry, and generated-script rerun, while repositories revalidate source resource hashes before consuming those artifacts. The configuration DTO marks selector fields, the backend folds legacy selector-only candidates, and the frontend renders only real unresolved inputs.

**Tech Stack:** Rust, Axum, Diesel/SQLite, Arrow/Parquet, React, TypeScript, Vitest, Playwright/functional coverage catalog.

**Spec:** `docs/superpowers/specs/2026-11-03-run-private-result-selector-overlay-design.md`

## Global Constraints

- Never mutate a published data resource or historical workflow/run input.
- Only fields named by the current facade's `OrderedUniqueSelection.selector` are server-managed.
- Private overlay artifacts must not create `data_resources` rows or participate in alias/resource candidate selection.
- User edits to ordinary Struct fields retain existing Replace/SaveAsNew semantics.
- All repository queries continue to receive `UserId`; cross-user and missing resources remain indistinguishable 404s.
- Effective Struct files remain Snappy Parquet and all execution continues through the existing tensoreval adapter boundary.
- Preserve unrelated uncommitted chart-workbench files and stage only files owned by this feature.

## Review Focus

- A forged Struct update changes `result`: Task 1 proves the persisted server-managed value is preserved and final execution still follows selected results.
- A legacy selector-only child points to a missing/replaced source: Task 1 proves it remains a normal candidate instead of being silently dropped.
- Confirmation is replayed after private materialization: Task 2 proves no duplicate workflow, resource, or retained overlay reference is created.
- The base Struct changes between retry preview/confirmation or rerun preparation/commit: Task 3 proves hash revalidation rejects stale material and releases the staged overlay.
- A facade has no result selector or selector injection is byte-identical to the source: Tasks 1 and 2 prove old behavior remains and no redundant overlay is retained.
- A durable pre-change study-config message lacks the new DTO field: Task 4 treats it as an empty list and keeps the old card renderable.

---

### Task 1: Server-owned selector fields and real resource ambiguity

**Files:**
- Modify: `backend/src/application/study_configurations.rs`
- Modify: `backend/tests/study_configurations.rs`
- Modify: `frontend/src/types/studyConfiguration.ts`

**Interfaces:**
- Produces: `StructCardView.result_selector_fields: Vec<String>` serialized as `resultSelectorFields`.
- Produces: `PreparedStructOverlay { input_id, input_alias, source_resource_id, schema_ref, effective_values, overrides }` on `PreparedStudyConfirmation`.
- Produces: shared pure helpers `result_selector_fields(...)`, `apply_result_selector_overrides(...)`, and selector-excluding Struct difference comparison for later tasks.

- [x] **Step 1: Add failing configuration tests**

Add focused tests proving: selector fields appear in `resultSelectorFields`; an update that only changes `result` is canonicalized back to its baseline; result selection produces a `PreparedStructOverlay` but no `PreparedResourceReplacement`; changing `max_iter` still requires the requested Replace/SaveAsNew action; a facade without a selector produces neither overlay nor changed behavior.

- [x] **Step 2: Run the focused tests and verify they fail**

Run: `cd backend && cargo test --test study_configurations result_selector --features e2e-test`

Expected: FAIL because the DTO/overlay contract and canonicalization do not exist.

- [x] **Step 3: Implement selector ownership in `study_configurations.rs`**

Add exact DTO and prepared-confirmation fields above. Derive selector field names only from valid `OrderedUniqueSelection` rules targeting the current Struct input. Preserve baseline values for those fields on `update_struct`, exclude them from resource diffs/actions, apply selected results only to `effective_values`, and keep `LockedStudyContract.structs[*].fields` equal to effective execution values.

- [x] **Step 4: Add failing legacy-candidate tests**

Cover a live source plus a `source_kind=struct_edit` child whose lineage diff contains only `result`, a child whose diff also contains `max_iter`, and a selector-only child whose source is no longer live. Assert only the first child is folded from configuration candidates.

- [x] **Step 5: Implement candidate folding**

Filter compatible resources only when the lineage has a same-session, same-schema, still-live source and every diff field belongs to `result_selector_fields`. Do not mutate either resource and do not fold malformed lineage.

- [x] **Step 6: Update the TypeScript DTO and run focused checks**

Add backward-compatible `resultSelectorFields?:string[]` to `StudyInputCard.structCard` (the server always emits an array; old durable
message payloads may omit it), update fixtures, then run:

`cd backend && cargo test --test study_configurations --features e2e-test`

`cd ../frontend && npm test -- --run src/components/messages/StudyConfigurationCard.test.tsx`

Expected: PASS.

- [x] **Step 7: Commit Task 1 files only**

Commit message: `feat: separate result selectors from struct resources`

### Task 2: Private Struct artifacts in initial managed confirmation

**Files:**
- Create: `backend/src/application/study_struct_overlays.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/application/data_resources.rs`
- Modify: `backend/src/application/managed_workflows.rs`
- Modify: `backend/src/db/repositories/managed_workflow.rs`
- Modify: `backend/src/db/repositories/mod.rs`
- Modify: `backend/tests/managed_workflows.rs`

**Interfaces:**
- Consumes: `PreparedStructOverlay` and selector helpers from Task 1.
- Produces: `materialize_struct_overlay(...) -> StoredStructOverlay` with `artifact_id`, effective `content_hash`, canonical values and overrides.
- Produces: `WorkflowStructOverlayMaterial` passed to `ManagedWorkflowRepo::confirm_and_start` beside existing replacement materials.
- Produces: workflow snapshot JSON with `source` and `effective` objects; `resource_id` continues to identify the long-lived source while `artifact_id/content_hash` identify the effective file.

- [x] **Step 1: Add failing managed-confirmation tests**

Assert that selecting `bus_voltage,load_state` leaves the `data_resources` count/aliases/status unchanged, freezes a workflow input whose Parquet row has exactly that `result`, records source/effective hashes and overrides, and makes a second study see the original single resource. Add byte-identical coverage asserting the source artifact is reused.

- [x] **Step 2: Run the focused tests and verify they fail**

Run: `cd backend && cargo test --test managed_workflows private_struct_overlay --features e2e-test`

Expected: FAIL because confirmation still publishes a `struct_edit` resource.

- [x] **Step 3: Extract shared Struct encoding/materialization**

Move the canonical single-row Struct Parquet encoding needed by both resource replacement and private overlays into `study_struct_overlays.rs`. Keep schema validation and Snappy metadata identical to the existing `struct_parquet` behavior. Implement `materialize_struct_overlay` without creating a data resource.

- [x] **Step 4: Thread overlay materials through confirmation**

Change `ManagedWorkflowService::confirm` to materialize replacement artifacts and private overlays, collect every newly retained artifact ID, and release unconsumed IDs on replay/error. Change `confirm_and_start` to publish real replacements first, resolve each overlay's final source resource ID, revalidate source ID/schema/hash, and insert the effective artifact into `workflow_input_snapshots` and `simulation_runs.input_snapshot_json`.

- [x] **Step 5: Pin idempotency and stale-source failures**

Add tests for the same `clientOperationId`, a source hash change between preparation and commit, and repository failure after overlay staging. Assert one workflow, no new data resource, and no leaked storage reference.

- [x] **Step 6: Run managed workflow regression tests**

Run: `cd backend && cargo test --test managed_workflows --features e2e-test`

Expected: PASS.

- [x] **Step 7: Commit Task 2 files only**

Commit message: `feat: freeze result selectors in workflow overlays`

### Task 3: Preserve overlays across retry and generated-script rerun

**Files:**
- Modify: `backend/src/application/managed_workflows.rs`
- Modify: `backend/src/application/runs.rs`
- Modify: `backend/src/application/study_struct_overlays.rs`
- Modify: `backend/src/db/repositories/managed_workflow.rs`
- Modify: `backend/src/db/repositories/run.rs`
- Modify: `backend/tests/managed_workflows.rs`
- Modify: `backend/tests/simulation_runs.rs`

**Interfaces:**
- Consumes: snapshot `source`/`effective.overrides` and `materialize_struct_overlay` from Task 2.
- Produces: retry/run preparation records that identify current long-lived inputs plus exact expected hashes.
- Produces: repository commit methods that consume pre-materialized overlays only after revalidating source resources and idempotency keys.

- [x] **Step 1: Add failing retry tests**

Prove an unchanged failed workflow retry reuses the frozen effective artifact, while a changed base resource appears in the retry diff and—after confirmation—gets a new private overlay containing the latest ordinary fields plus the original frozen selector override.

- [x] **Step 2: Refactor retry preparation and confirmation**

Compare current resources with `snapshot_json.source.contentHash`, never with the effective overlay hash. Materialize only changed Struct overlays outside the SQLite transaction, include their expected source hashes in the impact hash, and revalidate/consume or release them during `confirm_retry`.

- [x] **Step 3: Add failing generated-script rerun tests**

In `simulation_runs.rs`, update a long-lived Struct after a successful managed run, rerun its generated script, and assert the new run snapshot/Parquet combines the latest ordinary fields with the script's frozen selector override. Add a no-selector manual-script control and a stale hash/idempotent replay case.

- [x] **Step 4: Split rerun into prepare/materialize/commit**

Make `RunRepo` expose a read-only managed-input preparation record, let `RunService` read current Struct values and call the shared overlay materializer, then commit the run only after repository hash revalidation. Preserve existing scheduler admission, `clientRunId` idempotency, result contract and artifact reference accounting.

- [x] **Step 5: Run retry and run regression tests**

Run: `cd backend && cargo test --test managed_workflows --features e2e-test`

Run: `cd backend && cargo test --test simulation_runs --features e2e-test`

Expected: PASS.

- [x] **Step 6: Commit Task 3 files only**

Commit message: `fix: retain result overlays on retry and rerun`

### Task 4: Render only actionable study inputs

**Files:**
- Modify: `frontend/src/components/messages/StudyConfigurationCard.tsx`
- Modify: `frontend/src/components/studies/StructParameterCard.tsx`
- Modify: `frontend/src/components/studies/ConfigurationSummaryCard.tsx`
- Modify: `frontend/src/components/messages/StudyConfigurationCard.test.tsx`

**Interfaces:**
- Consumes: `resultSelectorFields` from Task 1 and existing binding `mapping.selectionRequired` metadata.
- Produces: one rendering decision per input: compact ready state, resource selection, Struct editor, or column mapping.

- [x] **Step 1: Add failing component tests**

Cover: `result` is absent from editable controls and the managed-field hint is visible; a resolved Struct renders ordinary fields without a redundant resource confirmation; `selectionRequired=true` renders resource selection before the Struct editor; resolved non-Struct resources render compact “已就绪”; only genuinely unresolved inputs expose confirmation buttons; an old card without `resultSelectorFields` still renders.

- [x] **Step 2: Run the focused tests and verify they fail**

Run: `cd frontend && npm test -- --run src/components/messages/StudyConfigurationCard.test.tsx`

Expected: FAIL under the current coarse `collecting_inputs` branch.

- [x] **Step 3: Implement input-state rendering**

Extract a small local `StudyInputSection` component or equivalent pure rendering helper in `StudyConfigurationCard.tsx`. Inspect `selectionRequired` before the Struct editor, compact already-bound inputs, and reserve `ColumnMappingCard` for an actually unresolved mapping case.

- [x] **Step 4: Hide selector fields and correct resource copy**

Normalize `resultSelectorFields ?? []`, then filter those names out of defaults, changed-field calculation, required validation and controls in
`StructParameterCard`. Show “结果字段由结果选择自动写入”; do not show “将创建普通资源” when only a selector overlay is pending. Keep summary resource identity separate from selected results.

- [x] **Step 5: Run frontend regression checks**

Run: `cd frontend && npm test -- --run`

Run: `cd frontend && npm run build`

Expected: PASS.

- [x] **Step 6: Commit Task 4 files only**

Commit message: `fix: show only unresolved study inputs`

### Task 5: Contracts, traceability, and full verification

**Files:**
- Modify: `docs/api-design.md`
- Modify: `docs/db-design.md`
- Modify: `docs/frontend-design.md`
- Modify: `docs/development-status.md`
- Modify: `integration-tests/catalog/features.yaml`
- Modify if required by registry checks: `backend/src/contract_manifest.rs`

**Interfaces:**
- Consumes: final behavior and test names from Tasks 1–4.
- Produces: current authoritative documentation and feature-to-test evidence.

- [x] **Step 1: Update authoritative contracts**

Document `resultSelectorFields`, server-owned update semantics, private source/effective workflow snapshots, retry/rerun reconstruction, legacy selector-only folding, and frontend actionable-input rendering. Do not describe private overlays as data resources.

- [x] **Step 2: Add functional evidence**

Add or update feature entries pointing to the exact Rust and Vitest behavior tests. Update the public contract manifest only if the DTO/registry test requires an enumerated field; do not add routes or error codes.

- [x] **Step 3: Run formatting and focused static checks**

Run: `cd backend && cargo fmt --all -- --check`

Run: `cd backend && cargo clippy --all-targets --all-features -- -D warnings`

Expected: PASS.

- [x] **Step 4: Run the full deterministic suite**

Run: `cd backend && cargo test --features e2e-test`

Run: `cd ../frontend && npm test -- --run`

Run: `cd ../frontend && npm run build`

Run: `cd .. && ./scripts/check-functional-coverage --require-pr`

Expected: all tests/builds pass and functional/public-contract coverage remains 100%.

- [x] **Step 5: Review the final diff against the spec and dirty-worktree boundary**

Verify no existing chart-workbench file was staged or overwritten, no selector-only confirmation inserts a data resource, and every newly staged artifact has a consume/release path.

- [x] **Step 6: Commit documentation and traceability only**

Commit message: `docs: document run-private result selector overlays`. Because the worktree already contains unrelated changes, use
patch/hunk staging and verify `git diff --cached --name-only` plus the cached diff before every commit; never stage the user's existing
chart-workbench hunks in `backend/src/application/mod.rs` or any other overlapping file.
