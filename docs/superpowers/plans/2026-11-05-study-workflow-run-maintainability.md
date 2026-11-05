# Study Workflow and Run Maintainability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reassign study configuration, frozen input, Struct overlay, managed workflow, script retry, and ordinary run responsibilities so shared safety rules have one implementation while public V4 behavior remains unchanged.

**Architecture:** Keep `StudyConfigurationService`, `ManagedWorkflowService`, `ManagedWorkflowRepo`, `RunService`, and `RunRepo` as stable facades. Introduce one frozen-input/lineage repository boundary, stage Struct overlays until the workflow/run input transaction atomically commits their artifact links, and share only input-tree, physical-result, and failure-diagnostic primitives between managed and ordinary execution; their state machines and publication transactions stay separate.

**Tech Stack:** Rust 2024, Axum, Tokio, Diesel/SQLite immediate transactions, Arrow/Parquet 60, Snappy Parquet, React/TypeScript/Vitest/Playwright for unchanged consumer verification.

**Spec:** `docs/superpowers/specs/2026-11-05-study-workflow-run-maintainability-design.md`

## Global Constraints

- Modify directly on `master`; do not create a feature worktree unless the user changes the repository instruction.
- All tensoreval calls remain inside `backend/src/adapters/`; shared application modules only create controlled adapter specs.
- Every repository query remains explicitly user-scoped; missing and cross-user resources remain indistinguishable 404 responses.
- Keep all HTTP/SSE routes, JSON DTOs, stable error codes, database schema, Agent tools/skills, prompts, and frontend flows unchanged.
- Keep exact configuration revision, engine snapshot, facade hash, active-model, resource-hash, and single-live-workflow confirmation gates.
- Keep explicit replacement lineage, the 64-hop limit, cycle/same-alias rejection, source-less input behavior, and legacy merged-overrides fallback.
- Keep managed correction budget at at most 3 repairs and 4 engine executions; ordinary runs never repair.
- Keep managed and ordinary state machines, result policies, cancellation semantics, and terminal publication transactions separate.
- New Struct overlays remain staged until artifact/link and frozen input are committed in one immediate transaction through the existing artifact batch primitive.
- Do not weaken or reclassify the known seven external `case_projection_invalid` failures in the current adjacent eig-rs tree.

## Review Focus

- A concurrent confirmation/retry/run replay arriving after overlays are staged must return the immutable prior operation without committing staged artifacts or changing quota; pinned in Tasks 1 and 4.
- A multi-overlay request whose later item fails staging or transaction validation must clean earlier staging and leave artifact/link/input counts unchanged; pinned in Tasks 1 and 4.
- Source-less, unchanged-source, explicit replacement, legacy merged-overrides, same-alias, cyclic, and over-64-depth inputs must resolve identically for workflow retry and Agent-script rerun; pinned in Tasks 1 and 3.
- Cancellation during author generation, engine execution, and repair must prevent later attempts or publication while preserving the existing with-candidate/without-candidate terminal contract; pinned in Tasks 1 and 8.
- The same physically valid output must remain acceptable to an open manual script while being rejected by a frozen/managed result contract when its logical schema drifts; pinned in Tasks 7 and 8.

---

### Task 1: Freeze the Cross-Path Compatibility Matrix

**Files:**
- Modify: `backend/tests/managed_workflows.rs`
- Modify: `backend/tests/workflow_retries.rs`
- Modify: `backend/tests/simulation_runs.rs`
- Modify: `backend/tests/script_repair.rs`
- Modify: `backend/tests/study_configurations.rs`
- Create: `.superpowers/sdd/2026-11-05-study-workflow-run-maintainability/task-1-brief.md` (ignored execution ledger only)

**Interfaces:**
- Consumes: Current public application services, HTTP routes, SQLite schema, and artifact usage APIs.
- Produces: Behavior-level compatibility tests that every later task must keep green; no production interface.

- [ ] **Step 1: Add the frozen-input parity tests**

Add `retry_and_rerun_resolve_the_same_struct_successor_matrix` in `workflow_retries.rs`. Build table-driven cases for unchanged source, explicit replacement, source-less input, legacy merged overrides, same-alias impostor, cycle, and a 65-link chain. Assert retry preview/confirm and script rerun either freeze the same source/effective hash or return their existing path-specific stable error with no new workflow/run.

- [ ] **Step 2: Add artifact and replay accounting tests**

Add `later_overlay_failure_cleans_earlier_material` in `managed_workflows.rs` and `concurrent_replay_never_consumes_uncommitted_overlay_artifacts` in `workflow_retries.rs`. Assert `storage_scope_artifacts`, `used_artifacts`, `used_bytes`, workflow/run/input counts, and replayed IDs before and after the operation.

- [ ] **Step 3: Add the three-phase cancellation matrix**

