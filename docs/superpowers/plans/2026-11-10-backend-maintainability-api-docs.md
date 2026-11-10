# Backend Maintainability and API Source Documentation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Document every registered backend HTTP/SSE handler with accurate Rustdoc and improve backend maintainability through a small set of behavior-preserving, evidence-backed refactors.

**Architecture:** Keep every public contract and existing layer boundary intact. Work in reviewable slices: document the API surface, centralize repeated API query parsing, split event transport responsibilities, isolate managed-result validation, centralize SQLite insert-ID lookup, scope test-only fault injection, then align boundary documentation and authoritative design docs.

**Tech Stack:** Rust 2024, Axum 0.8, Tokio, Diesel/SQLite, Arrow/Parquet, Serde, Rustdoc, Cargo test/Clippy.

**Spec:** `docs/superpowers/specs/2026-11-10-backend-maintainability-api-docs-design.md`

## Global Constraints

- Preserve all 81 registered method/path contracts, DTOs, status codes, stable error codes, SSE payloads, database schema, Agent permissions, engine contract, and frontend behavior.
- Keep every tensoreval invocation inside `backend/src/adapters/`; do not change process arguments, timeout/cancellation, result scanning, or snapshot publication order.
- Keep repository ownership explicit through `UserId`; cross-user and absent resources remain indistinguishable 404 responses.
- Do not expose server paths, commands, SQL, arbitrary ECharts options, or new unrestricted input shapes.
- Do not add OpenAPI/Swagger dependencies and do not enable crate-wide `missing_docs`.
- Use Chinese `//!`/`///` prose consistent with the existing backend and current design documents.
- Work directly on `master`; stage only files named by the current task. Preserve and exclude the pre-existing changes to `backend/resources/agent/v4/manifest.json` and `backend/resources/agent/v4/skill-index.json`.
- Do not update `integration-tests/catalog/features.yaml` unless a public contract changes; a required public change stops this plan and returns to design.

## Review Focus

- **Route/comment drift:** a method/path in a handler comment must match `api::route_manifest()` and `docs/api-design.md`; Task 1 and Task 3 pin the 81-route manifest before and after the documentation pass.
- **Column query compatibility:** missing values, empty comma segments, whitespace, ordering, and duplicates must retain current behavior; Task 2 adds a focused unit test before sharing the parser.
- **Durable event compatibility:** malformed stored payloads must still fall back to assistant text, while event IDs, revisions, camelCase fields, and tool args remain stable; Task 4 adds characterization coverage before splitting the module.
- **Managed result contract compatibility:** Arrow string/list storage variants, unit semantics, physical nullability, actual null values, and Snappy requirements must remain unchanged; Task 5 owns focused schema/value tests.
- **Parallel failure injection:** a session-scoped injected event failure must not be consumed by another concurrently running test or session, while the E2E global control fault remains supported; Task 7 adds unit and integration coverage.

## File Structure

### New files

- `backend/src/api/query.rs` — the one comma-separated `columns` query parser used by result, resource, and model DataFrame handlers.
- `backend/src/application/events/mod.rs` — event wire types, durable-message projection, time helpers, and stable public re-exports.
- `backend/src/application/events/writer.rs` — `EventWriter` durable write-then-publish behavior and operation-card upserts.
- `backend/src/application/events/bus.rs` — session-scoped broadcast channel lifecycle.
- `backend/src/application/workflow_result_contract.rs` — managed workflow result schema, dtype/unit, and non-null value validation.

### Removed/replaced files

- `backend/src/application/events.rs` — replaced by the `events/` module directory without changing `crate::application::*` public paths.

### Existing ownership retained

- `backend/src/api/mod.rs` remains the only route registration/manifest declaration.
- `backend/src/api/error.rs` remains the only public HTTP error mapping boundary.
- `backend/src/application/workflow_worker.rs` remains the managed generation/execution/repair orchestration facade.
- `backend/src/db/repositories/artifact_refs.rs` remains the user-scope and artifact retain/release authority.
- `backend/src/db/repositories/mod.rs` owns only repository-wide mechanical helpers and shared error/state types.

---

### Task 1: Document core, authentication, session, and event APIs

