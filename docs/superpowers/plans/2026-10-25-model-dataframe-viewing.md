# Model DataFrame Viewing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users inspect active, draft, and historical model DataFrames, and let the Agent read bounded model rows before producing a safe draft mutation.

**Architecture:** Add a model-specific read service and six read-only routes over existing version/draft Parquet artifacts. Extract result/model shared pagination and Arrow conversion into an internal reader, then add a read-only React workbench and a controlled `read_model_dataframe` Agent tool without turning model tables into result datasets.

**Tech Stack:** Rust, Axum, Diesel/SQLite, Arrow/Parquet, rig tools, React, TypeScript, Vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-25-model-dataframe-viewing-design.md`

## Global Constraints

- Work directly on `master`; preserve and exclude unrelated user changes from every commit.
- All repository reads accept `UserId`; cross-user and missing resources both return 404.
- Model and result bodies remain Snappy Parquet artifacts; no database migration is added.
- Browsers and LLM tools receive stable IDs and controlled query values, never paths, SQL, scripts, or artifact locations.
- Existing `/api/datasets/{id}/rows` and `DatasetPage.datasetId` remain unchanged.
- Draft metadata, rows, cursors, and downloads bind `draftDataframeId + draftId + expectedDraftRevision`; stale reads return `draft_revision_conflict`.
- Model table UI is read-only. Model changes continue through `read_model_dataframe` followed by `modify_model`; users still publish with “生成版本”.
- Adding six routes, one tool, and one public workbench requires matching behavior tests, feature catalog contracts, current design docs, and an Agent bundle version bump from `v3.0.0` to `v3.1.0`.

## Review Focus

- A draft changes between the first and second page: preserve loaded rows, reject the old cursor with 409, and never merge revisions (Task 2 API test, Task 5 UI test).
- A user guesses another user's model or draft DataFrame ID: metadata, rows, and download all return the same 404 as a missing ID (Task 2 API test, Task 6 Playwright test).
- A zero-row or complex-list model table is opened: metadata and empty/list-valued rows remain valid, while complex values are rejected only when used in Agent `equals` (Task 1 reader test, Task 3 tool test).
- A query returns more matches than `limit`: the tool scans the complete table, reports exact `matchCount`, sets `truncated`, and never treats the first row as unique (Task 3 tool test).
- A session switch or late request completes after another model table is opened: stale responses do not replace the current workbench and no prior-session rows remain visible (Task 4 hook test, Task 5 UI test).

---

### Task 1: Extract the shared Parquet DataFrame reader

**Files:**
- Create: `backend/src/application/dataframe_reader.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/application/datasets.rs`
- Test: `backend/src/application/dataframe_reader.rs`
- Test: `backend/src/application/datasets.rs`

**Interfaces:**
- Consumes: `ArtifactService`, `CurrentUser`, and `ParquetInspector::projection`.
- Produces: `DataFrameReadSource`, `DataFramePageCore`, `DataFrameDownload`, `DataFrameReadError`, and `ParquetDataFrameReader::{page,download}` for Tasks 2 and 3.

- [ ] **Step 1: Write failing reader tests**

  Add tests named `cursor_is_bound_to_resource_kind_identity_hash_and_revision`,
  `projection_rejects_unknown_and_duplicate_columns`, and
  `zero_rows_and_nested_list_values_have_stable_json_pages`. Assert a cursor issued for
  `dataset:1` is invalid for `model-dataframe:1` and another draft revision; `limit` accepts only `1..=500`; empty tables
  preserve registered columns; List/LargeList/ListView/fixed-list values use the same recursive JSON encoding already covered
  by `large_float_lists_are_returned_as_json_arrays`.

- [ ] **Step 2: Run the focused tests and verify red**

  Run: `cd backend && cargo test dataframe_reader --lib`

  Expected: FAIL because `application::dataframe_reader` and the resource-bound cursor do not exist.

- [ ] **Step 3: Introduce the internal reader interfaces**

  In `dataframe_reader.rs`, define:

  ```rust
  pub(crate) struct DataFrameReadSource {
      pub resource_key: String,
      pub artifact_id: i32,
      pub schema_json: String,
      pub row_count: i64,
      pub content_hash: String,
  }
  pub(crate) struct DataFramePageCore {
      pub columns: Vec<String>,
      pub rows: Vec<Vec<serde_json::Value>>,
      pub next_cursor: Option<String>,
      pub diagnostics: BTreeMap<String, u64>,
  }
  pub(crate) struct DataFrameDownload {
      pub path: PathBuf,
      pub file_name: String,
      pub media_type: &'static str,
  }
  pub(crate) struct ParquetDataFrameReader { /* ArtifactService */ }
  ```

  Add `page(&CurrentUser, &DataFrameReadSource, Option<usize>, Option<&str>, &[String])` and
  `download(&CurrentUser, &DataFrameReadSource, String)`. Bind cursor payloads to `resource_key + content_hash + offset`.
  Move projection validation, safe filename handling, artifact ownership/path resolution, cursor code, recursive Arrow value
  conversion, and diagnostics out of `DatasetService`.

- [ ] **Step 4: Adapt `DatasetService` without changing its public DTO**

  Compose `ParquetDataFrameReader` inside `DatasetService`. Map shared reader errors back to existing `DatasetError`, build the
  source key as `dataset:{id}`, and translate `DataFramePageCore` into the existing `DatasetPage { dataset_id, ... }`.
  Leave metadata/views/lineage/delete behavior in `datasets.rs`.

- [ ] **Step 5: Verify focused and result regression tests**

  Run: `cd backend && cargo test dataframe_reader --lib && cargo test --test result_deletion`

  Expected: PASS; result row responses still serialize `datasetId` and complex list values remain readable.

- [ ] **Step 6: Commit the refactor**

  ```bash
  git add backend/src/application/dataframe_reader.rs backend/src/application/mod.rs backend/src/application/datasets.rs
  git commit -m "refactor: share parquet dataframe reading"
  ```

### Task 2: Add user-scoped model DataFrame metadata, rows, and downloads

**Files:**
- Create: `backend/src/application/model_dataframes.rs`
- Create: `backend/src/api/model_dataframes.rs`
- Create: `backend/tests/model_dataframe_reads.rs`
- Modify: `backend/src/db/repositories/version.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/api/mod.rs`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/contract_manifest.rs`