Extend `script_repair.rs` with `cancellation_during_generation_execution_and_repair_stops_future_work`. Use existing scenario barriers; assert no later engine/author call, no partial result, and the current candidate/no-candidate script anchor contract.

- [ ] **Step 4: Lock configuration and ordinary-run compatibility**

Extend `study_configurations.rs` and `simulation_runs.rs` to assert numeric JSON equivalence, selector-field preservation, open manual result acceptance, frozen Agent rerun contract rejection, stable public error details, and user-scoped replay.

- [ ] **Step 5: Run the compatibility baseline**

Run:

```bash
cd backend
cargo test --features e2e-test --test managed_workflows --test workflow_retries --test simulation_runs --test script_repair --test study_configurations
```

Expected: all compatibility tests pass on the pre-refactor implementation. If a proposed assertion fails, determine whether it exposes a real current defect; do not silently redefine the baseline.

- [ ] **Step 6: Commit**

```bash
git add backend/tests/managed_workflows.rs backend/tests/workflow_retries.rs backend/tests/simulation_runs.rs backend/tests/script_repair.rs backend/tests/study_configurations.rs
git commit -m "test: characterize workflow input compatibility"
```

### Task 2: Centralize Study Policies and Retry Tokens

**Files:**
- Create: `backend/src/application/study_configuration_policy.rs`
- Modify: `backend/src/application/study_configurations.rs`
- Modify: `backend/src/application/managed_workflows.rs`
- Modify: `backend/src/application/mod.rs`
- Test: `backend/src/application/study_configuration_policy.rs`
- Test: `backend/tests/workflow_retries.rs`

**Interfaces:**
- Consumes: `opaque_token::{generate_token, hash_token, is_generated_token}` and existing catalog contracts.
- Produces:
  - `pub(crate) fn json_values_equivalent(left: &Value, right: &Value) -> bool`
  - `pub(crate) fn result_selector_fields(facade: &FacadeContract, input: &FacadeInputContract) -> Result<Vec<String>, StudyConfigurationError>`
  - `pub(crate) fn apply_result_selector_overrides(facade: &FacadeContract, input: &FacadeInputContract, fields: &mut Value, selected_results: &[String]) -> Result<Value, StudyConfigurationError>`
  - `pub(crate) fn validate_struct_values(snapshot: &EngineResourceSnapshot, schema_ref: &str, values: &Value, protected_fields: &[String]) -> Result<(), StudyConfigurationError>`

- [ ] **Step 1: Write policy module tests before moving implementation**

Add unit tests for `50 == 50.0`, selector ordering/deduplication, protected selector preservation, unknown fields, nullability, range violations, list/matrix shape errors, and unavailable Struct schema. Reference the new module functions so the test compile initially fails.

- [ ] **Step 2: Run the new unit tests and verify RED**

Run: `cd backend && cargo test --features e2e-test --lib study_configuration_policy`

Expected: compile/test failure because `study_configuration_policy` does not yet export the specified functions.

- [ ] **Step 3: Move the pure rules into `study_configuration_policy.rs`**

Move, do not duplicate, the numeric comparison, selector-field, selector-override, Struct validation, and field-validation logic. Keep error codes/messages byte-compatible and keep database/resource reads in `StudyConfigurationService`.

- [ ] **Step 4: Replace retry token generation with the opaque-token API**

In `ManagedWorkflowService::preview_retry`, call `generate_token()` and `hash_token()`. In confirm, require `is_generated_token(retry_preview_id)` before hashing so retry continues to accept only a current 64-character lowercase-hex token, not legacy data-resource tokens.

- [ ] **Step 5: Run policy and workflow tests**

Run:

```bash
cd backend
cargo test --features e2e-test --lib study_configuration_policy
cargo test --features e2e-test --test study_configurations --test workflow_retries
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS; `rg -n "getrandom|Sha256::digest\(retry" backend/src/application/managed_workflows.rs` returns no retry-token implementation.

- [ ] **Step 6: Commit**

```bash
git add backend/src/application/study_configuration_policy.rs backend/src/application/study_configurations.rs backend/src/application/managed_workflows.rs backend/src/application/mod.rs backend/tests/workflow_retries.rs
git commit -m "refactor: centralize study policies and retry tokens"
```

### Task 3: Establish One Frozen Struct Input Resolver

**Files:**
- Create: `backend/src/db/repositories/workflow_input.rs`
- Modify: `backend/src/db/repositories/resource_lineage.rs`
- Modify: `backend/src/db/repositories/managed_workflow.rs`
- Modify: `backend/src/db/repositories/run.rs`
- Modify: `backend/src/db/repositories/mod.rs`
- Test: `backend/tests/workflow_retries.rs`
- Test: `backend/tests/simulation_runs.rs`

**Interfaces:**
- Consumes: `frozen_struct_overrides`, `resolve_live_struct_successor`, `WorkflowInputSnapshotRecord`, and exact `UserId`/session ownership.
- Produces:
  - `pub struct StructOverlayRequest { input_id, source_resource_id, source_artifact_id, source_content_hash, source_schema_hash, schema_ref, user_overrides, selector_overrides, overrides }`
  - `pub(crate) struct ResolvedFrozenStructInput { request: Option<StructOverlayRequest>, effective_artifact_id: i32, effective_content_hash: String, source_changed: bool }`
  - `pub(crate) fn resolve_frozen_struct_input_on(conn: &mut SqliteConnection, session_id: i32, snapshot: &WorkflowInputSnapshotRecord) -> RepoResult<ResolvedFrozenStructInput>`
  - `WorkflowInputRepo::overlay_requests_for_script(user: &UserId, script_id: i32) -> Result<Vec<StructOverlayRequest>, RepoError>`

- [ ] **Step 1: Add direct parity assertions to Task 1’s matrix**

Make the retry and rerun tests assert exact request source ID/artifact/content/schema hashes and exact classified overrides through their public outcomes. Add a case where current alias matches but there is no explicit replacement lineage.

- [ ] **Step 2: Run the parity tests and record the baseline**

Run: `cd backend && cargo test --features e2e-test --test workflow_retries --test simulation_runs`

Expected: PASS before movement; save exact counts in the task ledger.

- [ ] **Step 3: Implement `workflow_input.rs`**

Move shared snapshot parsing and live-successor resolution behind the produced interfaces. Keep the 64-hop bound, cycle detection, exact alias/schema checks, source-less reuse, and legacy merged-overrides fallback in one path. `resource_lineage.rs` retains the lower-level recursive database helpers.

- [ ] **Step 4: Migrate both consumers**

Change `ManagedWorkflowRepo` retry material/overlay request creation and `RunRepo::struct_overlay_requests` to call `workflow_input` interfaces. Delete their local JSON/source/successor inference; preserve each repository’s error mapping.

- [ ] **Step 5: Run frozen-input and deletion-safety tests**

Run:

```bash
cd backend
cargo test --features e2e-test --test workflow_retries --test simulation_runs --test managed_workflows --test data_resources --test session_deletion
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS; `rg -n "frozen_struct_overrides|resolve_live_struct_successor" backend/src/db/repositories/managed_workflow.rs backend/src/db/repositories/run.rs` returns no direct consumers.

- [ ] **Step 6: Commit**

```bash
git add backend/src/db/repositories/workflow_input.rs backend/src/db/repositories/resource_lineage.rs backend/src/db/repositories/managed_workflow.rs backend/src/db/repositories/run.rs backend/src/db/repositories/mod.rs backend/tests/workflow_retries.rs backend/tests/simulation_runs.rs
git commit -m "refactor: unify frozen struct input resolution"
```

### Task 4: Stage Overlays and Commit Them Atomically With Frozen Inputs

**Files:**
- Create: `backend/src/application/struct_overlays.rs`
- Modify: `backend/src/application/data_resources.rs`
- Delete: `backend/src/application/study_struct_overlays.rs`
- Modify: `backend/src/application/managed_workflows.rs`
- Modify: `backend/src/application/runs.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/db/repositories/artifact.rs`
- Modify: `backend/src/db/repositories/managed_workflow.rs`
- Modify: `backend/src/db/repositories/run.rs`
- Modify: `backend/src/db/repositories/mod.rs`
- Test: `backend/tests/managed_workflows.rs`
- Test: `backend/tests/workflow_retries.rs`
- Test: `backend/tests/simulation_runs.rs`
- Test: `backend/tests/artifact_store.rs`