**Files:**
- Modify: `backend/src/api/mod.rs`
- Modify: `backend/src/api/auth.rs`
- Modify: `backend/src/api/sessions.rs`
- Modify: `backend/src/api/events.rs`
- Modify: `backend/src/api/storage.rs`
- Modify: `backend/src/api/examples.rs`
- Modify: `backend/src/api/error.rs`

**Interfaces:**
- Consumes: `api::route_manifest() -> Vec<RouteContract>` and the current `ApiError` mappings.
- Produces: module `//!` documentation and handler `///` comments for health, auth, session/message/history/SSE, storage usage, and example routes; no signature changes.

- [ ] **Step 1: Record the route-contract baseline**

Run: `cd backend && cargo test --features e2e-test --test contract_manifest production_router_and_manifest_share_one_declaration -- --exact`

Expected: PASS with 81 unique method/path entries.

- [ ] **Step 2: Add module documentation**

Add `//!` blocks that state each module's route family, authentication boundary, application dependency, and non-ownership of domain rules. Expand `api/mod.rs` to identify route registration, auth middleware, body limits, and manifest ownership.

- [ ] **Step 3: Add handler and exported API Rustdoc**

For every handler in these files, add `///` with exact method/path, authentication/scope, non-obvious request constraint, success status/response, side effect, and route-specific stable failure semantics. Also document `RouteContract`, its fields, `router`, `route_manifest`, and the public `ApiError` boundary.

- [ ] **Step 4: Cross-check comments against source facts**

Use `backend/src/api/mod.rs`, each handler extractor/response, called application service, and `backend/src/api/error.rs`; correct any comment that claims a status, side effect, or error not present in code.

- [ ] **Step 5: Verify the slice**

Run: `cd backend && cargo fmt --all -- --check && cargo doc --no-deps --all-features && cargo test --features e2e-test --test contract_manifest --test auth_scope --test auth_sse --test history_search --test sse_recovery --test case_import`

Expected: all commands exit 0; Rustdoc contains no broken-link error.

- [ ] **Step 6: Commit**

```bash
git add backend/src/api/mod.rs backend/src/api/auth.rs backend/src/api/sessions.rs backend/src/api/events.rs backend/src/api/storage.rs backend/src/api/examples.rs backend/src/api/error.rs
git commit -m "docs: document core backend API handlers"
```

### Task 2: Centralize column queries and document upload/resource/model APIs

**Files:**
- Create: `backend/src/api/query.rs`
- Modify: `backend/src/api/mod.rs`
- Modify: `backend/src/api/attachment.rs`
- Modify: `backend/src/api/uploads.rs`
- Modify: `backend/src/api/data_resources.rs`
- Modify: `backend/src/api/model_dataframes.rs`
- Modify: `backend/src/api/datasets.rs`
- Modify: `backend/src/api/workspaces.rs`

**Interfaces:**
- Consumes: existing `columns: Option<String>` query fields.
- Produces: `pub(super) fn parse_columns(value: Option<&str>) -> Vec<String>` in `api/query.rs`; existing handler signatures and response DTOs remain unchanged.

- [ ] **Step 1: Write the parser compatibility test before implementation**

Create `api/query.rs`, register `mod query;`, and add `parse_columns_preserves_existing_wire_semantics` asserting:

```rust
assert_eq!(parse_columns(None), Vec::<String>::new());
assert_eq!(parse_columns(Some("a,, b,a,")), vec!["a", " b", "a"]);
```

Do not trim, sort, or deduplicate values.

- [ ] **Step 2: Run the test and confirm RED**

Run: `cd backend && cargo test --features e2e-test api::query::tests::parse_columns_preserves_existing_wire_semantics --lib`

Expected: FAIL because `parse_columns` is not yet defined.

- [ ] **Step 3: Implement the shared parser**

Implement `pub(super) fn parse_columns(value: Option<&str>) -> Vec<String>` with the existing split/filter/map semantics.

- [ ] **Step 4: Run the parser test and confirm GREEN**

Run the Step 2 command.

Expected: PASS.

- [ ] **Step 5: Replace the three duplicate parsers**

Use `query::parse_columns` in `model_dataframes.rs`, `data_resources.rs`, and `datasets.rs`. Rename compressed handler locals such as `s`, `p`, `q`, `f`, and `cols` in `datasets.rs` to `state`, `user`, `query`, `download`, and `columns`; do not change signatures or payloads.

- [ ] **Step 6: Document the route families**