**Interfaces:**
- Consumes: Task 1 `ParquetDataFrameReader` and existing `VersionRepo`/artifact ownership.
- Produces: `ModelDataFrameReadService`, six HTTP routes, `ModelDataFrameMetadata`, `ModelDataFramePage`, and repository read rows used by Task 3.

- [ ] **Step 1: Write failing API behavior tests**

  In `model_dataframe_reads.rs`, create version and draft artifacts through public import/publish flows. Add tests:

  - `version_dataframe_metadata_rows_projection_cursor_and_download_are_read_only`;
  - `historical_version_dataframe_is_readable_without_activation_and_deleted_version_is_not`;
  - `draft_reads_require_exact_owner_id_and_revision_across_metadata_rows_and_download`;
  - `model_dataframe_ids_are_private_and_missing_or_foreign_ids_are_indistinguishable`.

  Assert metadata shape, `ModelDataFramePage.resourceId`, a second page, selected columns, Parquet MIME/safe filename, no new
  database rows/references after reads, stale draft 409 with `draft_revision_conflict`, published draft 404, and all six
  foreign-resource requests 404.

- [ ] **Step 2: Run the new integration test and verify red**

  Run: `cd backend && cargo test --test model_dataframe_reads`

  Expected: FAIL because the model DataFrame routes are absent.

- [ ] **Step 3: Add scoped repository reads**

  Define `VersionDataFrameReadRow` and `DraftDataFrameReadRow` in `version.rs` and add:

  ```rust
  pub async fn find_version_dataframe(
      &self, user: &UserId, dataframe_id: i32
  ) -> Result<VersionDataFrameReadRow, VersionRepoError>;

  pub async fn find_draft_dataframe(
      &self, user: &UserId, dataframe_id: i32, draft_id: i32, expected_revision: i64
  ) -> Result<DraftDataFrameReadRow, VersionRepoError>;
  ```

  Version lookup joins through version/workspace/session and excludes deleted versions. Draft lookup first scopes the DataFrame
  to the user, then returns `DraftConflict` only for that user's stale draft/revision; missing and foreign resources remain
  `RepoError::NotFound`.