**Interfaces:**
- Consumes: Task 3 `StructOverlayRequest`, `ArtifactMetadata::parquet`, `ArtifactRepo::commit_batch_with_publish`, current engine schema snapshot, and `ArtifactStore` staging/publish.
- Produces:
  - repository command enum `pub enum OverlayArtifactSource { Existing { artifact_id: i32, content_hash: String }, Batch { draft_index: usize, content_hash: String } }`
  - repository command `pub struct WorkflowOverlayInput { input_id, source_resource_id, source_artifact_id, source_content_hash, source_schema_hash, schema_ref, schema_hash, effective_values_json, user_overrides_json, selector_overrides_json, overrides_json, artifact: OverlayArtifactSource }`
  - `pub(crate) struct StagedOverlaySet { inputs: Vec<WorkflowOverlayInput>, drafts: Vec<ArtifactDraft>, staged: Vec<StagedArtifact> }`
  - `StagedOverlaySet::into_parts(self) -> (Vec<WorkflowOverlayInput>, Vec<ArtifactDraft>, Vec<StagedArtifact>)`
  - `StructOverlayService::stage_initial(user: &CurrentUser, plans: &[PreparedStructOverlay]) -> Result<StagedOverlaySet, StructOverlayError>`
  - `StructOverlayService::stage_requests(user: &CurrentUser, requests: &[StructOverlayRequest]) -> Result<StagedOverlaySet, StructOverlayError>`
  - `ManagedWorkflowRepo::confirm_and_start_with_overlay_artifacts<P>(user: &UserId, prepared: PreparedStudyConfirmation, inputs: Vec<WorkflowOverlayInput>, drafts: Vec<ArtifactDraft>, publish: P, operation: String, correction_budget: i32, agent_bundle_version: String, agent_skill_refs_json: String, now_ms: i64) -> Result<ConfirmStartOutcome, ManagedWorkflowRepoError>`
  - `ManagedWorkflowRepo::confirm_retry_with_overlay_artifacts<P>(user: &UserId, token_hash: String, inputs: Vec<WorkflowOverlayInput>, drafts: Vec<ArtifactDraft>, publish: P, engine_snapshot_id: String, current_facade_hash: String, correction_budget: i32, agent_bundle_version: String, agent_skill_refs_json: String, now_ms: i64) -> Result<ManagedWorkflowAggregate, ManagedWorkflowRepoError>`
  - `RunRepo::create_with_overlay_artifacts<P>(user: &UserId, request: CreateRun, inputs: Vec<WorkflowOverlayInput>, drafts: Vec<ArtifactDraft>, publish: P) -> Result<CreatedRun, RunRepoError>`

For all three generic methods, `P: FnMut(usize, &str) -> Result<(), RepoError> + Send + 'static`; each resolves `draft_index` only inside the artifact batch transaction.

- [ ] **Step 1: Write atomic staging failure tests**

Update Task 1 integration tests to inject: failure on the second staging item, stale resource during transaction finalization, missing scope link, and a concurrent replay after staging. Also add `staging_overlay_does_not_change_scope_usage_before_repository_commit` as a `struct_overlays.rs` unit test. Assert no unowned logical link, no quota drift, no partial workflow/run/input, and the same immutable replay ID.

- [ ] **Step 2: Run the new tests and verify RED**

Run:

```bash
cd backend
cargo test --features e2e-test --lib staging_overlay_does_not_change_scope_usage_before_repository_commit -- --exact
```

Expected: compile/test failure because `StructOverlayService` and the staged-overlay interface do not exist yet.

- [ ] **Step 3: Implement `StructOverlayService` staging**

Move overlay value reconstruction, schema validation, Parquet creation, source-content reuse detection, and staging out of `DataResourceService`. Do not call `ArtifactService::commit`; return existing descriptors or staged drafts. Ensure `StagedOverlaySet` owns every staging file until publication or drop.

- [ ] **Step 4: Add artifact/input atomic repository entry points**

Use `ArtifactRepo::commit_batch_with_publish` for initial confirmation, retry confirm, and Agent rerun create. In the finalizer transaction, resolve every `draft_index`, revalidate all mutable inputs, and insert frozen inputs. A batch-created link is adopted by exactly one input without an extra retain; an existing descriptor is retained once per frozen input. Transaction rollback leaves no artifact/link/quota row.

- [ ] **Step 5: Migrate application callers and remove old ownership**

Inject `StructOverlayService` into `ManagedWorkflowService` and `RunService`. Replace both `DataResourceService::materialize_*` calls, delete those methods and `study_struct_overlays.rs`, and keep existing public error mapping and replay-before-staging order.

- [ ] **Step 6: Run atomicity and lifecycle tests**

Run:

```bash
cd backend
cargo test --features e2e-test --test artifact_store --test managed_workflows --test workflow_retries --test simulation_runs --test data_resources --test quota_concurrency --test failure_atomicity
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS; `rg -n "materialize_(study|retry)_struct_overlays" backend/src` and `rg -n "study_struct_overlays" backend/src` return no matches.

- [ ] **Step 7: Commit**

```bash
git add backend/src/application/struct_overlays.rs backend/src/application/data_resources.rs backend/src/application/study_struct_overlays.rs backend/src/application/managed_workflows.rs backend/src/application/runs.rs backend/src/application/mod.rs backend/src/db/repositories/artifact.rs backend/src/db/repositories/workflow_input.rs backend/src/db/repositories/managed_workflow.rs backend/src/db/repositories/run.rs backend/src/db/repositories/mod.rs backend/tests/managed_workflows.rs backend/tests/workflow_retries.rs backend/tests/simulation_runs.rs backend/tests/artifact_store.rs
git commit -m "refactor: atomically freeze staged struct overlays"
```

### Task 5: Split Managed Workflow Repository Transactions by Ownership

**Files:**
- Create: `backend/src/db/repositories/managed_workflow_confirmation.rs`
- Create: `backend/src/db/repositories/managed_workflow_retry.rs`
- Create: `backend/src/db/repositories/managed_workflow_attempt.rs`
- Create: `backend/src/db/repositories/managed_workflow_recovery.rs`
- Modify: `backend/src/db/repositories/managed_workflow.rs`
- Modify: `backend/src/db/repositories/mod.rs`
- Modify: `backend/src/application/managed_workflows.rs`
- Test: `backend/tests/managed_workflows.rs`
- Test: `backend/tests/workflow_retries.rs`
- Test: `backend/tests/script_repair.rs`
- Test: `backend/tests/failure_atomicity.rs`

**Interfaces:**
- Consumes: Task 3 workflow-input resolver and Task 4 staged-overlay artifact finalization.
- Produces:
  - `pub struct ManagedWorkflowStartCommand` containing only stable IDs, hashes, frozen JSON, correction budget, provenance, operation ID, and timestamp—no application service/catalog types.
  - The existing `ManagedWorkflowRepo` public methods and `ManagedWorkflowAggregate`/error behavior, implemented in responsibility-specific child modules.
  - `pub(crate) fn load_owned_on(conn: &mut SqliteConnection, user: &str, workflow_id: i32) -> Result<ManagedWorkflowAggregate, ManagedWorkflowRepoError>` as the single scoped aggregate loader.

- [ ] **Step 1: Add the start-command conversion test**

Add `start_command_preserves_every_locked_confirmation_field` to the `managed_workflows.rs` unit tests. Construct a `PreparedStudyConfirmation`, convert it to `ManagedWorkflowStartCommand`, and assert exact configuration/session/revision, locked-contract JSON, frozen model/resources/results, overlay descriptors, provenance, operation ID, correction budget, and timestamp. The test initially fails to compile because the command/conversion does not exist; Task 9 separately audits that repositories no longer import `PreparedStudyConfirmation`.

- [ ] **Step 2: Run workflow transaction tests before movement**

Run: `cd backend && cargo test --features e2e-test --test managed_workflows --test workflow_retries --test script_repair --test failure_atomicity`

Expected: PASS; record counts.

- [ ] **Step 3: Introduce `ManagedWorkflowStartCommand`**

Convert `PreparedStudyConfirmation` to the command in application code before repository entry. Move any mutation of the application object into explicit command construction or repository-owned frozen records; repository modules must not import application DTOs.

- [ ] **Step 4: Move repository implementations without splitting transactions**

Move confirmation/replay/start to `managed_workflow_confirmation.rs`, retry preview/confirm/material to `managed_workflow_retry.rs`, attempt/candidate/terminal publication to `managed_workflow_attempt.rs`, and cancel/recovery to `managed_workflow_recovery.rs`. Keep shared types and the single scoped loader in `managed_workflow.rs`; child helpers take the caller’s `&mut SqliteConnection`.

- [ ] **Step 5: Run transaction, recovery, and schema tests**

Run:

```bash
cd backend
cargo test --features e2e-test --test managed_workflows --test workflow_retries --test script_repair --test failure_atomicity --test v4_schema --test session_deletion
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS; `rg -n "PreparedStudyConfirmation" backend/src/db/repositories` returns no matches.

- [ ] **Step 6: Commit**

```bash
git add backend/src/db/repositories/managed_workflow.rs backend/src/db/repositories/managed_workflow_confirmation.rs backend/src/db/repositories/managed_workflow_retry.rs backend/src/db/repositories/managed_workflow_attempt.rs backend/src/db/repositories/managed_workflow_recovery.rs backend/src/db/repositories/mod.rs backend/src/application/managed_workflows.rs backend/tests/managed_workflows.rs backend/tests/workflow_retries.rs backend/tests/script_repair.rs backend/tests/failure_atomicity.rs
git commit -m "refactor: split managed workflow transactions"
```

### Task 6: Split Study and Workflow Application Orchestration

**Files:**
- Create: `backend/src/application/study_configuration_prepare.rs`
- Create: `backend/src/application/study_configuration_inputs.rs`
- Create: `backend/src/application/study_confirmation.rs`
- Create: `backend/src/application/workflow_retry.rs`
- Modify: `backend/src/application/study_configurations.rs`
- Modify: `backend/src/application/managed_workflows.rs`
- Modify: `backend/src/application/mod.rs`
- Test: `backend/tests/study_configurations.rs`
- Test: `backend/tests/managed_workflows.rs`
- Test: `backend/tests/workflow_retries.rs`
- Test: `backend/tests/app_bootstrap.rs`

**Interfaces:**
- Consumes: Task 2 pure policies, Task 4 `StructOverlayService`, and Task 5 `ManagedWorkflowStartCommand`.
- Produces:
  - Stable `StudyConfigurationService` and `ManagedWorkflowService` facade methods used by API/tools.
  - `pub(crate) struct ManagedWorkflowDependencies` with named repository, configuration, event, artifact/overlay, author/analyzer, engine, snapshot, scheduler/cancellation, timeout/limits, scripts, and auto-start fields.
  - `workflow_retry.rs` owns retry preview/confirm validation and command construction.