Add module `//!` and handler `///` comments to uploads, upload confirmation, resources, model/draft DataFrames, datasets, attachments, workspaces, versions, and deletion previews. Explicitly preserve body-limit overrides, revision/impact tokens, cursor semantics, immutable downloads, and user scoping.

- [ ] **Step 7: Verify the slice**

Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --features e2e-test --test upload_batches --test data_resources --test model_dataframe_reads --test model_versions --test workspace_versions --test result_deletion`

Expected: all commands exit 0.

- [ ] **Step 8: Commit**

```bash
git add backend/src/api/query.rs backend/src/api/mod.rs backend/src/api/attachment.rs backend/src/api/uploads.rs backend/src/api/data_resources.rs backend/src/api/model_dataframes.rs backend/src/api/datasets.rs backend/src/api/workspaces.rs
git commit -m "refactor: document data APIs and share column parsing"
```

### Task 3: Document study, workflow, run, and derived-result APIs

**Files:**
- Modify: `backend/src/api/study_configurations.rs`
- Modify: `backend/src/api/workflows.rs`
- Modify: `backend/src/api/scripts.rs`
- Modify: `backend/src/api/retries.rs`
- Modify: `backend/src/api/runs.rs`
- Modify: `backend/src/api/analysis.rs`
- Modify: `backend/src/api/comparisons.rs`
- Modify: `backend/src/api/chart_views.rs`

**Interfaces:**
- Consumes: current study revision, workflow, retry, run deletion, dataset analysis/comparison, and chart service contracts.
- Produces: complete `//!`/`///` coverage for all remaining registered handlers; no code or DTO signature changes.

- [ ] **Step 1: Add module-level ownership documentation**

Describe fixed-button confirmation, managed execution, immutable script/run/result resources, analysis/comparison delegation, and chart-model ownership without duplicating domain algorithms.

- [ ] **Step 2: Add handler Rustdoc**

Document exact method/path and the critical revision, idempotency, cancellation, retry, tombstone, source snapshot, replay, and 404 semantics for each handler. For chart handlers, distinguish 201 creation from 200 replay and state that `ChartModel` is not arbitrary ECharts JSON.

- [ ] **Step 3: Audit all registered handlers**

Run: `cd backend && rg -n -B 4 '^pub async fn|^async fn health' src/api`

Expected: every registered handler in `api/mod.rs` is present and has a nearby `///`; manually reconcile the list to all 81 entries from `route_manifest()`.

- [ ] **Step 4: Verify public contracts and focused behavior**

Run: `cd backend && cargo fmt --all -- --check && cargo doc --no-deps --all-features && cargo test --features e2e-test --test contract_manifest --test study_configurations --test scripts --test simulation_runs --test chart_views --test result_deletion && cargo test --features e2e-test --test managed_workflows workflow_routes_are_scoped_and_return_stable_conflicts -- --exact`

Expected: all commands exit 0 and the route manifest remains 81/81 unique.

- [ ] **Step 5: Commit**

```bash
git add backend/src/api/study_configurations.rs backend/src/api/workflows.rs backend/src/api/scripts.rs backend/src/api/retries.rs backend/src/api/runs.rs backend/src/api/analysis.rs backend/src/api/comparisons.rs backend/src/api/chart_views.rs
git commit -m "docs: document workflow and result API handlers"
```

### Task 4: Split durable event projection, writing, and broadcast ownership

**Files:**
- Delete: `backend/src/application/events.rs`
- Create: `backend/src/application/events/mod.rs`
- Create: `backend/src/application/events/writer.rs`
- Create: `backend/src/application/events/bus.rs`
- Test: unit tests in `backend/src/application/events/mod.rs` and `writer.rs`

**Interfaces:**
- Consumes: `Message`, `NewMessage`, `MessageRepo`, `RepoError`, `UserId`, and Tokio broadcast.
- Produces: unchanged public exports `AppEvent`, `EventWriter`, `EventBus`, message/card types, `MESSAGE_TYPES`, `ms_to_iso`, and `now_ms`; `application/mod.rs` imports remain unchanged.

- [ ] **Step 1: Add a projection characterization test in the existing module**