- [ ] **Step 4: Implement the model read application service**

  Define `ModelDataFrameScope::{ModelVersion,Draft}`, `ModelDataFrameMetadata`, `ModelDataFramePage`, `DraftReadExpectation`,
  `ModelDataFrameReadError`, and `ModelDataFrameReadService` in `model_dataframes.rs`. Provide separate
  `version_metadata/page/download` and `draft_metadata/page/download` methods. Use resource keys
  `model-dataframe:{id}` and `draft-dataframe:{id}:draft:{draft_id}:revision:{revision}` and Task 1's reader. Map errors to the
  spec's `not_found`, `draft_revision_conflict`, `invalid_page`, and `dataframe_read_failed` contracts.

- [ ] **Step 5: Register handlers and routes**

  Add typed metadata/rows/download handlers in `api/model_dataframes.rs`; share one parser for `limit/cursor/columns`, and one
  `DraftQuery { draft_id, expected_draft_revision }`. Wire `ModelDataFrameReadService` into `AppState` and add exactly these
  routes:

  ```text
  GET /api/model-dataframes/{id}
  GET /api/model-dataframes/{id}/rows
  GET /api/model-dataframes/{id}/download
  GET /api/draft-dataframes/{id}
  GET /api/draft-dataframes/{id}/rows
  GET /api/draft-dataframes/{id}/download
  ```

  Add the two new HTTP error codes to `STABLE_ERROR_CODES`; change the route manifest assertion from 42 to 48 and assert all six
  paths.

- [ ] **Step 6: Run backend API tests**

  Run: `cd backend && cargo test --test model_dataframe_reads && cargo test --test contract_manifest && cargo test --test auth_scope`

  Expected: PASS with 48 unique routes and unchanged result-dataset behavior.

- [ ] **Step 7: Commit the backend read API**

  ```bash
  git add backend/src/application/model_dataframes.rs backend/src/api/model_dataframes.rs backend/src/db/repositories/version.rs backend/src/application/mod.rs backend/src/api/mod.rs backend/src/contract_manifest.rs backend/tests/model_dataframe_reads.rs backend/tests/contract_manifest.rs
  git commit -m "feat: expose read-only model dataframes"
  ```

### Task 3: Close the Agent read-before-modify loop

**Files:**
- Create: `backend/src/tools/read_model_dataframe.rs`
- Modify: `backend/src/application/model_dataframes.rs`
- Modify: `backend/src/tools/mod.rs`
- Modify: `backend/resources/agent/v3/manifest.json`
- Modify: `backend/resources/agent/v3/skill-index.json`
- Modify: `backend/resources/agent/v3/system.md`
- Modify: `backend/resources/agent/v3/skills/model-intake.md`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/config.rs`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/tool_manifest.rs`
- Modify: `backend/tests/workspace_tools.rs`
- Modify: `backend/tests/agent_knowledge.rs`
- Modify: `backend/tests/app_bootstrap.rs`

**Interfaces:**
- Consumes: Task 2 `ModelDataFrameReadService` and workspace-scoped version/draft lookups.
- Produces: rig tool `read_model_dataframe`, `ModelDataFrameQuery`, and `ModelDataFrameQueryResult` with exact `matchCount`.

- [ ] **Step 1: Write failing tool behavior tests**

  Add tests named:

  - `read_model_dataframe_reads_active_and_exact_draft_without_paths`;
  - `read_model_dataframe_reports_exact_match_count_beyond_return_limit`;
  - `read_model_dataframe_rejects_stale_draft_complex_predicates_and_oversized_output`;
  - `agent_reads_unique_old_value_before_modify_model_advances_only_the_draft`.

  Cover an empty equals map preview, string/bool/signed/unsigned/finite Float32/Float64 equality, lossless numeric range checks,
  unsupported null/list/object predicates, at most 8 equality columns, at most 32 output columns, limit `1..=100`, 64 KiB
  serialized output, complete scan with `matchCount > limit`, and unchanged active content after modification.

