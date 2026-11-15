# Conversation Agent Direct RSS Analysis Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the current conversation Agent directly generate, validate, repair, execute, and expose controlled RSS analyses over the current active model, published local DataFrames, and live result DataFrames.

**Architecture:** eig-rs adds full-data `assert_all` and lockstep `explode` functions and publishes their exact machine/document contracts. zjudss adds catalog-driven script preflight, active-only model input freezing, exact dataset/model-origin checks, bounded repair feedback, and immutable script/input lineage; the existing conversation Agent remains the only script author. The frontend lazily exposes the final analysis script and frozen sources without adding edit or run authority.

**Tech Stack:** Rust, Axum, Diesel/SQLite, rig, Polars/RSDss, Parquet/Snappy, React, TypeScript, Vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-11-14-conversation-agent-direct-rss-analysis-design.md`

## Global Constraints

- Work directly on `master`; do not create a feature worktree unless the user changes this instruction.
- Preserve unrelated dirty files in `/Users/dongshufeng/IdeaProjects/eig-rs` and its `sparrowzz` submodule; stage only paths named by each task.
- All tensoreval process calls remain behind `backend/src/adapters/`; tools, handlers, and application services never spawn or assemble commands.
- Analysis model inputs must belong to the current session's exact active model version; draft and historical model DataFrames are forbidden.
- When active model and dataset inputs are mixed, every dataset must resolve to exactly one frozen model-version origin equal to the active version ID.
- The LLM and browser receive only stable IDs, controlled specs, bounded schema/sample data, and sanitized diagnostics—never paths, SQL, Python, shell, secrets, or arbitrary ECharts options.
- `run_result_analysis` permits at most four calls per user request: initial attempt plus three repairs; conversation `MAX_TURNS` is exactly 12.
- `assert_all` evaluates the complete DataFrame and treats false or null as failure; `explode` rejects null, empty, non-list, duplicate, missing, or unequal-length selected columns without partial output.
- Analysis execution has no fixed wall-clock timeout and no new user-cancel endpoint. Existing 100-file, 2 GiB/file, 8 GiB/total, quota, and path-safety gates remain authoritative.
- Outputs remain immutable Snappy Parquet; no CSV results, partial publication, fixed metrics, or hard-coded `v_pu` output column is introduced.
- V4 remains an empty-database baseline: update `backend/migrations/00000000000000_initial/`, not a compatibility migration.
- Agent bundle becomes `v4.5.0`, `result-analysis` becomes `1.3.0`, allowed tools become 24, while engine contract stays `tensoreval-parquet-v3` and catalog schema versions do not change.
- Use test-driven development for every behavior change and commit after each independently verified task.
- Unless a step explicitly changes directory, run eig-rs commands from `/Users/dongshufeng/IdeaProjects/eig-rs`, backend commands from `/Users/dongshufeng/IdeaProjects/zjudss/backend`, frontend commands from `/Users/dongshufeng/IdeaProjects/zjudss/frontend`, and repository scripts from `/Users/dongshufeng/IdeaProjects/zjudss`.

## Review Focus

- A direct result, a resource-only derived result, a multi-model derived result, a lineage cycle, and depth 65 must resolve respectively to exact active origin, no origin, multiple origins, invalid cycle, and invalid depth; Task 5 pins all five outcomes.
- Switching active versions between workspace inspection and execution must return `active_model_changed` without substituting a same-name table; Task 5 pins the race.
- A false/null full-data predicate or null/empty/misaligned List row must fail without a result file; Tasks 1–3 pin engine and CLI behavior.
- Unknown, wrong-arity, nested IO/URL/DSA, input-rebinding, direct-alias, or wrong-terminal-output scripts must fail before script artifact/run creation; Tasks 4 and 6 pin zero-write preflight.
- A fifth repair or a non-repairable infrastructure/path/quota error must not invoke the engine, and repair feedback must contain no absolute path or secret; Tasks 6 and 8 pin the budget and redaction.

## File Map

- `eig-rs/mems/src/ml/expr_df/frame_ops.rs`: exact `assert_all`/`explode` runtime dispatch and focused unit tests.
- `eig-rs/mems/src/bin/tensoreval.rs`: CLI-level named-input/result-dir proof for the two new functions.
- `eig-rs/sparrowzz/dss/docs/catalog/functions.json`: machine-readable arity and behavior contracts.
- `eig-rs/sparrowzz/dss/docs/rsdss_function_reference.md`: Agent-readable current syntax and failure semantics.
- `backend/src/application/analysis/script_inspector.rs`: current-snapshot structural allowlist and terminal-output preflight.
- `backend/src/application/analysis/model_origin.rs`: active/result model-origin compatibility policy.
- `backend/src/application/analysis/inputs.rs`: resolution and staging of dataset, resource, and active-model inputs.
- `backend/src/application/analysis/diagnostics.rs`: adapter failure classification and sanitized model feedback.
- `backend/src/application/analysis.rs`: orchestration, preflight, immutable snapshots, and details facade.
- `backend/src/db/repositories/{analysis,result,version,version_deletion}.rs`: source snapshots, recursive origin reads, active lookup, and delete races.
- `backend/src/tools/read_result_dataset.rs`: bounded result metadata/sample tool.
- `backend/src/tools/{inspect_workspace,read_model_dataframe,run_result_analysis}.rs`: active IDs, skill gates, three-way input union, and attempt claims.
- `backend/resources/agent/v4/{system.md,skill-index.json,skills/result-analysis.md,manifest.json}`: direct-author routing and bundle authority.
- `backend/src/api/analysis.rs`: script and frozen-input detail projection on the existing GET route.
- `frontend/src/components/messages/AnalysisDetails.tsx`: lazy, read-only script/source disclosure.
- `frontend/src/components/messages/ResultCard.tsx`: analysis-detail entry point.
- `frontend/src/types/{run,dataset}.ts` and `frontend/src/api/client.ts`: additive analysis/lineage DTOs.
- `integration-tests/catalog/features.yaml`, deterministic E2E, release journey, and current design docs: behavior traceability and release evidence.

---

### Task 1: Add full-DataFrame `assert_all` to eig-rs

**Files:**
- Modify: `/Users/dongshufeng/IdeaProjects/eig-rs/mems/src/ml/expr_df/frame_ops.rs`

**Interfaces:**
- Consumes: existing `exact_arguments`, `resolve::dataframe`, `translate`, `ExprDfError`, and Polars lazy evaluation.
- Produces: top-level RSDss `assert_all(predicate, frame) -> DataFrame`, returning the input unchanged only when every predicate value is true.

- [ ] **Step 1: Write failing unit tests**

Add focused tests named `assert_all_returns_the_unchanged_frame_when_every_row_is_true`, `assert_all_reports_false_and_null_counts`, `assert_all_rejects_non_boolean_or_wrong_length_predicates`, and `assert_all_accepts_an_empty_frame`. Pin these assertions:

```rust
assert_eq!(asserted, source);
assert!(error.contains("rows=3"));
assert!(error.contains("false=1"));
assert!(error.contains("null=1"));
assert!(non_boolean_error.contains("Boolean"));
assert_eq!(empty.height(), 0);
```

- [ ] **Step 2: Run the tests and prove the function is absent**

Run: `cargo test -p mems assert_all --lib`

Expected: FAIL because `assert_all` is an unsupported DataFrame function.

- [ ] **Step 3: Implement exact runtime dispatch**

Add `assert_all` to `frame_ops::supports` and `frame_ops::evaluate`, with this private interface:

```rust
fn assert_all(
    expression: &eig_expr::Expr,
    frames: &HashMap<String, DataFrame>,
    literals: Option<&HashMap<String, LiteralValue>>,
) -> ExprDfResult<DataFrame>;
```

Require exactly two arguments. Evaluate the predicate against the full frame and require an equal-length Boolean result; empty is vacuously valid. On a false/null condition, return `ExprDfError::OperationFailed { function: "assert_all", detail }` containing only `rows`, `false`, and `null` counts. Use a stable type/length error without row values for invalid predicates.

- [ ] **Step 4: Run focused tests**

Run: `cargo test -p mems assert_all --lib`

Expected: all `assert_all` tests PASS.

- [ ] **Step 5: Format and commit only the runtime file**

```bash
cargo fmt --all -- --check
git add mems/src/ml/expr_df/frame_ops.rs
git commit -m "feat(mems): add full-frame assertion"
```

### Task 2: Add strict lockstep `explode` to eig-rs

**Files:**
- Modify: `/Users/dongshufeng/IdeaProjects/eig-rs/mems/src/ml/expr_df/frame_ops.rs`

**Interfaces:**
- Consumes: Task 1's frame-op dispatch pattern and existing literal-column helpers.
- Produces: top-level RSDss `explode(columns, frame) -> DataFrame` with strict positive lockstep lengths.

- [ ] **Step 1: Write failing unit tests**

Add tests named `explode_repeats_scalar_columns_and_expands_one_list`, `explode_expands_multiple_columns_in_lockstep`, and `explode_rejects_invalid_selected_columns`. Cover single/multiple columns, repeated scalar values, duplicate/missing/non-list columns, null lists, empty lists, and unequal lengths.

```rust
assert_eq!(expanded.height(), 3);
assert_eq!(expanded.column("phase")?.str()?.get(1), Some("B"));
assert!(invalid_cases.iter().all(|result| result.is_err()));
```

- [ ] **Step 2: Run tests and verify unsupported-function failures**

Run: `cargo test -p mems explode --lib`

Expected: FAIL because `explode` is unsupported.

- [ ] **Step 3: Implement strict `explode`**

Add dispatch plus this private interface:

```rust
fn explode(
    expression: &eig_expr::Expr,
    frames: &HashMap<String, DataFrame>,
    literals: Option<&HashMap<String, LiteralValue>>,
) -> ExprDfResult<DataFrame>;
```

Resolve exactly two arguments; accept one name, comma-separated names, or a name array. Validate non-empty/unique names, List/Array dtype, and for every row non-null, positive, equal selected-list lengths before calling Polars explode. Return a stable `OperationFailed` detail and no frame on any violation.

- [ ] **Step 4: Run focused and adjacent frame-op tests**

Run: `cargo test -p mems explode --lib && cargo test -p mems ml::expr_df::frame_ops::tests --lib`

Expected: PASS with no regression in existing join/pivot/unpivot behavior.

- [ ] **Step 5: Commit**

```bash
git add mems/src/ml/expr_df/frame_ops.rs
git commit -m "feat(mems): add strict dataframe explode"
```

### Task 3: Publish and prove the new eig-rs engine contract

**Files:**
- Modify: `/Users/dongshufeng/IdeaProjects/eig-rs/mems/src/bin/tensoreval.rs`
- Modify: `/Users/dongshufeng/IdeaProjects/eig-rs/sparrowzz/dss/docs/catalog/functions.json`
- Modify: `/Users/dongshufeng/IdeaProjects/eig-rs/sparrowzz/dss/docs/rsdss_function_reference.md`
- Modify: `/Users/dongshufeng/IdeaProjects/eig-rs/sparrowzz` gitlink in the eig-rs parent

**Interfaces:**
- Consumes: Tasks 1–2 runtime functions.
- Produces: `functions.json` schema-version-2 entries with arity 2 and searchable documentation; real CLI proof that output is a named Parquet DataFrame.

- [ ] **Step 1: Write a failing CLI test**

Add `expr_df_mode_asserts_and_explodes_named_inputs_to_parquet`. Build named `bus_model` and `bus_voltage` fixtures, execute a multi-step RSS program ending in `phase_voltage_pu`, and assert:

```rust
assert_eq!(names, ["phase_voltage_pu"]);
assert_eq!(result.height(), 3);
assert_eq!(result.get_column_names_str(), ["bus", "phase", "v_pu"]);
```

Also add a companion failure assertion proving a zero/null base or misaligned List does not write any result file.

- [ ] **Step 2: Run the CLI test before catalog/docs changes**

Run: `cargo test -p mems --bin tensoreval expr_df_mode_asserts_and_explodes`

Expected: the success case passes only after Tasks 1–2; catalog/document assertions are still absent.

- [ ] **Step 3: Add exact catalog entries and reference sections**

In `functions.json`, append `assert_all` and `explode` to `frame_functions`, each with `arity: 2`, exact signatures, and failure behavior from the spec. Keep `schema_version: 2`. In the Markdown reference, document full-data/null semantics, strict non-empty lockstep behavior, and the active-bus/result example; do not add `v_pu` to result schemas.

- [ ] **Step 4: Add catalog/runtime consistency assertions**

Extend the existing frame-op catalog test to assert both function names, arities, and runtime support. Run:

`cargo test -p mems --lib frame_function && cargo test -p mems --bin tensoreval expr_df_mode_asserts_and_explodes`

Expected: PASS.

- [ ] **Step 5: Commit the resource submodule without user files**

```bash
git -C sparrowzz add dss/docs/catalog/functions.json dss/docs/rsdss_function_reference.md
git -C sparrowzz commit -m "docs: publish strict analysis functions"
```

Do not stage `dss/docs/catalog/__pycache__/` or `dss/models/.DS_Store`.

- [ ] **Step 6: Commit CLI proof and the exact submodule pointer in eig-rs**

```bash
git add mems/src/bin/tensoreval.rs sparrowzz
git commit -m "test(mems): verify guarded list analysis"
```

### Task 4: Add catalog-driven analysis script preflight

**Files:**
- Create: `backend/src/application/analysis/script_inspector.rs`
- Modify: `backend/src/application/analysis.rs`
- Test: unit tests in `backend/src/application/analysis/script_inspector.rs`

**Interfaces:**
- Consumes: `backend/src/application/managed_rss.rs` tokenization concepts and `EngineResourceSnapshot.functions_json`.
- Produces:

```rust
pub(super) struct AnalysisScriptViolation {
    pub code: &'static str,
    pub fields: Vec<String>,
    pub diagnostic: serde_json::Value,
}