Add `malformed_structured_payload_falls_back_without_losing_identity`: construct a tool message with kind `result`, invalid `payload_json`, id `42`, revision `3`, content `fallback`, and timestamp `0`; assert `AppEvent::from_message` returns `Assistant`, preserves id/revision/content, and returns event ID `42:3`.

- [ ] **Step 2: Run event tests before moving code**

Run: `cd backend && cargo test --features e2e-test application::events::tests --lib`

Expected: PASS, establishing the pre-refactor behavior.

- [ ] **Step 3: Create the module directory and move the event model/projection**

Move constants, time helpers, wire/card types, private durable payload types, `AppEvent`, `ResultOutputRef::from_dataset`, `AppEvent::from_message`, `message_id`, `revision`, and `event_id` into `events/mod.rs`. Keep serde attributes and public paths byte-for-byte compatible.

- [ ] **Step 4: Move the durable writer**

Move `EventWriter`, its public methods, operation-card upsert behavior, and `normalize_tool_args` into `events/writer.rs`; re-export `EventWriter` from `mod.rs`. Keep write-before-publish ordering and changed-only operation broadcasts.

- [ ] **Step 5: Move the session broadcast bus**

Move `EventBus` and its `new`, `subscribe`, `publish`, and `unregister` methods into `events/bus.rs`; re-export it from `mod.rs`. Keep channel capacity 256 and unregister/drop behavior.

- [ ] **Step 6: Re-home tests and verify compatibility**

Keep projection/wire tests in `events/mod.rs`; move `tool_call_args_null_normalized` beside writer code. Run: `cd backend && cargo fmt --all -- --check && cargo test --features e2e-test application::events --lib && cargo test --features e2e-test --test contract_manifest --test sse_recovery && cargo test --features e2e-test --test failure_atomicity injected_event_commit_failure_publishes_nothing_and_keeps_history_readable -- --exact`

Expected: all commands exit 0.

- [ ] **Step 7: Commit**

```bash
git add backend/src/application/events.rs backend/src/application/events
git commit -m "refactor: separate durable event responsibilities"
```

### Task 5: Extract managed workflow result-contract validation

**Files:**
- Create: `backend/src/application/workflow_result_contract.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/application/workflow_worker.rs`

**Interfaces:**
- Consumes: `ParquetMetadata`, `SchemaFieldContract`, `DataTypeContract`, `ParquetInspector`, and a result file `Path`.
- Produces: `pub(super) fn validate_schema(metadata: &ParquetMetadata, expected: &[SchemaFieldContract]) -> Result<(), String>` and `pub(super) fn validate_non_null_values(path: &Path, expected: &[SchemaFieldContract]) -> Result<(), String>`.

- [ ] **Step 1: Add null-value characterization coverage before extraction**

In the current `workflow_worker` test module, add `rejects_nulls_in_non_nullable_result_columns`. Write a temporary Snappy Parquet file with column `value = [Some(1.0), None]`, declare `value` as non-nullable `f64`, and assert the validator returns an error containing `结果列 value 含 null`.

- [ ] **Step 2: Run the result-contract tests before moving code**

Run: `cd backend && cargo test --features e2e-test application::workflow_worker::tests --lib`

Expected: all five result-contract tests PASS.

- [ ] **Step 3: Extract the validation module**

Move `validate_schema`, `validate_non_null_values`, `canonical_arrow`, `dtype_matches`, and `title_case` to the new private module. Register `mod workflow_result_contract;` in `application/mod.rs`; import only the two public-to-parent validators in `workflow_worker.rs`.

- [ ] **Step 4: Move focused tests with their owner**

Move the dtype, schema/unit, and null-value tests to `workflow_result_contract.rs`. Do not move workflow orchestration tests or output cleanup ownership.