- [ ] **Step 2: Run the tool tests and verify red**

  Run: `cd backend && cargo test --test workspace_tools read_model_dataframe`

  Expected: FAIL because the query service and tool are absent.

- [ ] **Step 3: Add the bounded query interface**

  In `model_dataframes.rs`, define:

  ```rust
  pub enum ModelReadScope { Active, Draft }
  pub struct ModelDataFrameQuery {
      pub scope: ModelReadScope,
      pub table_name: String,
      pub columns: Vec<String>,
      pub equals: BTreeMap<String, Value>,
      pub limit: usize,
      pub expected_draft: Option<DraftRef>,
  }
  pub struct ModelDataFrameQueryResult {
      pub columns: Vec<String>,
      pub rows: Vec<Vec<Value>>,
      pub match_count: u64,
      pub truncated: bool,
      /* actual scope/version/draft/revision/content hash */
  }
  ```

  Implement `query_workspace(&CurrentUser, session_id: i32, ModelDataFrameQuery)`. Stream every batch to compute exact
  `match_count`, retain only `limit` rows, load filter/output column union, enforce the approved equality conversions, and fail
  before serialization exceeds 64 KiB. Add `ModelDataFrameReadError` variants/codes
  `model_scope_unavailable`, `invalid_model_query`, and `model_query_output_too_large`, and add all three to
  `STABLE_ERROR_CODES` alongside Task 2's `dataframe_read_failed`.

- [ ] **Step 4: Register the rig tool and service dependency**

  Implement `read_model_dataframe::Args` with camelCase `scope/tableName/columns/equals/limit/expectedDraft`. Mark the tool
  read-only, bind it to the current `ToolServices.session_id`, map only stable errors, and add `model_dataframes` to
  `ToolServices`. Register the tool immediately after `inspect_workspace`; update the registry tests from 16 to 17 unique tools.

- [ ] **Step 5: Update and re-hash Agent bundle v3.1.0**

  Set `agentBundleVersion` and `skill-index.bundleVersion` to `v3.1.0`; update active code/config/tests that load the embedded
  bundle. Add the tool to `allowedTools` and the `model-intake` skill gate. Update system/skill instructions to require
  inspect → read → unique match → modify, plus recovery rules for stale revision, invalid query, oversized output, and storage
  failure. Recompute the changed `model-intake.md` SHA-256 stored in `skill-index.json`, then recompute
  `manifest.systemSha256` and `manifest.skillIndexSha256` from the final exact file bytes with `shasum -a 256`.

- [ ] **Step 6: Verify tool registry, bundle, and modification behavior**

  Run: `cd backend && cargo test --test workspace_tools && cargo test --test tool_manifest && cargo test --test agent_knowledge && cargo test --test app_bootstrap`

  Expected: PASS with 17 tools, bundle `v3.1.0`, valid resource hashes, and no path/artifact ID in tool output.

- [ ] **Step 7: Commit the Agent read tool**

  ```bash
  git add backend/src/tools/read_model_dataframe.rs backend/src/application/model_dataframes.rs backend/src/tools/mod.rs backend/resources/agent/v3/manifest.json backend/resources/agent/v3/skill-index.json backend/resources/agent/v3/system.md backend/resources/agent/v3/skills/model-intake.md backend/src/application/mod.rs backend/src/config.rs backend/src/contract_manifest.rs backend/tests/tool_manifest.rs backend/tests/workspace_tools.rs backend/tests/agent_knowledge.rs backend/tests/app_bootstrap.rs
  git commit -m "feat: let agent inspect model rows safely"
  ```

### Task 4: Add frontend model DataFrame contracts and race-safe loading