pub(super) fn inspect_analysis_script(
    source: &str,
    input_aliases: &[String],
    expected_outputs: &[String],
    functions: &serde_json::Value,
) -> Result<(), AnalysisScriptViolation>;
```

- [ ] **Step 1: Write failing parser/contract tests**

Add a valid multi-step `select -> join -> assert_all -> explode -> select` case and separate failures for missing semicolon, bare expression, direct alias assignment, input rebinding, wrong terminal outputs, duplicate targets, unknown/wrong-arity functions, and nested `import_df_url`/`ds_*`.

```rust
assert!(inspect_analysis_script(valid, &inputs, &outputs, &catalog).is_ok());
assert_eq!(violation.code, "analysis_script_invalid");
assert_eq!(forbidden.code, "analysis_function_forbidden");
```

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test --features e2e-test analysis_script_inspector`

Expected: FAIL because the module/function does not exist.

- [ ] **Step 3: Implement structural parsing and catalog lookup**

Build the assignment dependency graph needed to identify terminal variables. Read allowed names/aliases/arity from `frame_functions`, `expression_functions_by_arity`, `variable_arity_expression_functions`, and `special_expression_forms`. Explicitly forbid IO/read/import, DSA, and projection families even when catalogued. Validate identifiers against inputs, outputs, and reserved function names.