- [ ] **Step 5: Verify managed execution behavior**

Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --features e2e-test application::workflow_result_contract --lib && cargo test --features e2e-test --test script_repair --test workflow_retries && cargo test --features e2e-test --test managed_workflows scripted_author_sequences_generation_and_repair -- --exact`

Expected: all commands exit 0.

- [ ] **Step 6: Commit**

```bash
git add backend/src/application/mod.rs backend/src/application/workflow_worker.rs backend/src/application/workflow_result_contract.rs
git commit -m "refactor: isolate workflow result contract validation"
```

### Task 6: Centralize connection-local SQLite insert IDs

**Files:**
- Modify: `backend/src/db/repositories/mod.rs`
- Modify: `backend/src/db/repositories/analysis.rs`
- Modify: `backend/src/db/repositories/comparison.rs`
- Modify: `backend/src/db/repositories/managed_workflow.rs`
- Modify: `backend/src/db/repositories/message.rs`
- Modify: `backend/src/db/repositories/run.rs`
- Modify: `backend/src/db/repositories/session.rs`
- Modify: `backend/src/db/repositories/study_configuration.rs`
- Modify: `backend/src/db/repositories/upload_confirmation_transactions.rs`
- Modify: `backend/src/db/repositories/version.rs`

**Interfaces:**
- Consumes: a mutable Diesel `SqliteConnection` immediately after an insert on that same connection.
- Produces: `pub(super) fn last_insert_rowid(conn: &mut SqliteConnection) -> RepoResult<i32>` in `repositories/mod.rs`.

- [ ] **Step 1: Write the repository helper test before implementation**

Add `last_insert_rowid_is_connection_local_and_monotonic` in `repositories/mod.rs`: create an in-memory SQLite table, insert one row and assert `1`, insert another and assert `2` through the new helper.

- [ ] **Step 2: Run the test and confirm RED**

Run: `cd backend && cargo test --features e2e-test db::repositories::tests::last_insert_rowid_is_connection_local_and_monotonic --lib`

Expected: FAIL because the helper is not defined.

- [ ] **Step 3: Implement the helper**

Implement the exact signature with `diesel::select(diesel::dsl::sql::<Integer>("last_insert_rowid()"))` and map the Diesel error through `RepoError`.

- [ ] **Step 4: Run the helper test and confirm GREEN**

Run the Step 2 command.

Expected: PASS.

- [ ] **Step 5: Replace all repository-local forms**

Replace the three private `last_id` functions and every inline `last_insert_rowid()` expression listed in the Files block with `last_insert_rowid(conn)`. Keep the call immediately after the same insert and before any other statement on that connection.

- [ ] **Step 6: Verify repository behavior**

Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --features e2e-test --test auth_scope --test model_versions --test simulation_runs --test study_configurations --test upload_batches && cargo test --features e2e-test --test managed_workflows confirmation_is_idempotent_and_creates_one_provisional_run -- --exact`

Expected: all commands exit 0.

- [ ] **Step 7: Commit**

```bash
git add backend/src/db/repositories/mod.rs backend/src/db/repositories/analysis.rs backend/src/db/repositories/comparison.rs backend/src/db/repositories/managed_workflow.rs backend/src/db/repositories/message.rs backend/src/db/repositories/run.rs backend/src/db/repositories/session.rs backend/src/db/repositories/study_configuration.rs backend/src/db/repositories/upload_confirmation_transactions.rs backend/src/db/repositories/version.rs
git commit -m "refactor: centralize SQLite insert id lookup"
```

### Task 7: Scope test-only event failure injection by session

**Files:**
- Modify: `backend/src/testing/faults.rs`
- Modify: `backend/src/application/events/writer.rs`
- Modify: `backend/tests/failure_atomicity.rs`
- Modify: `backend/tests/managed_workflows.rs`

**Interfaces:**
- Consumes: existing global `FaultInjector::fail_next(FaultPoint)` used by E2E control routes.
- Produces: `pub fn fail_next_for_session(&self, point: FaultPoint, session_id: i32)` and `pub(crate) fn take(&self, point: FaultPoint, session_id: i32) -> bool`; global E2E injection remains available.

- [ ] **Step 1: Add failing isolation tests**

In `testing/faults.rs`, add:

- `scoped_failure_is_not_consumed_by_another_session`: arm session 10, assert session 11 is false, session 10 is true, and session 10 is false on the second take.
- `global_failure_remains_available_to_any_session`: arm the existing global fault and assert an arbitrary session consumes it once.

- [ ] **Step 2: Run tests and confirm RED**

Run: `cd backend && cargo test --features e2e-test testing::faults::tests --lib`

Expected: FAIL because session-scoped APIs do not exist and `take` has no session argument.

- [ ] **Step 3: Implement scoped counters without changing global control behavior**

Keep the existing global atomic counter for `fail_next`. Add a mutex-protected per-session counter map for `fail_next_for_session`. Make `take(point, session_id)` consume a matching scoped count first, then fall back to the global count.