**Files:**
- Create: `frontend/src/types/modelDataframe.ts`
- Create: `frontend/src/hooks/useModelDataFrame.ts`
- Create: `frontend/src/hooks/useModelDataFrame.test.ts`
- Modify: `frontend/src/types/index.ts`
- Modify: `frontend/src/types/contracts.test.ts`
- Modify: `frontend/src/api/client.ts`
- Modify: `frontend/src/api/client.test.ts`
- Modify: `frontend/src/hooks/useWorkspace.ts`

**Interfaces:**
- Consumes: Task 2 routes and JSON contracts.
- Produces: `OpenDataFrame`, `ModelDataFrameMetadata`, `ModelDataFramePage`, API helpers, and `useModelDataFrame` for Task 5.

- [ ] **Step 1: Write failing type/client/hook tests**

  Assert exact version and draft URLs/query names, model page `resourceId`, preserved result `datasetId`, merged model pages,
  reset on source/projection changes, single in-flight load-more, and generation guards that ignore an earlier table or prior
  session response. Assert `draft_revision_conflict` exposes a conflict state while retaining already loaded rows.

- [ ] **Step 2: Run focused Vitest and verify red**

  Run: `cd frontend && npm test -- --run src/api/client.test.ts src/hooks/useModelDataFrame.test.ts src/types/contracts.test.ts`

  Expected: FAIL because model DataFrame contracts and helpers do not exist.

- [ ] **Step 3: Define model-specific DTOs and API helpers**

  Add the approved discriminated `OpenDataFrame` union and model metadata/page types. Implement
  `getModelDataFrame`, `getModelDataFrameRows`, and `downloadModelDataFrame` so version requests use only the DataFrame ID and
  draft requests always include `draftId` plus `expectedDraftRevision`. Do not reuse `getDataset*` or expose delete/view APIs.

- [ ] **Step 4: Implement the race-safe hook**

  `useModelDataFrame(source, selectedColumns)` loads metadata plus the first 100 rows, merges subsequent pages, preserves rows
  on later errors, and returns `{metadata,page,loading,error,conflict,loadMore,download,reload}`. Increment a generation for every
  source/projection change and ignore late responses. Update `useWorkspace.reload(id)` to return the fetched `Workspace` after
  dispatch so Task 5 can resolve a new draft table by name after conflict.

- [ ] **Step 5: Run frontend contract tests**

  Run: `cd frontend && npm test -- --run src/api/client.test.ts src/hooks/useModelDataFrame.test.ts src/types/contracts.test.ts`

  Expected: PASS with no change to result-dataset contracts.

- [ ] **Step 6: Commit frontend data access**

  ```bash
  git add frontend/src/types/modelDataframe.ts frontend/src/types/index.ts frontend/src/types/contracts.test.ts frontend/src/api/client.ts frontend/src/api/client.test.ts frontend/src/hooks/useModelDataFrame.ts frontend/src/hooks/useModelDataFrame.test.ts frontend/src/hooks/useWorkspace.ts
  git commit -m "feat: load model dataframe pages in frontend"
  ```

### Task 5: Build the read-only model table workbench

**Files:**
- Create: `frontend/src/components/dataframes/DataFrameTable.tsx`
- Create: `frontend/src/components/model/ModelDataFrameWorkbench.tsx`
- Create: `frontend/src/components/model/ModelDataFrameWorkbench.test.tsx`
- Modify: `frontend/src/components/datasets/DatasetTable.tsx`
- Modify: `frontend/src/components/datasets/DatasetTable.test.tsx`
- Modify: `frontend/src/components/datasets/DatasetWorkbench.tsx`
- Modify: `frontend/src/components/workspace/WorkspacePanel.tsx`
- Modify: `frontend/src/components/workspace/WorkspacePanel.test.tsx`
- Modify: `frontend/src/components/ResultPanel.tsx`
- Modify: `frontend/src/App.tsx`

**Interfaces:**
- Consumes: Task 4 `OpenDataFrame` and `useModelDataFrame`.
- Produces: accessible active/draft/history “查看” entries and the modal `ModelDataFrameWorkbench`.