- [ ] **Step 4: Run focused tests and Clippy for the module**

Run: `cargo test --features e2e-test analysis_script_inspector && cargo clippy --lib --features e2e-test -- -D warnings`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/src/application/analysis.rs backend/src/application/analysis/script_inspector.rs
git commit -m "feat: validate controlled analysis RSS"
```

### Task 5: Freeze active-model sources and prove exact dataset model origin

**Files:**
- Modify: `backend/migrations/00000000000000_initial/up.sql`
- Modify: `backend/src/db/schema.rs`
- Modify: `backend/src/db/models.rs`
- Modify: `backend/src/db/repositories/analysis.rs`
- Modify: `backend/src/db/repositories/result.rs`
- Modify: `backend/src/db/repositories/version.rs`
- Modify: `backend/src/db/repositories/version_deletion.rs`
- Modify: `backend/src/db/repositories/mod.rs`
- Modify: `backend/src/application/analysis.rs`
- Modify: `backend/src/application/analysis/inputs.rs`
- Create: `backend/src/application/analysis/model_origin.rs`
- Modify: `backend/src/application/datasets.rs`
- Create: `backend/tests/result_analysis_sources.rs`

**Interfaces:**
- Consumes: existing owner/session-scoped repositories and immutable model/result/resource artifacts.
- Produces:

```rust
pub enum AnalysisInputSource {
    DatasetId(i32),
    DataResourceId(String),
    ActiveModelDataFrameId(i32),
}