- [ ] **Step 4: Pass EventWriter's session into fault consumption**

Update both fault checks in `events/writer.rs` to call `take(FaultPoint::EventCommit, self.session_id)`.

- [ ] **Step 5: Scope in-process integration faults**

Change `failure_atomicity.rs` and the managed-workflow replay test to call `fail_next_for_session(..., session.id)`. Do not change `testing/control.rs`; its `/__e2e/faults` route continues to use the global method.

- [ ] **Step 6: Verify parallel execution**

Run: `cd backend && cargo test --features e2e-test testing::faults::tests --lib && cargo test --features e2e-test --test failure_atomicity --test managed_workflows`

Expected: all commands exit 0 under default parallel test execution; the two confirmation-card tests no longer exchange 200/500 results.

- [ ] **Step 7: Commit**

```bash
git add backend/src/testing/faults.rs backend/src/application/events/writer.rs backend/tests/failure_atomicity.rs backend/tests/managed_workflows.rs
git commit -m "test: scope event commit faults by session"
```

### Task 8: Document runtime, Agent, adapter, and engine-resource boundaries

**Files:**
- Modify: `backend/src/adapters/mod.rs`
- Modify: `backend/src/adapters/contract.rs`
- Modify: `backend/src/adapters/import.rs`
- Modify: `backend/src/adapters/stub.rs`
- Modify: `backend/src/adapters/process/mod.rs`
- Modify: `backend/src/adapters/process/capture.rs`
- Modify: `backend/src/adapters/process/command.rs`
- Modify: `backend/src/adapters/process/control.rs`
- Modify: `backend/src/adapters/process/execution.rs`
- Modify: `backend/src/adapters/process/import.rs`
- Modify: `backend/src/adapters/process/monitor.rs`
- Modify: `backend/src/adapters/process/output.rs`
- Modify: `backend/src/adapters/process/result_scan.rs`
- Modify: `backend/src/adapters/process/task_file.rs`
- Modify: `backend/src/agent/mod.rs`
- Modify: `backend/src/agent/factory.rs`
- Modify: `backend/src/agent/failure_analyzer.rs`
- Modify: `backend/src/agent/knowledge.rs`
- Modify: `backend/src/agent/prompt.rs`
- Modify: `backend/src/agent/script_author.rs`
- Modify: `backend/src/engine_resources/mod.rs`
- Modify: `backend/src/engine_resources/examples.rs`
- Modify: `backend/src/engine_resources/facades.rs`
- Modify: `backend/src/engine_resources/knowledge.rs`
- Modify: `backend/src/engine_resources/loader.rs`
- Modify: `backend/src/engine_resources/projections.rs`
- Modify: `backend/src/engine_resources/publisher.rs`
- Modify: `backend/src/engine_resources/snapshot.rs`
- Modify: `backend/src/engine_resources/tree.rs`

**Interfaces:**
- Consumes: existing process, capability, Agent knowledge/factory, resource tree/loader/publisher, and snapshot APIs.
- Produces: module and critical facade Rustdoc only, plus a test-only availability guard on `EngineResourceSnapshot::minimal`; no runtime signature changes.

- [ ] **Step 1: Capture the production warning baseline**

Run: `cd backend && cargo check --release`

Expected before the change: output includes `associated function minimal is never used`.

- [ ] **Step 2: Restrict the minimal snapshot constructor to test builds**

Add `#[cfg(any(test, feature = "e2e-test"))]` to `EngineResourceSnapshot::minimal`. Keep its signature and all test callers unchanged.

- [ ] **Step 3: Add adapter/process documentation**

Document the unique process boundary, capability probe, structured specs, bounded dual-stream capture, process-group cancellation, runtime/final scans, import/projector wrappers, and RAII task-file cleanup in their owning modules and facade items.

- [ ] **Step 4: Add Agent boundary documentation**

Document validated embedded knowledge, manifest-controlled tool exposure, prompt assembly, per-request factory construction, isolated ScriptAuthor, and isolated FailureAnalyzer. Do not edit prompt/skill resources or hashes.

- [ ] **Step 5: Add engine-resource documentation**

Document safe tree traversal, catalog/facade/projection/example loading, byte-derived snapshot identity, immutable publication/rebase, and restart semantics in `engine_resources` modules and public types.

