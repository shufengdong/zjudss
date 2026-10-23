# V3 DataFrame Models, Scripts, and Runs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make canonical Parquet DataFrames the only versioned model representation, keep RSS scripts as mutable session resources, require an explicit user button to create runs, and support controlled deletion of historical versions and individual result datasets.

**Architecture:** All model-like uploads and applicable examples pass through a persistent asynchronous `ModelImport`, adapter-owned import/classification, candidate resolution, and an optimistic-locking mutable draft. Explicit publish freezes a content-addressed immutable version and activates it. Scripts are independently uploaded or Agent-created, statically validated through the adapter, lazily displayed in chat, and dynamically checked against the active version when a user creates a `SimulationRun`. Results remain Parquet datasets; deletion releases only the selected source artifact while durable snapshots/tombstones keep materialized descendants explainable.

**Tech Stack:** Rust 2024, Axum, Diesel/SQLite, Arrow/Parquet Snappy, Tokio workers/SSE, tensoreval process adapter, rig/DeepSeek tools, React/TypeScript, Vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-23-v3-dataframe-model-scripts-runs-design.md`

## Global Constraints

- Execute after `2026-10-23-v3-fixed-user-auth.md`; all repository calls use authenticated `UserId` and trusted per-user storage scope.
- V3 is an empty-database cutover. Delete V2 source kinds, task API, and compatibility branches rather than migrating or dual-reading them.
- Every tensoreval invocation—including import classification, import execution, and script validation—must stay in `backend/src/adapters/`.
- The frontend and LLM pass only stable IDs and controlled specs, never server paths, commands, SQL, or arbitrary chart options.
- Model versions contain only canonical named Parquet DataFrames. Uploaded DSS/bundles/tabular source bytes are staging inputs and are released after apply/failure/discard as specified.
- Only the explicit script-card run button creates a run. Agent tools cannot execute, auto-publish, auto-activate, delete, or resolve user confirmations.
- A failed import/publish/run/result collection must not expose partial draft/version/result state.
- Use `SimulationRun`/`simulation_runs` and the Chinese UI term “运行记录”; remove `SimulationTask`/“仿真任务” public contracts.
- Public contract changes require behavior tests and functional-catalog evidence in the same task.

## Review Focus

- Import batches own unresolved candidates; the draft changes only in the final expected-draft transaction.
- Draft concurrency compares both `draft_id` and `revision`, including the empty-draft case.
- Publish retry returns one receipt, duplicate live content activates the existing version, and deleted content can receive a new version ID.
- Script static validation and active-model compatibility are separate; run creation rechecks both transactionally.
- Version deletion checks all active consumers and uses one-time impact tokens; derived materializations survive via snapshots/tombstones.
- Dataset deletion hides rows/download immediately and releases quota without breaking an already materialized view/analysis/comparison.

---

### Task 1: Replace the V2 workspace/task schema with V3 lifecycle entities

**Files:**
- Modify: `backend/migrations/00000000000000_initial/up.sql`
- Modify: `backend/migrations/00000000000000_initial/down.sql`
- Replace relevant declarations: `backend/src/db/schema.rs`
- Replace relevant models: `backend/src/db/models.rs`
- Modify exports: `backend/src/db/repositories/mod.rs`
- Add: `backend/src/db/repositories/model_import.rs`
- Replace: `backend/src/db/repositories/workspace.rs`
- Add: `backend/src/db/repositories/script.rs`
- Replace: `backend/src/db/repositories/simulation.rs`
- Replace: `backend/src/db/repositories/dataset.rs`
- Modify: `backend/src/db/repositories/{analysis,comparison}.rs`
- Add test: `backend/tests/v3_schema.rs`

- [ ] Write failing migration/repository tests for unique draft-per-workspace, unique table names, immutable published table rows, active-version workspace ownership, run/session/model/script consistency, idempotency keys, nullable artifact only after dataset deletion, and descendant snapshots.

- [ ] Run `cd backend && cargo test --test v3_schema --features e2e-test` and confirm failure against V2 tables.

- [ ] Replace `ingestions`, `version_artifacts`, `dataframe_bindings`, and `simulation_tasks` with:

  ```text
  workspace_drafts
  draft_dataframes
  model_imports
  model_import_candidates
  model_import_selections
  model_versions
  model_dataframes
  publish_receipts
  script_resources
  simulation_runs
  result_datasets
  dataset_lineage_snapshots
  version_deletion_previews
  ```

  Keep analysis/comparison/view tables but snapshot each input's stable ID, name, schema/hash, and producer summary. Add `deleted_at_ms` and `tombstone_json` to versions/runs/datasets where required; normal repository reads always filter tombstones. `dataset_views` must reference its independently materialized view dataset, not only the source.

- [ ] Enforce database invariants with foreign keys, unique constraints, partial indexes, and narrow triggers: one draft/workspace; `(draft_id,table_name)` and `(version_id,table_name)` unique; live `(workspace_id,content_hash)` unique; immutable model-version/dataframe content; deleted dataset has no artifact; run stores both resource and frozen script artifact/hash.

- [ ] Define repository records around stable domain names:

  ```rust
  pub struct DraftRef { pub id: i32, pub revision: i64 }
  pub struct ModelImportRecord { pub id: i32, pub status: ModelImportStatus, /* ... */ }
  pub struct ScriptResource { /* session, artifact/hash, validation, provenance */ }
  pub struct SimulationRun { /* model/script snapshot, state, timing, error */ }
  pub struct ResultDataset { pub artifact_id: Option<i32>, pub deleted_at_ms: Option<i64>, /* ... */ }
  ```

- [ ] Run the focused test, then `cd backend && cargo test --no-run --features e2e-test` to expose all V2 compile call sites for subsequent tasks.

- [ ] Commit: `git add backend/migrations backend/src/db backend/tests/v3_schema.rs && git commit -m "refactor: define v3 model lifecycle schema"`.

### Task 2: Extend the engine adapter with import and script-validation contracts

**Files:**
- Modify: `backend/src/adapters/mod.rs`
- Modify: `backend/src/adapters/contract.rs`
- Modify: `backend/src/adapters/process/{mod,command,output}.rs`
- Modify: `backend/src/adapters/stub.rs`
- Add: `backend/src/adapters/import.rs`
- Add: `backend/src/adapters/script.rs`
- Modify: `backend/src/engine_resources/{mod,manifest}.rs` or the actual manifest reader files
- Modify: `backend/resources/agent/v2/manifest.json` only when Task 10 cuts the whole bundle to V3
- Add test: `backend/tests/model_import_adapter.rs`
- Add test: `backend/tests/script_validation_adapter.rs`
- Modify: `backend/tests/process_engine_v2.rs` (rename to `process_engine_v3.rs` if no external references remain)
- Modify: `backend/tests/engine_resources.rs`

- [ ] Write adapter contract tests using a fake executable/scenario engine. Cover include-graph roots, stable candidate IDs without physical paths, engine-owned static/execution classification, exactly 21 static tables including zero-row tables, execution input tables excluding diagnostics/events, unsupported lifecycles, Snappy/schema verification, and source-tree boundaries.

- [ ] Write validation tests proving imports/paths/URLs/external reads are rejected by adapter diagnostics, standard required aliases and declared outputs are extracted, valid scripts need no active model, and no handler-side blacklist is required.

- [ ] Run `cd backend && cargo test --test model_import_adapter --test script_validation_adapter --features e2e-test` and confirm failure.

- [ ] Add structured adapter interfaces (names may be module-qualified, behavior may not vary):

  ```rust
  pub enum ImportProfile { StaticDss, SingleSolveExecution }
  pub struct ImportEntryCandidate { pub id: String, pub display_path: String }
  pub struct ImportSpec { pub input_tree: PathBuf, pub entry_candidate_id: String,
                          pub result_dir: PathBuf, pub timeout: Duration }
  pub struct ImportedModel { pub profile: ImportProfile,
                             pub dataframes: Vec<EngineDatasetOutput>,
                             pub diagnostics: serde_json::Value }
  pub struct StaticScriptValidation { pub valid: bool, pub required_aliases: Vec<String>,
                                      pub outputs: Vec<String>, pub diagnostics: Value }

  async fn inspect_model_import(&self, spec: ImportInspectSpec) -> Result<ImportInspection, AdapterError>;
  async fn import_model(&self, spec: ImportSpec) -> Result<ImportedModel, AdapterError>;
  async fn validate_script(&self, spec: ScriptValidationSpec) -> Result<StaticScriptValidation, AdapterError>;
  ```

  The process implementation must build argument arrays only. Prefer a dedicated structured engine subcommand if the pinned engine exposes one; otherwise generate a fixed internal import-only RSS template inside the adapter and pin its output names/order in the engine contract. Never execute bundle `main.rss` to infer model tables.

- [ ] Extend startup capabilities/preflight to verify both import profiles, named Parquet inputs, result-dir Parquet, and script validation. Add deterministic Stub/Scenario responses for application tests.

- [ ] Run the focused tests and `cd backend && cargo test --test engine_resources --features e2e-test`.

- [ ] Commit: `git add backend/src/adapters backend/src/engine_resources backend/tests && git commit -m "feat: add dataframe import adapter contracts"`.

### Task 3: Build persistent asynchronous model imports and candidate resolution

**Files:**
- Replace: `backend/src/application/ingestion.rs` → `backend/src/application/model_import.rs`
- Modify: `backend/src/application/mod.rs`
- Replace: `backend/src/api/ingestions.rs` → `backend/src/api/model_imports.rs`
- Modify: `backend/src/api/mod.rs`
- Modify: `backend/src/ingestion/{archive,bundle,tabular}.rs`
- Remove model behavior: `backend/src/ingestion/rss.rs`
- Modify: `backend/src/application/events.rs`
- Modify: `backend/src/db/repositories/model_import.rs`
- Add test: `backend/tests/model_imports.rs`
- Replace/modify: `backend/tests/{bundle_ingestion,dataframe_ingestion,failure_atomicity,quota_concurrency}.rs`

- [ ] Write failing behavior tests for 202 acceptance/idempotency; states `accepted|processing|awaiting_resolution|ready_to_apply|applied|failed|discarded`; single/multiple DSS roots; bundle manifest entry; direct CSV/Excel/Parquet ambiguity; same-table duplicate choice; minimum staging retention; quota accounting; revision conflict preview refresh; service-restart convergence; and cleanup on applied/failed/discarded.

- [ ] Run `cd backend && cargo test --test model_imports --features e2e-test` and confirm failure.

- [ ] Implement one worker pipeline shared by uploads and examples:

  ```rust
  pub struct CreateModelImport { pub session_id: i32, pub client_import_id: String,
      pub expected_draft: Option<DraftRef>, pub source: ModelImportSource }
  pub struct ResolveModelImport { pub entry_candidate_id: Option<String>,
      pub table_selections: Vec<TableSelection>, pub expected_draft: Option<DraftRef> }

  pub async fn accept(&self, user: &CurrentUser, request: CreateModelImport)
      -> Result<ModelImportView, ModelImportError>;
  pub async fn resolve_and_apply(&self, user: &CurrentUser, import_id: i32,
      request: ResolveModelImport) -> Result<ModelImportView, ModelImportError>;
  pub async fn discard(&self, user: &CurrentUser, import_id: i32) -> Result<(), ModelImportError>;
  ```

  Persist state/stages before spawning work. Candidate Parquets belong to the import until the final transaction. At apply, lock/re-read draft identity and revision, create the first draft if expected empty, whole-table replace selected names, increment revision once, transfer artifact references, then release staging references.

- [ ] Expose exact routes:

  ```text
  POST   /api/sessions/{id}/model-imports       multipart, returns 202
  GET    /api/model-imports/{id}
  POST   /api/model-imports/{id}/resolve
  DELETE /api/model-imports/{id}
  ```

  Use stable candidate IDs in requests and virtual display paths in responses. Return `draft_revision_conflict`, `model_import_unsupported`, `model_import_resolution_required`, and existing archive/quota errors with structured details.

- [ ] On startup, atomically mark `processing` imports `failed` with `service_restarted`, clear process work dirs, and keep persisted candidates/diagnostics readable. Publish complete stage snapshots through the existing durable-message-before-broadcast event path.

- [ ] Run focused ingestion/import tests and then `cd backend && cargo test --features e2e-test` (compile failures in still-unconverted V2 task/workspace tests may be temporarily narrowed in the commit, but no test may be skipped).

- [ ] Commit: `git add backend/src/application backend/src/api backend/src/ingestion backend/src/db/repositories backend/tests && git commit -m "feat: add persistent model imports"`.

### Task 4: Implement mutable drafts, explicit publish, activation, and historical deletion

**Files:**
- Replace: `backend/src/application/workspace.rs`
- Modify: `backend/src/application/model_patch.rs`
- Replace: `backend/src/db/repositories/workspace.rs`
- Replace: `backend/src/api/workspaces.rs`
- Modify: `backend/src/api/mod.rs`
- Modify: `backend/src/application/artifacts.rs`
- Replace tests: `backend/tests/workspace_versions.rs`, `backend/tests/workspace_tools.rs`
- Add test: `backend/tests/version_deletion.rs`

- [ ] Write failing tests for draft creation/copy-on-reference, `{draftId,revision}` optimistic locking, table replace/remove, explicit publish, atomic active switch/clear draft, publish receipt retry, live-content dedup activation, republish after tombstone, activation with draft-discard confirmation, impact preview token, active/busy rejection, changed-impact rejection, run/raw-result cascade, and derived tombstone survival.

- [ ] Run `cd backend && cargo test --test workspace_versions --test version_deletion --features e2e-test` and confirm failure.

- [ ] Define service requests and stable errors:

  ```rust
  pub struct ExpectedDraft { pub draft_id: Option<i32>, pub revision: Option<i64> }
  pub struct PublishDraftRequest { pub expected: ExpectedDraft, pub client_publish_id: String }
  pub struct ActivateVersionRequest { pub discard_draft: bool,
                                      pub expected_draft: Option<DraftRef> }
  pub struct VersionDeletionPreview { pub impact_token: String, pub run_count: u64,
      pub result_count: u64, pub reclaimable_bytes: u64, pub expires_at: String }
  ```

  Content hash sorts by canonical table name and hashes table name + Parquet content hash + canonical schema/row metadata. The publish transaction verifies all artifacts and model completeness before any pointer changes.

- [ ] Use these routes:

  ```text
  GET    /api/sessions/{id}/workspace
  GET    /api/sessions/{id}/versions
  POST   /api/sessions/{id}/versions
  GET    /api/versions/{id}
  POST   /api/versions/{id}/activate
  POST   /api/versions/{id}/deletion-preview
  DELETE /api/versions/{id}?impactToken=<opaque>
  ```

  Deletion previews are short-lived, user/version/impact-bound hashes persisted in `version_deletion_previews`. The delete transaction repeats active-consumer and impact-set checks, tombstones version/run/raw results, drops visible artifact refs, and preserves materialized descendants/snapshots.

- [ ] Update model patch tools to mutate only drafts using expected identity/revision. Remove rollback-as-new-version behavior; activating a history item only moves the active pointer.

- [ ] Run focused tests and all backend workspace tests.

- [ ] Commit: `git add backend/src/application backend/src/db/repositories/workspace.rs backend/src/api backend/tests && git commit -m "feat: add explicit dataframe model versions"`.

### Task 5: Route the model case library through the same import/draft transaction

**Files:**
- Replace: `backend/src/application/example_import.rs` → `backend/src/application/case_import.rs`
- Modify: `backend/src/api/examples.rs`
- Modify: `backend/src/api/mod.rs`
- Modify: `backend/src/engine_resources/examples.rs` or actual example catalog files
- Remove: `backend/src/tools/import_example.rs`
- Modify: `backend/src/tools/search_examples.rs`
- Modify: `backend/src/tools/inspect_example.rs`
- Replace test: `backend/tests/example_import.rs` → `backend/tests/case_import.rs`
- Modify: `backend/tests/release_engine_cases.rs`

- [ ] Write failing tests that the catalog contains only modelizable cases; preview reports new/overwrite tables and row counts; confirm uses expected draft ID/revision; same-name tables replace wholesale; absent case tables remain; and apply creates no version/script/run/result.

- [ ] Implement `POST /api/sessions/{id}/draft/example` to create a `ModelImport` with `source=case`. Return the same stages/candidates/merge preview and require the same `/resolve` confirmation. Keep example paths inside the engine snapshot and out of public DTOs.

- [ ] Remove Agent `import_example` write authority. Search/inspect tools may explain a case and return its stable ID; only the fixed “应用案例” UI action starts it.

- [ ] Run `cd backend && cargo test --test case_import --test release_engine_cases --features e2e-test`.

- [ ] Commit: `git add backend/src/application backend/src/api backend/src/engine_resources backend/src/tools backend/tests && git commit -m "feat: import cases into mutable drafts"`.

### Task 6: Add immutable session script resources and the Agent `create_script` tool

**Files:**
- Add: `backend/src/application/scripts.rs`
- Add: `backend/src/api/scripts.rs`
- Modify: `backend/src/api/sessions.rs`
- Modify: `backend/src/api/mod.rs`
- Add: `backend/src/tools/create_script.rs`
- Remove: `backend/src/tools/run_study.rs`
- Modify: `backend/src/tools/mod.rs`
- Modify: `backend/src/agent/{mod,knowledge}.rs`
- Modify: `backend/src/application/events.rs`
- Modify: `backend/src/db/repositories/{script,message}.rs`
- Add test: `backend/tests/scripts.rs`
- Modify: `backend/tests/tool_manifest.rs`

- [ ] Write failing tests for single `.rss` upload routing, Agent creation, independent duplicate resources, static-valid without active model, invalid-but-readable scripts, lazy full content, bounded message preview, provenance refs, no edit/delete endpoint, cross-user/session isolation, and full cleanup only with whole-session deletion.

- [ ] Run `cd backend && cargo test --test scripts --test tool_manifest --features e2e-test` and confirm failure.

- [ ] Implement:

  ```rust
  pub struct CreateScriptRequest { pub name: String, pub content: String,
      pub source: ScriptSource, pub agent_skill_refs: Vec<Value> }
  pub struct ScriptSummary { /* id/name/source/hash/static status/preview/eligibility */ }
  pub struct ScriptDetail { pub summary: ScriptSummary, pub content: String }
  ```

  Commit the full RSS artifact, call adapter static validation, persist status/aliases/outputs/diagnostics, and append a `script` chat message containing only summary + bounded preview. Eligibility joins the current active version tables at read time; creation remains allowed with no active model.

- [ ] Add routes:

  ```text
  GET  /api/sessions/{id}/scripts
  POST /api/sessions/{id}/scripts             single multipart .rss
  GET  /api/scripts/{id}                      includes full content
  ```

  The general upload UI must inspect the top-level selection: a lone `.rss` uses scripts; DSS/bundle/tabular uses model imports; mixed script+model selection is split into one script request and one model-import request with distinct client IDs.

- [ ] Register `create_script` as a skill-gated Agent tool with controlled `name` and `content`; remove `run_study` from source, registry, schemas, prompt, and manifest. Ensure an Agent turn cannot call any execution service indirectly.

- [ ] Run focused tests, registry/manifest parity tests, and all backend tests.

- [ ] Commit: `git add backend/src backend/tests && git commit -m "feat: add validated session scripts"`.

### Task 7: Replace simulation tasks with explicit user-created runs

**Files:**
- Replace: `backend/src/application/simulations.rs` → `backend/src/application/runs.rs`
- Replace: `backend/src/api/tasks.rs` → `backend/src/api/runs.rs`
- Replace: `backend/src/db/repositories/task.rs` → `backend/src/db/repositories/run.rs`
- Modify: `backend/src/db/repositories/simulation.rs`
- Modify: `backend/src/application/{mod,scheduler,events}.rs`
- Modify: `backend/src/api/mod.rs`
- Modify: `backend/src/adapters/mod.rs`
- Replace test: `backend/tests/v2_simulation.rs` → `backend/tests/simulation_runs.rs`
- Modify: `backend/tests/{failure_atomicity,quota_concurrency,sse_recovery}.rs`

- [ ] Write failing tests for explicit route-only creation, script/version/session ownership, active-version race, dynamic alias/schema incompatibility, client-run idempotency, each click with a new ID producing a new run, frozen script/model hashes, queued/running recovery, private input tree, all-or-none multi-Parquet result publication, and SSE run fields.

- [ ] Run `cd backend && cargo test --test simulation_runs --features e2e-test` and confirm failure.

- [ ] Implement the only launch API:

  ```rust
  pub struct CreateRunRequest {
      pub model_version_id: String,
      pub client_run_id: String,
  }

  POST /api/scripts/{id}/runs
  GET  /api/sessions/{id}/runs
  GET  /api/runs/{id}
  GET  /api/runs/{id}/datasets
  ```

  In one transaction, resolve the script/session/user, re-read workspace active, compare the requested version, re-evaluate script compatibility, acquire the idempotency receipt, add a script artifact reference as the run snapshot, and create `queued`. Return `409 active_model_changed` or stable validation details without creating a run.

- [ ] Materialize every `model_dataframes` artifact under a stable alias and execute only `RunMode::Dataframe` with `--result-dir ... --output-format parquet`. Validate all outputs before a result-publication transaction. Rename event payload fields and error codes from task to run; retain scheduler global/per-user limits.

- [ ] On startup mark queued/running runs failed with `service_restarted`; do not alter already committed result datasets. Remove public `/api/tasks/*` routes and all old request/DTO names.

- [ ] Run focused tests and `cd backend && cargo test --features e2e-test`.

- [ ] Commit: `git add backend/src backend/tests && git commit -m "feat: run scripts only on explicit user action"`.

### Task 8: Implement individual result deletion and durable derived lineage

**Files:**
- Modify: `backend/src/application/{analysis,comparisons,visualization}.rs`
- Modify: `backend/src/application/datasets.rs` or actual dataset service file
- Modify: `backend/src/db/repositories/{dataset,analysis,comparison}.rs`
- Modify: `backend/src/api/datasets.rs`
- Modify: `backend/src/api/analysis.rs`
- Modify: `backend/src/api/comparisons.rs`
- Modify: `backend/src/application/events.rs`
- Add test: `backend/tests/result_deletion.rs`
- Modify: `backend/tests/{result_analysis,task_comparison}.rs` (rename task comparison terminology)

- [ ] Write failing tests for deleting one run result, immediate list/rows/download 404 behavior, artifact/quota release, run retention with zero results, `result_busy`, ban on new descendants, already materialized analysis/comparison readability, materialized view readability, and tombstone lineage summaries.

- [ ] Make view creation persist its bounded projection/aggregation as an independent Snappy Parquet dataset. Snapshot all analysis/comparison/view input metadata at creation so responses never need deleted source artifacts.

- [ ] Implement `DELETE /api/datasets/{id}`. In one transaction check ownership and queued/running consumers, set `deleted_at_ms`, save tombstone metadata, detach `artifact_id`, and remove normal visibility; then invoke idempotent artifact GC. Publish a durable dataset-deleted event after commit.

- [ ] Rename comparison DTOs/routes that expose “task” to “run” while keeping comparison/analysis stable IDs. Ensure tools list only live datasets but can render lineage tombstone labels.

- [ ] Run focused tests and all backend analysis/comparison/dataset tests.

- [ ] Commit: `git add backend/src backend/tests && git commit -m "feat: delete individual result datasets"`.

### Task 9: Rebuild frontend types, import/draft/version workspace, and case flow

**Files:**
- Replace: `frontend/src/types/workspace.ts`
- Add: `frontend/src/types/modelImport.ts`
- Replace: `frontend/src/types/task.ts` → `frontend/src/types/run.ts`
- Add: `frontend/src/types/script.ts`
- Modify: `frontend/src/types/{index,message,dataset}.ts`
- Modify: `frontend/src/types/contracts.test.ts`
- Modify: `frontend/src/api/client.ts`
- Replace: `frontend/src/hooks/useIngestion.ts` → `frontend/src/hooks/useModelImports.ts`
- Replace: `frontend/src/hooks/useWorkspace.ts`
- Replace: `frontend/src/components/workspace/WorkspacePanel.tsx`
- Modify: `frontend/src/components/uploads/UploadQueue.tsx`
- Modify: `frontend/src/components/examples/{ExampleLibrary,ExampleDetail}.tsx`
- Modify: `frontend/src/state/workspace.ts`
- Replace tests: `frontend/src/state/workspace.test.ts`
- Add tests: `frontend/src/components/workspace/WorkspacePanel.test.tsx`

- [ ] Write failing tests for upload classification, async stage recovery, candidate entry/table choices, revision conflicts/reload, merge preview, whole-table overwrite, explicit enabled/disabled “生成版本”, automatic active result, history activation with draft-discard confirmation, and version deletion impact/busy handling.

- [ ] Replace public TS contracts with exact backend camelCase DTOs. IDs remain strings; timestamps ISO 8601. The workspace owns `draft: {id,revision,tables,diagnostics}|null`, `active`, and history. Model imports retain state across refresh and merge SSE snapshots by stable import ID.

- [ ] Add fixed actions for resolve/apply, discard import, publish version, activate version, and preview/confirm deletion. Never synthesize natural-language messages for these unambiguous operations. Route `.rss` uploads to scripts and all supported model files to model imports, splitting mixed selections as specified.

- [ ] Case “应用” starts a case import; display overwrite/add preview and require confirmation before resolve/apply. Do not publish or run afterward.

- [ ] Run `cd frontend && npm test -- --run && npm run build`.

- [ ] Commit: `git add frontend/src && git commit -m "feat: add v3 dataframe model workspace"`.

### Task 10: Add script cards, explicit run controls, run history, and result deletion UI

**Files:**
- Add: `frontend/src/components/messages/ScriptCard.tsx`
- Modify: `frontend/src/components/messages/MessageItem.tsx`
- Modify: `frontend/src/state/messageReducer.ts`
- Modify: `frontend/src/state/messageReducer.test.ts`
- Add: `frontend/src/hooks/useScripts.ts`
- Replace: `frontend/src/hooks/useSimulationHistory.ts` → `frontend/src/hooks/useRunHistory.ts`
- Replace: `frontend/src/components/tasks/TaskHistory.tsx` → `frontend/src/components/runs/RunHistory.tsx`
- Modify: `frontend/src/components/ResultPanel.tsx`
- Modify: `frontend/src/components/datasets/DatasetWorkbench.tsx`
- Modify: `frontend/src/hooks/useDataset.ts`
- Modify: `frontend/src/App.tsx`
- Add tests: `frontend/src/components/messages/ScriptCard.test.tsx`
- Add tests: `frontend/src/components/runs/RunHistory.test.tsx`
- Modify tests: `frontend/src/hooks/useDataset.test.ts`, `frontend/src/release/semantic-assertions.test.ts`

- [ ] Write failing tests at both sides of the long-script fold threshold; expand/collapse; lazy full-content fetch; copy full content only; copy failure; run disabled with no active/static error/incompatibility/submission; active-race error; one click/one client ID; no script edit/delete; “运行记录” labels/no delete; and single-result confirm/delete/count update.

- [ ] Implement a code-block `ScriptCard` with a small local `expanded/loading/content` state. Use a constant such as `SCRIPT_PREVIEW_LINES = 20`; short previews render fully, long previews clamp until “展开全文”. Both expand and copy fetch `GET /api/scripts/{id}` and cache only inside the mounted card, not the global message reducer.

- [ ] The run button captures the currently rendered active version ID and sends a newly generated `clientRunId`; disable until completion/error. On `active_model_changed`, reload workspace and require another click. Never call run from message receipt, Agent text, or effect hooks.

- [ ] Replace task history and API helpers with run terminology. Dataset detail shows “删除结果” with a confirmation that derived materializations remain; after success close the deleted dataset and update run result count. There is no run-delete button.

- [ ] Run `cd frontend && npm test -- --run && npm run build`.

- [ ] Commit: `git add frontend/src && git commit -m "feat: add explicit script run experience"`.

### Task 11: Cut Agent bundle, public contracts, browser journeys, and release gates to V3

**Files:**
- Modify: `backend/resources/agent/v2/system.md` and move bundle directory to `backend/resources/agent/v3/`
- Modify: `backend/resources/agent/v3/manifest.json`
- Modify: `backend/resources/agent/v3/skill-index.json`
- Modify: `backend/resources/agent/v3/skills/*.md`
- Modify: `backend/src/agent/knowledge.rs`
- Modify: `backend/src/config.rs`
- Modify: `backend/tests/{tool_manifest,contract_manifest,e2e_v2}.rs` (rename e2e test to V3)
- Replace/modify all: `integration-tests/e2e/*.spec.ts`
- Modify: `integration-tests/e2e/helpers/{control,events,journeys,uploads}.ts`
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `docs/{architecture,api-design,db-design,frontend-design,model-workspace-design,agent-prompt-skills-design,development-status,integration-testing-design}.md`
- Modify: `docs/v3-deployment.md`
- Modify: `.github/workflows/{pr-functional-coverage,release-real-deps}.yml`
- Modify: `scripts/check-functional-coverage`

- [ ] Publish an Agent V3 bundle that can inspect/mutate drafts, create scripts, inspect runs/results, and analyze/compare live results. Remove `run_study`, Agent activation/deletion, and Agent case-apply authority from code and manifest. Update system prompt to state that generated scripts wait for the user's run button.

- [ ] Replace V2 route/message/tool DTO docs and contract manifests in one cut. The ninth durable message kind is `script`; progress/result payloads use `runId`; document all import/publish/activate/delete errors and the empty-database reset requirement.

- [ ] Rewrite browser journeys to cover:

  1. DSS directory → async import → source release → draft → publish/active → script expand/copy → click run → multiple results.
  2. CSV/Excel/Parquet batches → mapping conflict → table replacement → publish → history activation.
  3. Static/single-Solve bundle success and unsupported multi-Solve/specialist rejection with unchanged draft.
  4. Case merge preview/confirm without publish/run.
  5. Agent `create_script` with no automatic run, followed by explicit click and active-race rejection.
  6. Single result deletion with readable materialized descendant and source rows/download 404.
  7. Historical version deletion with run/raw-result disappearance and descendant tombstone lineage.
  8. Two-user isolation for imports, drafts, versions, scripts, runs, datasets, SSE, and deletion.

- [ ] Update the feature catalog so every new/removed route, tool, message, format, error, and fixed-button behavior maps to actual Rust/Vitest/Playwright behavior. Do not use registry enumeration as the sole coverage for behavior.

- [ ] Run all deterministic gates:

  ```bash
  cd backend
  cargo fmt --all -- --check
  cargo test --features e2e-test
  cd ../frontend
  npm ci
  npm test -- --run
  npm run build
  npm run test:e2e
  cd ..
  ./scripts/check-functional-coverage --require-pr
  ```

- [ ] Run the candidate real-dependency gate from `docs/v3-deployment.md` with the pinned engine artifact and DeepSeek secret. It must exercise both import profiles, direct DataFrames, publish, explicit script run, and result deletion, and write engine/agent/seed hashes into the report. Do not reuse V2 evidence.

- [ ] Run final hygiene checks:

  ```bash
  rg -n 'SimulationTask|simulation_tasks|/api/tasks|run_study|source_kind.*rssbundle|draft_version_id' backend frontend integration-tests docs --glob '!docs/superpowers/**'
  git diff --check
  git status --short
  ```

  Expected production matches are zero except historical migration commentary deliberately retained in the approved spec. Verify the user's `backend/resources/init_users.csv` remains untouched/uncommitted unless the user separately asks to commit it.

- [ ] Commit: `git add backend/resources backend/src backend/tests frontend/src integration-tests docs .github scripts && git commit -m "release: cut v3 dataframe workflow"`.