- [ ] **Step 1: Write failing component tests**

  Assert active and draft cards enumerate every table with a named “查看 {tableName}” button; history versions expand without
  activation and expose the same buttons; model workbench shows the approved source title, table, column selection, pagination,
  and download but no chart tabs, lineage, analysis, or delete control. Add a stale-draft test that preserves existing rows,
  offers “重新加载最新修订”, opens the same table from the returned workspace snapshot, and closes with a notice if that table
  disappeared. Add session/source-switch tests that clear prior rows.

- [ ] **Step 2: Run focused component tests and verify red**

  Run: `cd frontend && npm test -- --run src/components/workspace/WorkspacePanel.test.tsx src/components/model/ModelDataFrameWorkbench.test.tsx src/components/datasets/DatasetTable.test.tsx`

  Expected: FAIL because the view callbacks and model workbench do not exist.

- [ ] **Step 3: Extract the presentation-only DataFrame table**

  Move table rendering into `dataframes/DataFrameTable.tsx` with props `{columns,page,loading,onMore}` using the common structural
  page fields `columns/rows/nextCursor/diagnostics`. Keep `DatasetTable.tsx` as a thin result-compatible wrapper so existing
  imports and tests remain valid; make both workbenches use the shared renderer.

- [ ] **Step 4: Implement `ModelDataFrameWorkbench`**

  Render a centered full-height modal body with title, column chooser, Parquet download, close button, loading/error states, and
  shared table. Implement draft conflict controls exactly as the spec. Use native buttons and labels; maintain full-width behavior
  at 390px.

- [ ] **Step 5: Wire workspace entries and top-level modal state**

  Add `onOpenDataFrame(OpenDataFrame)` through `ResultPanel` to `WorkspacePanel`. List active and draft tables; allow history
  expansion and viewing without activation. Replace App's standalone `datasetId` state with the approved `OpenDataFrame` union so
  result and model modals are mutually exclusive. Clear it on session selection/logout/resource removal; keep comparison behavior
  unchanged.

- [ ] **Step 6: Run frontend component suite and build**

  Run: `cd frontend && npm test -- --run && npm run build`

  Expected: all Vitest tests pass and TypeScript/Vite production build succeeds.

- [ ] **Step 7: Commit the model table UI**

  ```bash
  git add frontend/src/components/dataframes/DataFrameTable.tsx frontend/src/components/model/ModelDataFrameWorkbench.tsx frontend/src/components/model/ModelDataFrameWorkbench.test.tsx frontend/src/components/datasets/DatasetTable.tsx frontend/src/components/datasets/DatasetTable.test.tsx frontend/src/components/datasets/DatasetWorkbench.tsx frontend/src/components/workspace/WorkspacePanel.tsx frontend/src/components/workspace/WorkspacePanel.test.tsx frontend/src/components/ResultPanel.tsx frontend/src/App.tsx
  git commit -m "feat: view model tables in workspace"
  ```

### Task 6: Add browser coverage and reconcile all current contracts and docs