pub enum DatasetModelOrigin {
    Unique(i32),
    None,
    Multiple(Vec<i32>),
    Invalid,
}

pub async fn dataset_model_origin(
    &self,
    user: &UserId,
    session_id: i32,
    dataset_id: i32,
    max_depth: usize,
) -> RepoResult<DatasetModelOrigin>;

pub async fn find_active_version_dataframe(
    &self,
    user: &UserId,
    session_id: i32,
    dataframe_id: i32,
) -> Result<VersionDataFrameReadRow, VersionRepoError>;
```

- [ ] **Step 1: Write failing migration/repository tests**

In `result_analysis_sources.rs`, seed direct run results plus derived lineage. Assert exact outcomes for active direct origin, no origin, two origins, cycle, and depth 65. Add active lookup tests for same-name historical/draft/cross-session/cross-user IDs and an active-switch race. Pin `analysis_model_origin_mismatch` for every non-unique/non-matching mixed source and `analysis_input_not_found` for hidden or deleted sources.

```rust
assert_eq!(origin, DatasetModelOrigin::Unique(active_version_id));
assert!(matches!(none, DatasetModelOrigin::None));
assert!(matches!(multiple, DatasetModelOrigin::Multiple(_)));
assert!(matches!(cycle | too_deep, DatasetModelOrigin::Invalid));
assert_eq!(race_error.code(), "active_model_changed");
```

- [ ] **Step 2: Run the new test target**

Run: `cargo test --test result_analysis_sources --features e2e-test`

Expected: FAIL because model-source columns and origin APIs are absent.

- [ ] **Step 3: Extend the empty-database schema**

Add `engine_resource_snapshot_id` to `result_analysis_runs`. Add `source_kind`, `model_version_id`, and `model_dataframe_id` to both input/lineage snapshot tables; keep dataset/resource/model IDs mutually exclusive, and require model version+DataFrame IDs together for `active_model`. Preserve model IDs as audit values after physical `model_dataframes` deletion rather than adding a cascading FK to that table.

- [ ] **Step 4: Implement owner-scoped active lookup and recursive origin resolution**

Resolve direct run `model_version_id`, direct model lineage, and recursive source datasets up to exactly 64 edges. Detect cycles, depth overflow, no origin, and multiple distinct version IDs. Never use content-hash equality as origin equivalence.

- [ ] **Step 5: Extend input resolution and public lineage projection**

Update `ResolvedAnalysisInputs::resolve` to freeze active version/DataFrame/table/artifact/schema/content hashes and reject mixed active+dataset inputs unless every dataset has the same unique origin. Extend `LineageRow`/`DatasetLineageDto` with `sourceKind`, optional model IDs, and correct deleted/tombstone behavior.

- [ ] **Step 6: Pin version deletion races**

Add tests proving pending/running model-backed analysis blocks version deletion after active switches, while terminal analyses allow the existing version tombstone/delete behavior and derived lineage remains readable.

- [ ] **Step 7: Run tests and commit**

```bash
cargo test --test result_analysis_sources --features e2e-test
cargo test --test version_deletion --features e2e-test
git add backend/migrations/00000000000000_initial/up.sql backend/src/db/schema.rs backend/src/db/models.rs backend/src/db/repositories backend/src/application/analysis.rs backend/src/application/analysis backend/src/application/datasets.rs backend/tests/result_analysis_sources.rs backend/tests/version_deletion.rs
git commit -m "feat: freeze active model analysis inputs"
```

### Task 6: Integrate preflight, sanitized repair diagnostics, and atomic analysis execution

**Files:**
- Modify: `backend/src/application/analysis.rs`
- Create: `backend/src/application/analysis/diagnostics.rs`
- Modify: `backend/src/application/analysis/outputs.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/tools/support.rs`
- Test: `backend/tests/result_analysis_sources.rs`
- Test: `backend/tests/tool_manifest.rs`

**Interfaces:**
- Consumes: Task 4 inspector, Task 5 resolved inputs/origins, `application::diagnostics::sanitize_adapter_stderr`, and `EngineResourceSnapshot`.
- Produces: zero-write preflight failures, snapshot-bound analysis rows, repairable structured diagnostics, and unchanged atomic Parquet publication.

- [ ] **Step 1: Write failing service tests**

Add tests proving an invalid/forbidden script creates zero analysis rows and zero script artifacts; a repairable parser/type/output failure creates one failed run with sanitized bounded details; a path/monitor/quota failure is marked non-repairable; and a valid active-model+dataset+resource analysis publishes one output with all three lineage entries.

```rust
assert_eq!(analysis_run_count(), 0);
assert_eq!(script_artifact_count(), 0);
assert!(!feedback.contains("/private/secret"));
assert!(feedback.len() <= 64 * 1024 + 1024);
assert_eq!(lineage_kinds, ["active_model", "dataset", "data_resource"]);
```

- [ ] **Step 2: Run focused tests**

Run: `cargo test --test result_analysis_sources --features e2e-test analysis_service`

Expected: FAIL before orchestration changes.

- [ ] **Step 3: Inject the current snapshot and run preflight before writes**

Extend `AnalysisService::new` with `Arc<EngineResourceSnapshot>` and `VersionRepo`. In `run`, order operations as request shape → owner/source/origin resolution → `inspect_analysis_script` → scope/script commit → analysis row → staging/adapter. Store exact snapshot ID and engine contract on the run.

- [ ] **Step 4: Add structured, sanitized diagnostic mapping**

Create a private mapping from `AdapterError`/output-contract failures to `{code, repairable, stage, diagnostic}`. Map repairable engine diagnostics to `analysis_engine_failed`, output mismatches to `analysis_result_contract_invalid`, and retain existing non-repairable infrastructure codes. Reuse existing stream/path/secret sanitization and cap model feedback at 64 KiB. Extend `DomainToolError` with optional details while preserving `DomainToolError::new` for all existing tools.

- [ ] **Step 5: Preserve execution semantics**

Keep no timeout/cancel endpoint, private staging cleanup, full result scanning, exact expected output names, and all-or-nothing publication. Map output-name mismatch to `analysis_result_contract_invalid`; do not classify monitor/path/quota/storage failures as repairable.

- [ ] **Step 6: Run service and regression tests**

Run: `cargo test --test result_analysis_sources --features e2e-test && cargo test --test result_deletion --features e2e-test && cargo test --test data_resources --features e2e-test`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add backend/src/application/analysis.rs backend/src/application/analysis backend/src/application/mod.rs backend/src/tools/support.rs backend/tests/result_analysis_sources.rs
git commit -m "feat: preflight and diagnose direct analyses"
```