- [ ] **Step 6: Verify production and test configurations**

Run: `cd backend && cargo fmt --all -- --check && cargo check --release && cargo clippy --all-targets --all-features -- -D warnings && cargo doc --no-deps --all-features && cargo test --features e2e-test --test process_engine_v4 --test engine_resources --test agent_knowledge --test tool_manifest`

Expected: all commands exit 0; `cargo check --release` no longer emits the unused `minimal` warning.

- [ ] **Step 7: Commit**

```bash
git add backend/src/adapters backend/src/agent backend/src/engine_resources
git commit -m "docs: clarify engine and agent runtime boundaries"
```

### Task 9: Document remaining backend ownership boundaries

**Files:**
- Modify: `backend/src/lib.rs`
- Modify: `backend/src/main.rs`
- Modify: `backend/src/config.rs`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/application/analysis.rs`
- Modify: `backend/src/application/comparisons.rs`
- Modify: `backend/src/application/dataframe_reader.rs`
- Modify: `backend/src/application/managed_workflows.rs`
- Modify: `backend/src/application/result_sources.rs`
- Modify: `backend/src/application/runs.rs`
- Modify: `backend/src/application/study_configurations.rs`
- Modify: `backend/src/application/upload_confirmation.rs`
- Modify: `backend/src/application/uploads.rs`
- Modify: `backend/src/auth/mod.rs`
- Modify: `backend/src/auth/seed.rs`
- Modify: `backend/src/auth/session.rs`
- Modify: `backend/src/catalog/mod.rs`
- Modify: `backend/src/catalog/inference.rs`
- Modify: `backend/src/catalog/schema.rs`
- Modify: `backend/src/db/mod.rs`
- Modify: `backend/src/db/repositories/mod.rs`
- Modify: `backend/src/db/repositories/artifact.rs`
- Modify: `backend/src/db/repositories/artifact_refs.rs`
- Modify: `backend/src/db/repositories/managed_workflow.rs`
- Modify: `backend/src/db/repositories/run.rs`
- Modify: `backend/src/db/repositories/session.rs`
- Modify: `backend/src/db/repositories/upload.rs`
- Modify: `backend/src/db/repositories/version.rs`
- Modify: `backend/src/ingestion/mod.rs`
- Modify: `backend/src/ingestion/archive.rs`
- Modify: `backend/src/ingestion/bundle.rs`
- Modify: `backend/src/ingestion/rss.rs`
- Modify: `backend/src/ingestion/tabular.rs`
- Modify: `backend/src/storage/mod.rs`
- Modify: `backend/src/storage/blob.rs`
- Modify: `backend/src/storage/parquet.rs`
- Modify: `backend/src/storage/staging.rs`
- Modify: `backend/src/tools/mod.rs`
- Modify: `backend/src/tools/support.rs`

**Interfaces:**
- Consumes: the ownership table in `docs/architecture.md` and actual facade/repository implementations.
- Produces: discoverable module/facade/safety-boundary Rustdoc; no public item or behavior changes.

- [ ] **Step 1: Document the composition root and application facades**

Explain startup preflight order, named dependency assembly, and the distinct ownership of upload, study configuration, workflow, ordinary run, result source, analysis, and comparison state machines. Document only non-obvious public facade invariants rather than every DTO field.

- [ ] **Step 2: Document authentication, catalog, ingestion, and storage boundaries**

Cover cookie/session scope, immutable user seed, catalog/schema inference, archive/bundle safety, RSS/tabular inspection, staging/CAS publication, and Parquet inspection. Keep validation and capacity values in their existing constants/config rather than duplicating them in prose.

- [ ] **Step 3: Document repository ownership**

Document `spawn_blocking` SQLite boundaries, shared errors/transitions, user scope, artifact reference accounting, immediate transactions, idempotency, tombstones, and domain-specific state ownership in the listed repository modules.

- [ ] **Step 4: Document tool registration ownership**

Document `ToolServices`, the private implementation catalog, manifest validation, skill gates, stable-ID arguments, and shared support helpers. Do not add comments to all 23 thin tool wrappers unless a wrapper has a non-obvious authorization or side-effect boundary.

- [ ] **Step 5: Verify documentation compiles cleanly**

Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo doc --no-deps --all-features`

Expected: all commands exit 0 without broken intra-doc links or new warnings.