- [ ] **Step 1: Add assembly and facade compatibility tests**

Extend `app_bootstrap.rs` to build the named dependencies without positional variants. Assert all existing study/workflow handlers compile against unchanged facade signatures and the same errors/views.

- [ ] **Step 2: Run the focused application suite before movement**

Run: `cd backend && cargo test --features e2e-test --test app_bootstrap --test study_configurations --test managed_workflows --test workflow_retries`

Expected: PASS; record counts.

- [ ] **Step 3: Split `StudyConfigurationService` implementation**

Move prepare/facade selection to `study_configuration_prepare.rs`, resource/Struct/result edits to `study_configuration_inputs.rs`, and exact confirmation preparation/locked contract construction to `study_confirmation.rs`. Keep DTOs, errors, constructor, `get`, view mapping, and stable facade declarations in `study_configurations.rs`.

- [ ] **Step 4: Split retry orchestration and use named dependencies**

Move retry preview/confirm methods and error mapping to `workflow_retry.rs`. Replace the long `ManagedWorkflowService::new` signature with `ManagedWorkflowDependencies`; update only `AppState` assembly and test assembly call sites.

- [ ] **Step 5: Run application, tool, and API contract tests**

Run:

```bash
cd backend
cargo test --features e2e-test --test app_bootstrap --test study_configurations --test managed_workflows --test workflow_retries --test workspace_tools --test contract_manifest
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS; public route/tool counts and stable codes are unchanged.

- [ ] **Step 6: Commit**

```bash
git add backend/src/application/study_configuration_prepare.rs backend/src/application/study_configuration_inputs.rs backend/src/application/study_confirmation.rs backend/src/application/workflow_retry.rs backend/src/application/study_configurations.rs backend/src/application/managed_workflows.rs backend/src/application/mod.rs backend/tests/app_bootstrap.rs backend/tests/study_configurations.rs backend/tests/managed_workflows.rs backend/tests/workflow_retries.rs
git commit -m "refactor: separate study and workflow orchestration"
```

### Task 7: Extract Shared Execution Safety Primitives

**Files:**
- Create: `backend/src/application/execution_input_tree.rs`
- Create: `backend/src/application/execution_results.rs`
- Create: `backend/src/application/execution_failure.rs`
- Modify: `backend/src/application/workflow_worker.rs`
- Modify: `backend/src/application/runs.rs`
- Modify: `backend/src/application/diagnostics.rs`
- Modify: `backend/src/application/mod.rs`
- Test: `backend/src/application/execution_input_tree.rs`
- Test: `backend/src/application/execution_results.rs`
- Test: `backend/src/application/execution_failure.rs`
- Test: `backend/tests/process_engine_v4.rs`
- Test: `backend/tests/simulation_runs.rs`
- Test: `backend/tests/script_repair.rs`

**Interfaces:**
- Consumes: Frozen model/resource/script artifact descriptors, `ArtifactService`, adapter `NamedInput`/dataset outputs, catalog dtype/unit contracts, and existing sanitizers.
- Produces:
  - `pub(crate) struct ExecutionInputSpec { script_artifact_id, model_frames, resources }`
  - `pub(crate) struct ExecutionInputTree { root: PathBuf, input_dir: PathBuf, result_dir: PathBuf, script_path: PathBuf, named_inputs: Vec<NamedInput> }` with RAII cleanup.
  - `pub(crate) async fn build_execution_input_tree(user: &CurrentUser, artifacts: &ArtifactService, data_dir: &Path, owner_kind: &str, owner_id: i32, spec: ExecutionInputSpec) -> Result<ExecutionInputTree, ExecutionInputError>`
  - `pub(crate) fn inspect_engine_outputs(outputs: &[EngineDatasetOutput], max_file_bytes: u64) -> Result<Vec<InspectedExecutionOutput>, ExecutionResultError>`
  - `pub(crate) fn failure_analysis_request(current_script: String, adapter_error: &AdapterError, locked_contract: Option<Value>, inputs: Option<Value>) -> FailureAnalysisRequest`

- [ ] **Step 1: Write primitive tests and result-policy divergence tests**

Test safe relative paths, duplicate aliases, missing owned artifacts, RAII cleanup, zero-row Parquet, nested list dtype/unit metadata, non-finite/null distinction, bounded UTF-8 stdout/stderr, path redaction, and identical physical output evaluated by open versus strict/frozen policies.

- [ ] **Step 2: Run primitive tests and verify RED**

Run: `cd backend && cargo test --features e2e-test --lib execution_`

Expected: compile/test failure because the modules/interfaces do not exist.

- [ ] **Step 3: Implement input-tree and physical-result primitives**

Move only identical filesystem/artifact-copy and physical Parquet inspection behavior. The builder rejects duplicate aliases and unsafe relative paths before engine invocation. `inspect_engine_outputs` returns inspected facts; it does not decide managed/open/frozen business acceptance.

- [ ] **Step 4: Implement shared failure-diagnostic construction**

Use existing sanitizers and 64 KiB UTF-8 head/tail rules. Return the analyzer request and public/audit metadata without writing run/workflow state or invoking the analyzer.

- [ ] **Step 5: Migrate duplicate helper calls but retain local policies**

Change `WorkflowWorker` and `RunService` to use the new primitives for input-tree setup, physical output inspection, and analyzer request construction. Keep managed schema/result validation and ordinary open/frozen decisions in their existing consumers until Task 8.

- [ ] **Step 6: Run safety and execution tests**

Run:

```bash
cd backend
cargo test --features e2e-test --lib execution_
cargo test --features e2e-test --test process_engine_v4 --test simulation_runs --test script_repair --test failure_atomicity
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS; no process construction appears outside `backend/src/adapters/`.