### Task 7: Add bounded source discovery and the three-way analysis tool contract

**Files:**
- Create: `backend/src/tools/read_result_dataset.rs`
- Modify: `backend/src/tools/mod.rs`
- Modify: `backend/src/tools/inspect_workspace.rs`
- Modify: `backend/src/tools/read_model_dataframe.rs`
- Modify: `backend/src/tools/run_result_analysis.rs`
- Modify: `backend/src/application/datasets.rs`
- Modify: `backend/tests/tool_manifest.rs`
- Modify: `backend/tests/workspace_tools.rs`

**Interfaces:**
- Consumes: Task 5 `ActiveModelDataFrameId`, existing DatasetService metadata/page methods, current skill state.
- Produces: direct `ReadResultDatasetTool`; `modelDataframeId|datasetId|resourceId` one-of inputs; active-only result-analysis model reads.

- [ ] **Step 1: Write failing direct tool tests**

Test `read_result_dataset` owner/session isolation, max 100 rows/32 columns and cursor projection. Test `inspect_workspace.active.tables[].modelDataframeId`. Test active reads after loading either skill and draft reads only after `model-intake`. Add `run_result_analysis` schema/behavior cases for each single source, all three mixed sources, zero/two source fields in one input, 21 inputs, duplicate/reserved aliases, stale active IDs, and frozen schema/content hashes.