**Files:**
- Modify: `integration-tests/e2e/v3-workflows.spec.ts`
- Modify: `integration-tests/e2e/auth-isolation.spec.ts`
- Modify: `integration-tests/e2e/responsive-accessibility.spec.ts`
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `docs/api-design.md`
- Modify: `docs/frontend-design.md`
- Modify: `docs/model-workspace-design.md`
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/db-design.md`
- Modify: `docs/development-status.md`
- Modify: `docs/integration-testing-design.md`

**Interfaces:**
- Consumes: Tasks 2–5 complete public behavior.
- Produces: browser evidence and machine-readable traceability for 48 routes, 17 tools, six skills, eight upload sources, and the new model table view.

- [ ] **Step 1: Write failing Playwright journeys**

  Add `active draft and historical model tables are readable without changing lifecycle state` to `v3-workflows.spec.ts`. It
  must view active rows, create/view a draft, prove active rows remain unchanged, publish, view the prior historical version
  without activating it, exercise column selection/load-more/download, and assert reads created no run/result.

  Extend auth isolation to probe all six foreign model/draft URLs and compare their 404 bodies with missing IDs. Extend responsive
  accessibility coverage to open the model workbench at 390px and run the existing axe serious/critical gate.

- [ ] **Step 2: Run focused Playwright and verify red or missing coverage**

  Run: `cd frontend && npm run test:e2e -- --grep "model tables|cannot enumerate|responsive"`

  Expected before catalog/docs reconciliation: product journeys pass only after Tasks 2–5; coverage gate still reports the new
  contracts as undocumented.

- [ ] **Step 3: Update feature catalog and contract assertions**

  Add `MODEL-DATAFRAME-READ-001` with repository/API/agent/browser layers. Reference all six routes,
  `tool:read_model_dataframe`, errors `invalid_page`, `dataframe_read_failed`, `model_scope_unavailable`,
  `invalid_model_query`, and `model_query_output_too_large`, plus existing draft/table errors. Attach the exact Rust, Vitest, and
  Playwright test names from Tasks 1–6. Update release semantic assertions if the coverage checker requires a new view contract;
  do not satisfy behavior coverage with registry-only tests.

- [ ] **Step 4: Reconcile current design and operations docs**

  Document the six routes and DTOs, read-only fixed UI behavior, draft conflict recovery, shared Parquet decoding, no schema
  migration, inspect/read/modify Agent sequence, v3.1.0 bundle, and test matrix. Correct the current stale route count from 41 to
  48 and tool count from 16 to 17; preserve 9 message types, 6 skills, 8 upload sources, and 6 ECharts visualization kinds.
  Mark the approved spec implemented only after all verification passes.

- [ ] **Step 5: Run browser tests and functional coverage**

  Run: `cd frontend && npm run test:e2e`

  Run: `cd .. && ./scripts/check-functional-coverage --require-pr`

  Expected: all Playwright projects pass; every runtime route/tool/error has behavior evidence and no stale catalog test name.

- [ ] **Step 6: Commit browser evidence and documentation**

  ```bash
  git add integration-tests/e2e/v3-workflows.spec.ts integration-tests/e2e/auth-isolation.spec.ts integration-tests/e2e/responsive-accessibility.spec.ts integration-tests/catalog/features.yaml docs/api-design.md docs/frontend-design.md docs/model-workspace-design.md docs/agent-prompt-skills-design.md docs/db-design.md docs/development-status.md docs/integration-testing-design.md docs/superpowers/specs/2026-10-25-model-dataframe-viewing-design.md docs/superpowers/plans/2026-10-25-model-dataframe-viewing.md
  git commit -m "docs: cover model dataframe viewing contracts"
  ```

### Task 7: Run release verification and review the complete change

**Files:**
- Modify only files required to correct failures found by the commands below.

**Interfaces:**
- Consumes: all previous tasks.
- Produces: a clean, reviewable V3 candidate with no known contract, formatting, test, or build failure.

- [ ] **Step 1: Run backend release checks**

  Run:

  ```bash
  cd backend
  cargo fmt --all -- --check
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --features e2e-test
  ```

  Expected: all commands exit 0 with no warnings promoted to errors and all backend/unit/integration/doc tests passing.

- [ ] **Step 2: Run frontend release checks**

  Run:

  ```bash
  cd frontend
  npm test -- --run
  npm run build
  npm run test:e2e
  ```

  Expected: Vitest, TypeScript/Vite build, and Playwright all exit 0.

- [ ] **Step 3: Run traceability and whitespace checks**

  Run:

  ```bash
  ./scripts/check-functional-coverage --require-pr
  git diff --check
  git status --short
  ```

  Expected: coverage passes, no whitespace errors, and status contains only the planned files.

- [ ] **Step 4: Review the complete diff against the approved spec**

  Confirm all six routes are read-only/user-scoped, result DTOs are unchanged, stale draft reads cannot mix revisions, Agent
  query uniqueness uses full `matchCount`, UI has no direct editing/result-only controls, bundle hashes/version agree, and every
  public contract is listed in `features.yaml`.

- [ ] **Step 5: Commit any verification-only corrections**

  If Step 1–4 required changes, stage only those corrections and commit:

  ```bash
  git commit -m "fix: complete model dataframe viewing verification"
  ```

  If no files changed, do not create an empty commit.