- [ ] **Step 7: Commit**

```bash
git add backend/src/application/execution_input_tree.rs backend/src/application/execution_results.rs backend/src/application/execution_failure.rs backend/src/application/workflow_worker.rs backend/src/application/runs.rs backend/src/application/diagnostics.rs backend/src/application/mod.rs backend/tests/process_engine_v4.rs backend/tests/simulation_runs.rs backend/tests/script_repair.rs
git commit -m "refactor: share execution safety primitives"
```

### Task 8: Isolate Managed Worker and Ordinary Run State Machines

**Files:**
- Create: `backend/src/application/run_execution.rs`
- Create: `backend/src/application/run_results.rs`
- Create: `backend/src/application/run_deletion.rs`
- Create: `backend/src/db/repositories/run_deletion.rs`
- Modify: `backend/src/application/runs.rs`
- Modify: `backend/src/application/workflow_worker.rs`
- Modify: `backend/src/db/repositories/run.rs`
- Modify: `backend/src/application/mod.rs`
- Test: `backend/tests/simulation_runs.rs`
- Test: `backend/tests/result_deletion.rs`
- Test: `backend/tests/script_repair.rs`
- Test: `backend/tests/workflow_retries.rs`
- Test: `backend/tests/session_deletion.rs`

**Interfaces:**
- Consumes: Task 7 shared primitives and existing `RunRepo`/`ManagedWorkflowRepo` state transitions.
- Produces:
  - `runs.rs` as DTO/error/facade/assembly owner.
  - `run_execution.rs` as single-execution orchestration owner for ordinary and Agent rerun runs.
  - `run_results.rs` with explicit `OpenResultPolicy` and `FrozenResultPolicy`; neither can request repair.
  - `run_deletion.rs` application/repository modules owning preview/token/tombstone while retaining the `RunService`/`RunRepo` facade.
  - `WorkflowWorker` retaining author/repair loop, correction budget, managed strict-result policy, attempt state, and managed finalization only.

- [ ] **Step 1: Extend cancellation and policy tests from Review Focus**

Assert all three cancellation barriers stop later work, open manual results remain accepted, frozen rerun drift remains terminal without repair, managed drift consumes repair budget only when repairable, and run deletion still atomically releases script/input/direct-result references.

- [ ] **Step 2: Run state-machine tests before movement**

Run: `cd backend && cargo test --features e2e-test --test simulation_runs --test result_deletion --test script_repair --test workflow_retries --test session_deletion`

Expected: PASS; record counts.

- [ ] **Step 3: Move ordinary execution and result policy**

Move `execute_inner`, ordinary progress stages, open/frozen result decisions, failure terminalization, and asynchronous failure enrichment into `run_execution.rs`/`run_results.rs`. Keep `RunService` public methods and scheduler admission unchanged.

- [ ] **Step 4: Move run deletion transaction ownership**

Move deletion impact/token/tombstone implementation to application/repository `run_deletion.rs` child modules. Reuse `opaque_token` without changing current token format/error mapping; keep direct-result deletion atomic and derived lineage readable.

- [ ] **Step 5: Reduce `WorkflowWorker` to managed-only orchestration**

Remove migrated physical helpers. Keep author/repair prompts, at most 3 corrections/4 calls, parent attempt lineage, repairability decisions, strict managed result policy, candidate finalization, and cancellation/recovery semantics.

- [ ] **Step 6: Run state-machine, recovery, and contract suites**

Run:

```bash
cd backend
cargo test --features e2e-test --test simulation_runs --test result_deletion --test script_repair --test workflow_retries --test managed_workflows --test session_deletion --test scripts --test contract_manifest
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS; route/DTO/error manifests remain unchanged and ordinary execution contains no repair path.

- [ ] **Step 7: Commit**

```bash
git add backend/src/application/run_execution.rs backend/src/application/run_results.rs backend/src/application/run_deletion.rs backend/src/application/runs.rs backend/src/application/workflow_worker.rs backend/src/application/mod.rs backend/src/db/repositories/run_deletion.rs backend/src/db/repositories/run.rs backend/tests/simulation_runs.rs backend/tests/result_deletion.rs backend/tests/script_repair.rs backend/tests/workflow_retries.rs backend/tests/session_deletion.rs
git commit -m "refactor: isolate workflow and run state machines"
```

### Task 9: Document Ownership, Audit Boundaries, and Run the Repository Gate

**Files:**
- Modify: `docs/development-status.md`
- Modify: `docs/api-design.md` only where internal flow/ownership text is now inaccurate
- Modify: `docs/db-design.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/superpowers/specs/2026-11-05-study-workflow-run-maintainability-design.md` status/evidence only
- Modify: `integration-tests/catalog/features.yaml` only if a public contract actually changed (default: no change)
- Test: full repository gates

**Interfaces:**
- Consumes: Tasks 1–8 completed behavior and module ownership.
- Produces: Current documentation, mechanical boundary evidence, final independent review, and a clean `master` worktree.

- [ ] **Step 1: Update current documentation from implemented facts**

Record the final module owners, atomic staged-overlay handoff, shared execution primitives, separate state machines, deleted duplicate paths, actual test counts, and known external eig-rs status. Do not copy exact public enum counts into multiple documents.

- [ ] **Step 2: Run mechanical boundary audits**

Run:

```bash
rg -n "materialize_(study|retry)_struct_overlays|study_struct_overlays" backend/src
rg -n "PreparedStudyConfirmation" backend/src/db/repositories
rg -n "frozen_struct_overrides|resolve_live_struct_successor" backend/src/db/repositories/managed_workflow*.rs backend/src/db/repositories/run*.rs
rg -n "getrandom|Sha256::digest" backend/src/application/managed_workflows.rs backend/src/application/workflow_retry.rs
rg -n "std::process::Command|tokio::process::Command|Command::new" backend/src --glob '!backend/src/adapters/**'
git diff --check
```

Expected: the first, second, fourth, and process scans have no prohibited matches; lineage primitives are consumed only through `workflow_input.rs`; `git diff --check` exits 0.

- [ ] **Step 3: Run backend gates**

Run:

```bash
cd backend
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --features e2e-test
```

Expected: repository tests pass. If the unfiltered suite encounters only the already documented seven external `case_projection_invalid` resources, record them separately and rerun the complete code suite with only the exact known external assertion filtered; do not broaden the filter.

- [ ] **Step 4: Run frontend and traceability gates**

Run:

```bash
cd frontend
npm test -- --run
npm run build
npm run test:e2e
cd ..
./scripts/check-functional-coverage --require-pr
```

Expected: all deterministic frontend tests/build/E2E and 100% required feature/contract coverage pass. No functional catalog edit is expected when public behavior is unchanged.

- [ ] **Step 5: Request independent code review**

Use `superpowers:requesting-code-review` with the approved spec, this plan, the implementation base commit, final head, full verification evidence, and focus on transaction atomicity, reference counts, user scoping, cancellation, retry/rerun parity, recovery, and result-policy separation. Fix every Critical/Important finding with a failing test first and rerun affected/full gates.

- [ ] **Step 6: Commit documentation or review fixes**

If only documentation changed:

```bash
git add docs
git commit -m "docs: document workflow and run ownership"
```

If code review required fixes, commit them separately as `fix: address workflow run refactor review`. Do not create empty commits.

- [ ] **Step 7: Verify the final worktree**

Run: `git status --short`

Expected: no output; branch remains `master` unless the user explicitly changes the integration instruction.

## Plan Self-Review Record

- **Spec coverage:** Tasks 2–8 cover every design owner and flow; Task 9 covers current docs, complete gates, known external status, and independent review. No spec requirement is left without a task.
- **Step scan:** Each checkbox is one test, implementation, verification, or commit action. Commands identify exact targets and expected outcomes; no step contains a hidden public-contract decision.
- **Type consistency:** `StructOverlayRequest` is produced in Task 3 and consumed in Task 4; `StagedOverlaySet` is produced/consumed in Task 4; Tasks 5–8 depend on those stable interfaces without renaming them.
- **Review Focus:** Concurrent replay and staging failures are exercised in Tasks 1/4; input lineage parity in Tasks 1/3; three-phase cancellation in Tasks 1/8; result-policy separation in Tasks 7/8.
- **Proportion:** The plan defines boundaries, signatures, tests, commands, and commit units without embedding production function bodies.