```rust
assert_eq!(read["metadata"]["id"], dataset_id.to_string());
assert_eq!(workspace["active"]["tables"][0]["modelDataframeId"], frame_id);
assert_eq!(draft_error.code, "skill_required");
```

- [ ] **Step 2: Run focused tool tests**

Run: `cargo test --test tool_manifest --features e2e-test read_result_dataset && cargo test --test workspace_tools --features e2e-test read_model_dataframe`

Expected: FAIL because the tool/fields/gates do not exist.

- [ ] **Step 3: Implement bounded result reads**

Define `read_result_dataset::{Args, ReadResultDatasetTool}` with `datasetId`, optional `columns`, `limit`, and `cursor`. Return `{metadata,page}` only when `DatasetService::session_id` equals the current tool session. Do not register it in the runtime catalog until Task 8 updates the manifest atomically.

- [ ] **Step 4: Implement active discovery and skill gating**

Expose stable model DataFrame IDs in active/draft workspace summaries. Add `ToolServices::require_any_skill`; permit `scope=active` after `result-analysis` or `model-intake`, but keep `scope=draft` strictly `model-intake` plus exact revision.

- [ ] **Step 5: Extend `run_result_analysis` input parsing**

Add optional `model_dataframe_id` serialized as `modelDataframeId`; require exactly one of the three source fields and map it to `ActiveModelDataFrameId`. Keep 1..=20 total inputs, distinct aliases, `intent` string semantics, and required `expectedOutputs`.

- [ ] **Step 6: Run tests and commit**

```bash
cargo test --test tool_manifest --features e2e-test read_result_dataset
cargo test --test workspace_tools --features e2e-test read_model_dataframe
git add backend/src/tools backend/src/application/datasets.rs backend/tests/tool_manifest.rs backend/tests/workspace_tools.rs
git commit -m "feat: expose bounded analysis sources"
```

### Task 8: Enforce repair budgets and publish Agent bundle v4.5.0

**Files:**
- Modify: `backend/src/application/chat.rs`
- Modify: `backend/src/tools/mod.rs`
- Modify: `backend/src/tools/run_result_analysis.rs`
- Modify: `backend/resources/agent/v4/system.md`
- Modify: `backend/resources/agent/v4/skills/result-analysis.md`
- Modify: `backend/resources/agent/v4/skill-index.json`
- Modify: `backend/resources/agent/v4/manifest.json`
- Modify: `backend/tests/fixtures/engine_resources/catalog/functions.json`
- Modify: `backend/tests/agent_knowledge.rs`
- Modify: `backend/tests/agent_factory.rs`
- Modify: `backend/tests/tool_manifest.rs`
- Modify: `backend/tests/scripted_agent.rs`

**Interfaces:**
- Consumes: Task 7 tool implementation and three-way input schema; Tasks 1–3 current engine docs/catalog.
- Produces: request-local four-call claim, `MAX_TURNS = 12`, 24 registered tools, `v4.5.0`, and `result-analysis@1.3.0` direct-author instructions.

- [ ] **Step 1: Write failing request-budget and routing tests**

Add a direct tool test that calls valid normalized analysis arguments five times and asserts only four reach `AnalysisService`. Add scripted scenarios proving Chinese analysis does not load `model-intake`, uses a uniquely identified source without asking, asks one necessary question for a materially ambiguous source, loads exact docs only when needed, repairs once from a sanitized repairable error, stops after a non-repairable error, and reports budget exhaustion without a fifth engine call.

```rust
assert_eq!(engine.call_count(), 4);
assert_eq!(fifth.code, "analysis_repair_budget_exhausted");
assert!(!tool_names.contains(&"model-intake"));
```

- [ ] **Step 2: Run tests and verify current 8-turn/23-tool failures**