- [ ] **Step 6: Commit**

```bash
git add backend/src/lib.rs backend/src/main.rs backend/src/config.rs backend/src/contract_manifest.rs backend/src/application/mod.rs backend/src/application/analysis.rs backend/src/application/comparisons.rs backend/src/application/dataframe_reader.rs backend/src/application/managed_workflows.rs backend/src/application/result_sources.rs backend/src/application/runs.rs backend/src/application/study_configurations.rs backend/src/application/upload_confirmation.rs backend/src/application/uploads.rs backend/src/auth/mod.rs backend/src/auth/seed.rs backend/src/auth/session.rs backend/src/catalog/mod.rs backend/src/catalog/inference.rs backend/src/catalog/schema.rs backend/src/db/mod.rs backend/src/db/repositories/mod.rs backend/src/db/repositories/artifact.rs backend/src/db/repositories/artifact_refs.rs backend/src/db/repositories/managed_workflow.rs backend/src/db/repositories/run.rs backend/src/db/repositories/session.rs backend/src/db/repositories/upload.rs backend/src/db/repositories/version.rs backend/src/ingestion/mod.rs backend/src/ingestion/archive.rs backend/src/ingestion/bundle.rs backend/src/ingestion/rss.rs backend/src/ingestion/tabular.rs backend/src/storage/mod.rs backend/src/storage/blob.rs backend/src/storage/parquet.rs backend/src/storage/staging.rs backend/src/tools/mod.rs backend/src/tools/support.rs
git commit -m "docs: document backend ownership boundaries"
```

### Task 10: Reconcile authoritative documentation and run final gates

**Files:**
- Modify: `docs/api-design.md`
- Modify: `docs/architecture.md`
- Modify: `docs/development-status.md`
- Modify if and only if touched boundaries require it: `docs/db-design.md`
- Modify if and only if touched boundaries require it: `docs/integration-testing-design.md`

**Interfaces:**
- Consumes: completed Tasks 1-9 and their actual verification output.
- Produces: current source navigation, ownership map, intentional-large-file notes, refactor summary, and truthful verification evidence.

- [ ] **Step 1: Reconcile the HTTP design document**

Keep the 81-route semantics unchanged. Add source navigation to `api/mod.rs`, route-family modules, `api/error.rs`, frontend DTOs, and application services; state that route registration is authoritative for method/path while handler Rustdoc explains local execution semantics.

- [ ] **Step 2: Reconcile architecture ownership**

Record `api/query.rs`, the `events/` split, `workflow_result_contract.rs`, shared `last_insert_rowid`, scoped test faults, and the retained ownership of large schema/event/chart/workflow files. Do not describe a refactor that is absent from the final diff.

- [ ] **Step 3: Run deterministic backend and coverage gates**

Run:

```bash
cd backend
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo doc --no-deps --all-features
cargo test --features e2e-test
cd ..
./scripts/check-functional-coverage --require-pr
```

Expected: every command exits 0. The backend suite must pass with default parallelism; do not substitute `--test-threads=1`.

- [ ] **Step 4: Audit hard boundaries mechanically**

Run searches confirming no process spawn escaped `backend/src/adapters`, no public route/DTO manifest changed, no migration changed, and the only pre-existing Agent resource differences remain the two excluded hash files. Compare `api::route_manifest()`/contract tests to the 81-route baseline.

- [ ] **Step 5: Update development status with observed evidence**

Record the exact implemented ownership changes and only the verification commands that actually passed. If an external eig-rs or environment gate fails, name it separately and do not label the candidate fully green.

- [ ] **Step 6: Re-run formatting and documentation checks after the status edit**

Run: `cd backend && cargo fmt --all -- --check && cargo doc --no-deps --all-features && cd .. && git diff --check`

Expected: all commands exit 0.

- [ ] **Step 7: Commit documentation**

```bash
git add docs/api-design.md docs/architecture.md docs/development-status.md
git add docs/db-design.md docs/integration-testing-design.md  # only if actually changed
git commit -m "docs: record backend maintainability baseline"
```

- [ ] **Step 8: Review the final branch**

Run `git status --short`, `git diff a733b14..HEAD --stat`, and inspect every commit. Confirm the two pre-existing Agent hash files remain unstaged, no frontend/migration/feature-catalog file changed, and every task has its own verification evidence.