Run: `cargo test --test tool_manifest --features e2e-test analysis_repair_budget && cargo test --test scripted_agent --features e2e-test direct_analysis`

Expected: FAIL.

- [ ] **Step 3: Add request-local attempt claims and turn-limit copy**

Add `analysis_attempts: Arc<Mutex<u8>>` to every `ToolServices` constructor and `claim_analysis_attempt(max: u8)`. Claim after intent/input normalization and immediately before `AnalysisService::run`. Set `MAX_TURNS` to 12 and replace the research-specific max-turn message with a generic unfinished-operation message.

- [ ] **Step 4: Register the 24th tool and update direct-author policy**

Register `read_result_dataset`. Rewrite the system first line to inspect sources only when required. Update result-analysis triggers, required tools, active-only rules, exact model-origin behavior, `assert_all`/`explode`, direct execution, no confirmation, and repair policy. Keep model publication/study confirmation/chart restrictions unchanged.

- [ ] **Step 5: Bump versions and recompute all hashes**

Set bundle `v4.5.0` and skill `1.3.0`. Recompute each changed skill SHA-256, then `skill-index.json`, then `system.md`; write the exact lowercase hashes into `skill-index.json` and `manifest.json`. Keep `engineContractVersion: tensoreval-parquet-v3`.

Use: `shasum -a 256 backend/resources/agent/v4/system.md backend/resources/agent/v4/skill-index.json backend/resources/agent/v4/skills/result-analysis.md`

- [ ] **Step 6: Run bundle, tool, and scripted behavior tests**

Run: `cargo test --test agent_knowledge --features e2e-test && cargo test --test agent_factory --features e2e-test && cargo test --test tool_manifest --features e2e-test && cargo test --test scripted_agent --features e2e-test`

Expected: PASS with 24 unique tools, no hidden implementation, and the direct-analysis trace.

- [ ] **Step 7: Commit**

```bash
git add backend/src/application/chat.rs backend/src/tools backend/resources/agent/v4 backend/tests/fixtures/engine_resources/catalog/functions.json backend/tests/agent_knowledge.rs backend/tests/agent_factory.rs backend/tests/tool_manifest.rs backend/tests/scripted_agent.rs
git commit -m "feat: let conversation agent author analyses"
```

### Task 9: Expose immutable analysis script and source lineage in the UI

**Files:**
- Modify: `backend/src/application/analysis.rs`
- Modify: `backend/src/db/repositories/analysis.rs`
- Modify: `backend/src/api/analysis.rs`
- Create: `backend/tests/analysis_api.rs`
- Modify: `frontend/src/types/run.ts`
- Modify: `frontend/src/types/dataset.ts`
- Modify: `frontend/src/api/client.ts`
- Create: `frontend/src/components/messages/AnalysisDetails.tsx`
- Create: `frontend/src/components/messages/AnalysisDetails.test.tsx`
- Modify: `frontend/src/components/messages/ResultCard.tsx`
- Create: `frontend/src/components/messages/ResultCard.test.tsx`

**Interfaces:**
- Consumes: Task 5 input snapshots/lineage and Task 6 script artifact/snapshot identity.
- Produces: additive `AnalysisRun.script`, `AnalysisRun.inputs`, `engineResourceSnapshotId`, and a lazy read-only detail panel.

- [ ] **Step 1: Write failing API projection tests**

Assert `GET /api/analysis-runs/{id}` returns full script content/hash, engine snapshot/contract, ordered inputs for all three source kinds, and no physical path; cross-user remains 404.

```rust
assert_eq!(body["script"]["content"], executed_script);
assert_eq!(body["inputs"][0]["sourceKind"], "active_model");
assert!(!body.to_string().contains(data_dir));
```

- [ ] **Step 2: Implement application/repository details reads**

Add an owner-scoped `AnalysisRepo::inputs` read and `AnalysisService::details` that resolves the owned script artifact, verifies its hash, reads bounded-by-existing-2-MiB UTF-8 content, and returns frozen inputs without consulting live sources.

- [ ] **Step 3: Extend the existing GET DTO and Rustdoc**

Return additive `script: {content,contentHash}`, `inputs`, and `engineResourceSnapshotId`; keep existing status/intent/error/datasets and route unchanged.

- [ ] **Step 4: Write failing frontend lazy-load tests**

Test that simulation result cards have no analysis button, analysis cards do not fetch until “查看分析脚本与输入”, all three frozen source kinds render in ordinal order, loading/error/retry are isolated, unmount/ID change ignores late responses, and the panel contains no edit/run/download action.

- [ ] **Step 5: Implement DTOs and `AnalysisDetails`**

Use a discriminated lineage/input source union in `types/dataset.ts`/`types/run.ts`. `AnalysisDetails` calls existing `getAnalysis` only after expansion, renders a read-only RSS code block plus ordered source summaries, and uses `notify` for controlled failures.

- [ ] **Step 6: Run backend/frontend focused tests**

```bash
cd backend && cargo test --test analysis_api --features e2e-test
cd ../frontend && npm test -- --run src/components/messages/AnalysisDetails.test.tsx src/components/messages/ResultCard.test.tsx src/api/client.test.ts
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add backend/src/application/analysis.rs backend/src/db/repositories/analysis.rs backend/src/api/analysis.rs backend/tests frontend/src/types frontend/src/api/client.ts frontend/src/components/messages/AnalysisDetails.tsx frontend/src/components/messages/AnalysisDetails.test.tsx frontend/src/components/messages/ResultCard.tsx frontend/src/components/messages/ResultCard.test.tsx
git commit -m "feat: show analysis scripts and lineage"
```

### Task 10: Complete behavior coverage, release evidence, docs, and full verification

**Files:**
- Modify: `integration-tests/e2e/resilience.spec.ts`
- Modify: `integration-tests/release/real-dependencies.spec.ts`
- Modify: `integration-tests/release/semantic-assertions.ts`
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `docs/development-status.md`
- Modify: `docs/api-design.md`
- Modify: `docs/db-design.md`
- Modify: `docs/frontend-design.md`
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/v4-deployment.md`
- Modify: `docs/integration-testing-design.md`

**Interfaces:**
- Consumes: all prior tasks and the current eig-rs binary/resource snapshot.
- Produces: deterministic browser evidence, real LLM/tensoreval evidence, complete feature traceability, and current documentation.

- [ ] **Step 1: Extend deterministic browser coverage**

In the existing analysis recovery journey, seed active `bus` plus a same-version `bus_voltage`, script the Agent through `result-analysis` and direct `run_result_analysis`, then verify the output opens and “查看分析脚本与输入” survives refresh. Add a mismatch scenario proving a result from a different model version creates no analysis run.

- [ ] **Step 2: Run the focused Playwright journey**

Run from `frontend`: `npm run test:e2e -- --grep "analysis"`

Expected: PASS and route evidence for the existing analysis GET route.

- [ ] **Step 3: Upgrade the real dependency journey**

Replace/extend the copy-only analysis prompt with the approved per-phase p.u. journey over actual active `bus` and actual `bus_voltage`. Assert trace contains `load_agent_skill`, only necessary read/doc tools, and `run_result_analysis`; explicitly reject `model-intake`, modification, publication, study-confirmation, and script/run tools. Page through all derived rows and verify `time,bus,phase,v_pu`, exact active-model/dataset lineage, script presence, finite values, and full expected row count.

- [ ] **Step 4: Update functional catalog and current docs**

Add/adjust behavioral evidence for the 24th tool, changed tool schema, bundle/skill versions, additive analysis DTO/lineage, stable errors, active-only/matching-origin rules, full-data assertions, strict explode, four-attempt budget, 12 turns, and unchanged no-timeout/2-GiB/8-GiB limits. Do not update route count because no route is added.

- [ ] **Step 5: Run eig-rs full relevant gates**

```bash
cd /Users/dongshufeng/IdeaProjects/eig-rs
cargo fmt --all -- --check
cargo clippy -p mems --all-targets --all-features -- -D warnings
cargo test -p mems
```

Expected: all PASS; record the exact eig-rs and sparrowzz commits used.

- [ ] **Step 6: Run zjudss deterministic gates**

```bash
cd /Users/dongshufeng/IdeaProjects/zjudss/backend
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --features e2e-test
cd ../frontend
npm ci
npm test -- --run
npm run build
npm run test:e2e
cd ..
./scripts/check-functional-coverage --require-pr
git diff --check
```

Expected: every command PASS; no skipped/ignored/manual-only substitute for changed behavior.

- [ ] **Step 7: Run the real dependency release gate**

With approved provider secrets and the just-built current eig-rs binary/resources:

```bash
./scripts/test-release-integration
./scripts/check-functional-coverage --require-release
```

Expected: real direct-analysis segment succeeds, all projector/representative gates remain green, and the report records `v4.5.0`, `result-analysis@1.3.0`, current binary hash, and new snapshot ID without paths, script text, or secrets.

- [ ] **Step 8: Commit traceability and documentation**

```bash
git add integration-tests docs/development-status.md docs/api-design.md docs/db-design.md docs/frontend-design.md docs/agent-prompt-skills-design.md docs/v4-deployment.md docs/integration-testing-design.md
git commit -m "docs: publish direct analysis contract"
```

- [ ] **Step 9: Record final repository state**

Run `git status --short --branch` in zjudss, eig-rs, and `eig-rs/sparrowzz`; confirm zjudss contains only intended commits and all pre-existing unrelated eig-rs/submodule files remain untracked or modified exactly as before.
