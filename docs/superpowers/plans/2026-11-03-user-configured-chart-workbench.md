# User-Configured Chart Workbench Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Agent-generated result views with a user-configured, persisted chart workbench that compiles bounded DataFrame selections into typed chart models and safe ECharts rendering.

**Architecture:** Add a Rust `charting` boundary that owns the public configuration/model contracts, Arrow/List extraction, six deterministic chart compilers, and Snappy Plot Data materialization. Publish immutable chart views through user-scoped SQLite/CAS transactions and seven HTTP routes; React then provides a fixed-template builder, saved-chart workbench, and a typed `ChartModel -> EChartsOption` adapter while the Agent loses every chart-creation path.

**Tech Stack:** Rust, Axum, Diesel/SQLite, Arrow/Parquet with Snappy, React 19, TypeScript, ECharts 6, Vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-11-03-user-configured-chart-workbench-design.md`

## Global Constraints

- Work directly on `master`; do not create a feature worktree. Preserve and exclude unrelated user changes from every commit, especially currently modified docs/tests; stage only task-owned hunks and inspect `git diff --cached` before committing.
- All repository reads explicitly accept `UserId`; missing, foreign, and ordinary tombstoned resources all return the same 404.
- Public IDs are strings. No browser or LLM boundary receives a path, command, SQL, RSS, artifact ID, arbitrary ECharts option, formatter, function, or remote URL.
- Plot Data is an immutable Snappy Parquet artifact. Saving a chart must atomically publish the storage-scope reference and `dataset_views` row; deletion/session cleanup must atomically remove the business reference and release exactly one logical artifact reference.
- Opening a result still defaults to the table. Preview is read-only; only explicit “保存图表” publishes a chart. Saved charts are immutable and can only be copied into a new configuration.
- The only chart kinds are `line|bar|scatter|heatmap|boxplot|histogram`; all public configuration, style, budgets, deterministic data semantics, and palette constants come verbatim from the spec.
- V4 remains empty-database-only: edit `backend/migrations/00000000000000_initial`, do not add compatibility migrations, and document that SQLite plus DATA_DIR must be reset.
- Remove all LLM chart authority: `create_dataset_view`, `initialViews`, `AnalysisRequest.initial_views`, automatic view creation, and resource-view creation must not remain callable or compilable.
- The Agent bundle moves from `v4.3.0` to `v4.4.0`; the route manifest moves from 75 to 82 routes and the runtime tool registry from 24 to 23 tools.
- Any route, DTO, view kind behavior, stable error, tool, skill, or frontend workbench change must update the current design docs, `integration-tests/catalog/features.yaml`, deterministic behavior tests, and 100% PR functional coverage.

## Review Focus

- A variable nested List is truncated by capability discovery, has missing component paths, or is entirely missing: disable oversized component selectors, count field accesses separately from dropped points, and return `chart_no_data` only after authoritative full compilation (Tasks 2 and 3 tests).
- A source is deleted during save or before a saved chart is reopened: return 409 only for the verified race, keep ordinary tombstones 404, and open a persisted chart by view ID without requesting source metadata (Tasks 5, 6, and 10 tests).
- Int64/UInt64 category values exceed JavaScript's safe integer range or floats contain NaN/Inf: preserve integers as tagged decimal strings, reject unsafe value axes, and never emit a non-finite JSON number (Tasks 1–3 tests).
- Concurrent saves reuse an operation ID, change the payload, collide by name, or replay after tombstone: converge identical calls, check replay before name conflicts, reject changed payloads, and require a new ID after deletion (Tasks 5 and 6 tests).
- A late preview or desktop-right legend response arrives after dataset/session/layout changes: ignore stale responses, keep the current builder, and downgrade the legend on 390px without hiding it or overflowing (Tasks 9 and 10 tests).

---

### Task 1: Freeze the chart contracts, normalization, and errors

**Files:**
- Create: `backend/src/application/charting/mod.rs`
- Create: `backend/src/application/charting/contracts.rs`
- Create: `backend/src/application/charting/error.rs`
- Modify: `backend/src/application/mod.rs`
- Test: `backend/src/application/charting/contracts.rs`
- Include in commit: `docs/superpowers/specs/2026-11-03-user-configured-chart-workbench-design.md`
- Include in commit: `docs/superpowers/plans/2026-11-03-user-configured-chart-workbench.md`

**Interfaces:**
- Consumes: `DataFrameColumn`, `serde`, `serde_json`, and the exact unions/limits in spec §§5, 6.3, and 11.
- Produces: `ChartConfig`, `ChartPresentation`, `LineChartPresentation`, `ChartFieldRef`, `ChartCapabilities`, `ChartModel`, `ChartDiagnostics`, `ChartBudgets`, `NormalizedCreateChart`, `ChartError`, `normalize_create_request(name, config)`, and `VISUALIZATION_KINDS` for all later tasks.

- [ ] **Step 1: Write failing serde and normalization tests**

  Add unit tests named `six_chart_configs_reject_unknown_or_cross_kind_fields`,
  `presentation_accepts_only_the_frozen_palette_and_line_style_enums`,
  `normalization_trims_text_expands_labels_and_produces_stable_hashes`, and
  `category_values_preserve_large_integer_identity`. Assert all six exact config shapes, required presentation fields, line-only
  sampling/style, omitted optional title/splitBy, `mode=none` rejecting `maxPointsPerSeries`, duplicate normalized series labels,
  `2^63-1` as `{kind:"integer",value:"9223372036854775807"}`, and identical canonical/request hashes for semantically identical JSON.

- [ ] **Step 2: Run the contract tests and verify red**

  Run: `cd backend && cargo test --lib application::charting::contracts`

  Expected: FAIL because `application::charting` does not exist.

- [ ] **Step 3: Define the public and persisted contract types**

  Implement the spec's types as `#[serde(tag = "kind", deny_unknown_fields, rename_all = "camelCase")]` enums/structs. Define:

  ```rust
  pub fn normalize_create_request(
      name: String,
      config: ChartConfig,
  ) -> Result<NormalizedCreateChart, ChartError>;

  pub struct NormalizedCreateChart {
      pub name: String,
      pub config: ChartConfig,
      pub config_json: String,
      pub config_hash: String,
      pub request_hash: String,
  }
  ```

  Use canonical `chart-model-v1 + name + config` hashing, exact Unicode-whitespace trimming, generated `column[path]` labels,
  the frozen palettes/style defaults, and the exact 100/200/64-character and array/range constraints. Keep `ChartError` code,
  message, and bounded public details; do not reuse the old `VisualizationError`.

- [ ] **Step 4: Export the contract boundary**

  Add `pub mod charting` and re-export only the DTOs/services needed outside the module. Move the authoritative
  `VISUALIZATION_KINDS` constant to `charting`; leave the old visualization module temporarily compiling until Task 7.

- [ ] **Step 5: Run focused tests and formatting**

  Run: `cd backend && cargo fmt --all -- --check && cargo test --lib application::charting::contracts`

  Expected: PASS with all six discriminated configs and style enums covered.

- [ ] **Step 6: Commit the contract boundary**

  ```bash
  git add backend/src/application/charting backend/src/application/mod.rs docs/superpowers/specs/2026-11-03-user-configured-chart-workbench-design.md docs/superpowers/plans/2026-11-03-user-configured-chart-workbench.md
  git diff --cached --check
  git commit -m "feat: define controlled chart contracts"
  ```

### Task 2: Discover bounded chart capabilities and resolve scalar/List fields

**Files:**
- Create: `backend/src/application/charting/capabilities.rs`
- Create: `backend/src/application/charting/fields.rs`
- Modify: `backend/src/application/charting/mod.rs`
- Modify: `backend/src/application/dataframe_reader.rs`
- Test: `backend/src/application/charting/capabilities.rs`
- Test: `backend/src/application/charting/fields.rs`

**Interfaces:**
- Consumes: Task 1 `ChartFieldRef`, `ChartCapabilities`, `ChartBudgets`, `CategoryValue`, `ChartError`; existing projected Parquet scans.
- Produces: `CapabilityInspector::inspect(schema, batches, budgets)`, `ResolvedField`, `FieldValue`, and `resolve_field(array,row,field_ref)` for compiler tasks.

- [ ] **Step 1: Write failing capability and field-resolution tests**

  Cover scalar string/bool/signed/unsigned/float roles; `FixedSizeList`, `List`, `LargeList`, ListView and four-level numeric
  paths; a variable three-phase list; a missing path; null and non-finite leaves; unsafe Int64/UInt64 value-axis conversion;
  10,000-row incomplete discovery; 64-path field truncation; 256-column and 1 MiB capability limits. Assert truncated List
  fields are disabled instead of silently exposing a prefix.

- [ ] **Step 2: Run focused tests and verify red**

  Run: `cd backend && cargo test --lib application::charting::capabilities application::charting::fields`

  Expected: FAIL because inspectors/resolvers are absent.

- [ ] **Step 3: Implement physical type classification and List discovery**

  Add:

  ```rust
  impl CapabilityInspector {
      pub fn inspect(
          columns: &[DataFrameColumn],
          batches: &[RecordBatch],
          total_rows: u64,
          budgets: ChartBudgets,
      ) -> Result<ChartCapabilities, ChartError>;
  }
  ```

  Inspect at most `CHART_CAPABILITY_SCAN_ROWS=10_000`, mark `complete` accurately, bound fields/paths/serialized bytes, and
  emit only the roles/templates permitted by the spec compatibility table. Do not infer time from names.

- [ ] **Step 4: Implement exact scalar and nested component extraction**

  Define `FieldValue::{Category(CategoryValue),Number(f64),Missing,MissingComponent,NonFinite}` and resolve only direct columns
  plus numeric index paths of depth `1..=4`. Keep big integers tagged for category use and reject precision-losing value-axis
  conversion. One field access returns one classification; diagnostics are accumulated by the compiler, not inside Arrow arrays.

- [ ] **Step 5: Add a bounded batch collector to the shared reader**

  Add a crate-private helper that scans a narrow projection into batches while enforcing the chart input-row limit; preserve
  the existing visitor API and all dataset/model callers. Do not add public row endpoints or physical paths.

- [ ] **Step 6: Verify capability, reader, and complex-value regressions**

  Run: `cd backend && cargo test --lib application::charting && cargo test --lib dataframe_reader`

  Expected: PASS; existing table JSON handling remains unchanged.

- [ ] **Step 7: Commit capability discovery**

  ```bash
  git add backend/src/application/charting backend/src/application/dataframe_reader.rs
  git diff --cached --check
  git commit -m "feat: inspect chartable dataframe fields"
  ```

### Task 3: Compile line, bar, and scatter models deterministically

**Files:**
- Create: `backend/src/application/charting/compiler.rs`
- Create: `backend/src/application/charting/sampling.rs`
- Modify: `backend/src/application/charting/mod.rs`
- Test: `backend/src/application/charting/compiler.rs`
- Test: `backend/src/application/charting/sampling.rs`

**Interfaces:**
- Consumes: Tasks 1–2 contracts/resolved fields and projected `RecordBatch` values.
- Produces: `ChartCompiler::compile(config, source) -> CompiledChart`, `CompiledChart { model, plot_data, plot_schema, diagnostics }`, and deterministic LTTB used by persistence/API tasks.

- [ ] **Step 1: Write failing line/bar/scatter compiler tests**

  Add tests for three `v_mag[0..2]` lines over `bus`; value-axis line ordering; source-order duplicate X retention; explicit
  mean/min/max/sum aggregation; splitBy series order; series-label collision; unit conflict; bar duplicate rejection before
  aggregation; scatter duplicate preservation; null/non-finite/missing-component counters; all-missing `chart_no_data`; 100-series
  and 50,000-point boundaries; and LTTB only for value-axis line.

- [ ] **Step 2: Run compiler tests and verify red**

  Run: `cd backend && cargo test --lib application::charting::compiler application::charting::sampling`

  Expected: FAIL because the compiler does not exist.

- [ ] **Step 3: Implement candidate extraction and diagnostics accounting**

  Implement one scan that assigns source ordinal, expands each measure/split combination, and counts `inputRows`,
  `candidatePoints`, missing component field accesses, dropped candidate points, and non-finite candidate points exactly as spec
  §6.2 defines. A candidate with multiple missing fields increments `droppedMissing` once.

- [ ] **Step 4: Implement line/bar/scatter grouping and ordering**

  Use typed category keys so string `"1"`, integer `1`, and float `1.0` do not collide. Apply stable source/x sorting, series
  naming, duplicate rules, aggregation, unit checks, and final point/category/response budgets. Populate the exact Task 1 model
  variants, including axis/value display names from `ChartFieldRef.label`.

- [ ] **Step 5: Implement and freeze LTTB**

  Add `lttb(points: &[NumericPoint], threshold: usize) -> Vec<NumericPoint>` for sorted value-axis lines only. Sample per series,
  keep first/last points, preserve ordinal ties, and define `sampledPoints` as post-aggregation pre-sampling points removed.

- [ ] **Step 6: Verify deterministic compilers**

  Run: `cd backend && cargo test --lib application::charting::compiler application::charting::sampling`

  Expected: PASS with exact diagnostics and stable series/category order.

- [ ] **Step 7: Commit scalar chart compilation**

  ```bash
  git add backend/src/application/charting
  git diff --cached --check
  git commit -m "feat: compile scalar result charts"
  ```

### Task 4: Compile heatmap, boxplot, histogram, and Snappy Plot Data

**Files:**
- Create: `backend/src/application/charting/plot_data.rs`
- Modify: `backend/src/application/charting/compiler.rs`
- Modify: `backend/src/application/charting/mod.rs`
- Test: `backend/src/application/charting/compiler.rs`
- Test: `backend/src/application/charting/plot_data.rs`

**Interfaces:**
- Consumes: Task 3 `CompiledChart` and Task 1 model contracts.
- Produces: all six chart variants plus `PlotDataCodec::{write_snappy,read_model}` and `MaterializedPlot { staged, schema_json, schema_hash, content_hash, row_count, point_count, size_bytes }`.

- [ ] **Step 1: Write failing heatmap/boxplot/histogram tests**

  Assert heatmap mean/min/max/sum/count and duplicate-cell semantics; stable tagged category axes; `count` using non-null x/y/value;
  R-7 quartiles, Tukey 1.5-IQR whiskers and optional outliers; single-value groups; equal-value histogram single bin; normal bins
  `[start,end)` with a closed final bin; and empty/all-invalid errors.

- [ ] **Step 2: Write failing Plot Data round-trip tests**

  For every chart kind, compile a fixture, write Snappy Parquet, inspect every column chunk compression, read the artifact back,
  and assert the reconstructed `ChartModel` (including tagged categories, presentation, diagnostics, config hash, and ordinals)
  exactly equals the preview model. Assert only the spec's narrow Plot Data columns are written.

- [ ] **Step 3: Run focused tests and verify red**

  Run: `cd backend && cargo test --lib application::charting::compiler application::charting::plot_data`

  Expected: FAIL because the three compilers and Plot Data codec are incomplete.

- [ ] **Step 4: Implement the remaining deterministic compilers**

  Add heatmap keyed reducers, boxplot R-7/Tukey computation, and histogram boundary logic. Apply the exact category/cell/raw-value
  budgets before allocating final vectors.

- [ ] **Step 5: Implement per-kind Plot Data schemas and round-trip decoding**

  Use ArrowWriter with `Compression::SNAPPY`; store tagged category values as canonical JSON text and finite values in numeric
  columns. Reconstruct models exclusively from Plot Data plus frozen metadata, never from the source dataset.

- [ ] **Step 6: Verify all charting unit tests**

  Run: `cd backend && cargo test --lib application::charting`

  Expected: PASS for six compilers, Plot Data equality, Snappy compression, and budget boundaries.

- [ ] **Step 7: Commit full Plot Data compilation**

  ```bash
  git add backend/src/application/charting
  git diff --cached --check
  git commit -m "feat: materialize typed chart plot data"
  ```

### Task 5: Replace dataset view persistence and make artifact lifecycle atomic

**Files:**
- Modify: `backend/migrations/00000000000000_initial/up.sql`
- Modify: `backend/migrations/00000000000000_initial/down.sql`
- Modify: `backend/src/db/schema.rs`
- Modify: `backend/src/db/models.rs`
- Create: `backend/src/db/repositories/chart_view.rs`
- Modify: `backend/src/db/repositories/mod.rs`
- Modify: `backend/src/db/repositories/artifact.rs`
- Modify: `backend/src/db/repositories/session.rs`
- Modify: `backend/src/application/artifacts.rs`
- Modify: `backend/src/application/datasets.rs`
- Modify: `backend/src/application/events.rs`
- Create: `backend/tests/chart_view_repository.rs`
- Create: `backend/tests/session_deletion.rs`

**Interfaces:**
- Consumes: Task 4 `MaterializedPlot`, existing artifact publication lock/refcount/GC, current result ownership joins.
- Produces: `ChartViewRepo::{publish,find,list,delete}`, `ChartViewRow`, `ChartViewSummaryRow`, `NewChartView`, atomic artifact helpers, and unified `{id,name,kind,createdAt}` view summaries.

- [ ] **Step 1: Write failing migration/repository behavior tests**

  Test exact initial schema columns/checks/indexes; same-user creation; foreign/missing/tombstoned source 404; source/session mismatch;
  identical and conflicting operation replay; replay-before-name-conflict; concurrent identical save convergence; deleted-operation
  `chart_operation_gone`; exact-name case sensitivity and reuse after deletion; source tombstone survival; cursor order; cross-user 404;
  view tombstone; and `chartCount/chartBytes` in session deletion preview/impact.

- [ ] **Step 2: Write failing atomic artifact tests**

  Assert publication inserts storage-scope refcount plus view in one immediate transaction; injected finalize failure rolls back the
  logical reference; blob-without-DB crash debris is collected as an orphan; view deletion/session cleanup tombstones/releases one
  reference per view even when hashes deduplicate; retry does not double-release.

- [ ] **Step 3: Run repository tests and verify red**

  Run: `cd backend && cargo test --test chart_view_repository && cargo test --test session_deletion`

  Expected: FAIL against the old `view_dataset_id/spec_json/is_default` schema.

- [ ] **Step 4: Replace the empty-database schema and Diesel models**

  Implement the spec §8 table exactly, including `session_id`, `artifact_id`, request/config hashes, contract/diagnostics, Plot
  Data schema/content hashes, source snapshots, live/tombstone check, operation uniqueness and partial live-name uniqueness. Remove
  `view_dataset_id`, `spec_json`, and `is_default`. Regenerate/edit Diesel schema and models consistently.

- [ ] **Step 5: Add transaction-local artifact reference helpers**

  Expose crate-private `commit_draft_in_transaction`/`release_ref_in_transaction` entry points that existing artifact methods and
  `ChartViewRepo` share. Keep blob publication under the existing access gate. Do not duplicate quota/refcount arithmetic in the
  chart repository.

- [ ] **Step 6: Implement user-scoped chart view repository operations**

  Define repository signatures that accept `&UserId` and never client session/scope IDs. `publish` must run replay lookup first,
  verify the source/session, finalize the artifact and insert the view in one transaction. `find/list` must remain readable after
  source tombstone; `delete` must tombstone and release in one transaction.

- [ ] **Step 7: Unify event/dataset view summaries and session impacts**

  Replace `defaultViewId` and `isDefault` everywhere with `{id,name,kind,createdAt}`. Extend session deletion counts, bytes, impact
  hash and cleanup to chart views; preserve all other result/analysis lineage behavior.

- [ ] **Step 8: Verify persistence and deletion regressions**

  Run: `cd backend && cargo test --test chart_view_repository && cargo test --test session_deletion && cargo test --test result_deletion`

  Expected: PASS with source deletion preserving saved charts and no leaked logical references.

- [ ] **Step 9: Commit persistence replacement**

  ```bash
  git add backend/migrations/00000000000000_initial/up.sql backend/migrations/00000000000000_initial/down.sql backend/src/db/schema.rs backend/src/db/models.rs backend/src/db/repositories/chart_view.rs backend/src/db/repositories/mod.rs backend/src/db/repositories/artifact.rs backend/src/db/repositories/session.rs backend/src/application/artifacts.rs backend/src/application/datasets.rs backend/src/application/events.rs backend/tests/chart_view_repository.rs backend/tests/session_deletion.rs backend/tests/result_deletion.rs
  git diff --cached --check
  git commit -m "feat: persist immutable chart views"
  ```

### Task 6: Expose capabilities, preview, save, read, list, and delete APIs

**Files:**
- Create: `backend/src/application/charting/service.rs`
- Modify: `backend/src/application/charting/mod.rs`
- Create: `backend/src/api/chart_views.rs`
- Modify: `backend/src/api/mod.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/contract_manifest.rs`
- Create: `backend/tests/chart_views.rs`

**Interfaces:**
- Consumes: Tasks 1–5 compiler/materialization/repository contracts and `CurrentUser`.
- Produces: `ChartService::{capabilities,preview,save,metadata,data,list,delete}`, seven authenticated routes, public DTOs, and stable HTTP error mapping.

- [ ] **Step 1: Write failing API behavior tests**

  Cover exact request/response JSON for all seven routes; public string IDs; 201 create/200 replay; preview with zero DB/CAS writes;
  direct save without preview; six ChartModel kinds; saved metadata/config/diagnostics; opaque session cursor; source deletion plus
  view-ID reads; delete response/tombstone; every stable chart error and specified 400/409/413/404/500 mapping; 401; and all routes
  returning 404 for another user.

- [ ] **Step 2: Run API tests and verify red**

  Run: `cd backend && cargo test --features e2e-test --test chart_views`

  Expected: FAIL because routes and `ChartService` are absent.

- [ ] **Step 3: Implement `ChartService` source resolution and preview**

  Resolve a live source by `UserId`, open its owned artifact, call Task 2 capabilities or Tasks 3–4 compiler, and map only bounded
  public error details. Preview returns `viewId/name=null` and performs no stage/commit/database writes.

- [ ] **Step 4: Implement save and persisted reads**

  Recompile deterministically, write Plot Data, call Task 5 atomic publish, and return full metadata with `replayed`. Read `/data`
  from Plot Data plus frozen metadata only. Delete/list/metadata delegate to the user-scoped repository and collect unreferenced
  blobs only after successful transactions.

- [ ] **Step 5: Add typed Axum handlers and error mapping**

  Register exactly the spec's seven paths, `limit=1..100`, opaque cursor parsing, 10 MiB response budget, body DTOs and statuses.
  Add all chart stable errors and import `VISUALIZATION_KINDS` from `charting`.

- [ ] **Step 6: Wire `ChartService` into `AppState`**

  Construct it in production and e2e constructors with `ChartViewRepo`, `ResultRepo`, `ArtifactRepo/Service`, and the shared reader;
  keep dependency order named rather than positional where practical.

- [ ] **Step 7: Update route/contract assertions**

  Change the route count from 75 to 82; assert each method/path once and the complete stable-error set. Keep six view-kind enums.

- [ ] **Step 8: Verify API, auth, contract, and deletion tests**

  Run: `cd backend && cargo test --features e2e-test --test chart_views && cargo test --test contract_manifest && cargo test --test auth_scope && cargo test --test result_deletion`

  Expected: PASS with seven route contracts and no cross-user distinction.

- [ ] **Step 9: Commit the chart HTTP boundary**

  ```bash
  git add backend/src/application/charting/service.rs backend/src/application/charting/mod.rs backend/src/application/mod.rs backend/src/api/chart_views.rs backend/src/api/mod.rs backend/tests/chart_views.rs
  git add -p backend/src/contract_manifest.rs backend/tests/contract_manifest.rs
  git diff --cached --check
  git commit -m "feat: expose user-configured chart APIs"
  ```

### Task 7: Remove every Agent and analysis chart-creation path

**Files:**
- Delete: `backend/src/tools/create_dataset_view.rs`
- Delete: `backend/src/application/visualization.rs`
- Modify: `backend/src/tools/mod.rs`
- Modify: `backend/src/tools/run_result_analysis.rs`
- Modify: `backend/src/application/analysis.rs`
- Modify: `backend/src/application/comparisons.rs`
- Modify: `backend/resources/agent/v4/manifest.json`
- Modify: `backend/resources/agent/v4/skill-index.json`
- Modify: `backend/resources/agent/v4/system.md`
- Modify: `backend/resources/agent/v4/skills/result-analysis.md`
- Modify: `backend/resources/agent/v4/skills/comparison.md`
- Modify: `backend/tests/tool_manifest.rs`
- Modify: `backend/tests/agent_knowledge.rs`
- Modify: `backend/tests/data_resources.rs`
- Modify: `backend/tests/result_deletion.rs`
- Modify: `integration-tests/e2e/resilience.spec.ts`

**Interfaces:**
- Consumes: Task 6 UI-owned chart API as the sole creation boundary.
- Produces: 23-tool `v4.4.0` Agent bundle and `run_result_analysis` that can only produce Parquet datasets.

- [ ] **Step 1: Write failing negative-authority tests**

  Assert runtime registry and manifest contain 23 unique tools, omit `create_dataset_view`, `run_result_analysis` schema rejects
  `initialViews`, result-analysis requiredTools contains only `run_result_analysis`, and scripted Agent turns cannot invoke the
  removed tool. Assert analysis produces datasets without any implicit `dataset_views` rows.

- [ ] **Step 2: Run tool/knowledge tests and verify red**

  Run: `cd backend && cargo test --test tool_manifest && cargo test --test agent_knowledge`

  Expected: FAIL while the old tool and bundle entries remain.

- [ ] **Step 3: Remove tool and analysis service code**

  Delete tool registration/module, `ViewArg`, `Args.initial_views`, `AnalysisRequest.initial_views`, the post-analysis create loop,
  `create_view`, `create_resource_view`, fake view-analysis script creation, and comparison/data-resource callers. Keep ordinary
  `run_result_analysis` dataset lineage unchanged.

- [ ] **Step 4: Update the Agent bundle and hashes**

  Set bundle/manifest version to `v4.4.0`; change result-analysis wording to dataset filtering/aggregation only; remove visualize
  triggers/tool requirements or redirect plain “画图” to the result workbench; update system/comparison text and regenerate all
  affected SHA-256 values. Do not grant an HTTP chart tool to the Agent.

- [ ] **Step 5: Rewrite old resilience/data-resource tests**

  Remove `initialViews/create_dataset_view` fixtures. Preserve resilience coverage around analysis datasets and move view survival
  evidence to Task 6/Task 11 user-configured chart tests.

- [ ] **Step 6: Verify Agent, analysis, and tool registries**

  Run: `cd backend && cargo test --test tool_manifest && cargo test --test agent_knowledge && cargo test --test data_resources && cargo test --test result_deletion`

  Expected: PASS with 23 tools and no remaining symbol/schema/string that creates a chart through the Agent.

- [ ] **Step 7: Commit authority removal**

  ```bash
  git add -A backend/src/tools/create_dataset_view.rs backend/src/application/visualization.rs
  git add backend/src/tools/mod.rs backend/src/tools/run_result_analysis.rs backend/src/application/analysis.rs backend/src/application/comparisons.rs backend/resources/agent/v4/manifest.json backend/resources/agent/v4/skill-index.json backend/resources/agent/v4/system.md backend/resources/agent/v4/skills/result-analysis.md backend/resources/agent/v4/skills/comparison.md backend/tests/tool_manifest.rs backend/tests/agent_knowledge.rs backend/tests/data_resources.rs backend/tests/result_deletion.rs
  git add -p integration-tests/e2e/resilience.spec.ts
  git diff --cached --check
  git commit -m "refactor: move chart creation out of agent"
  ```

### Task 8: Replace frontend chart types and ECharts compilers

**Files:**
- Create: `frontend/src/types/chart.ts`
- Modify: `frontend/src/types/dataset.ts`
- Modify: `frontend/src/types/message.ts`
- Modify: `frontend/src/types/index.ts`
- Modify: `frontend/src/types/contracts.test.ts`
- Modify: `frontend/src/api/client.ts`
- Create: `frontend/src/hooks/useChartView.ts`
- Modify: `frontend/src/hooks/useDatasetView.ts`
- Modify: `frontend/src/components/datasets/ChartRenderer.tsx`
- Modify: `frontend/src/components/datasets/charts/shared.ts`
- Modify: `frontend/src/components/datasets/charts/line.ts`
- Modify: `frontend/src/components/datasets/charts/bar.ts`
- Modify: `frontend/src/components/datasets/charts/scatter.ts`
- Modify: `frontend/src/components/datasets/charts/heatmap.ts`
- Modify: `frontend/src/components/datasets/charts/boxplot.ts`
- Modify: `frontend/src/components/datasets/charts/histogram.ts`
- Modify: `frontend/src/components/datasets/charts.test.ts`
- Modify: `frontend/src/hooks/useDatasetView.test.ts`

**Interfaces:**
- Consumes: Task 6 JSON DTOs and exact palette/presentation rules.
- Produces: TypeScript `ChartConfig/ChartModel/ChartView*` types, chart API functions, stale-safe `useChartView(viewId)`, and six safe ECharts option builders.

- [ ] **Step 1: Write failing type/API/hook tests**

  Assert all six discriminated models, tagged large integer categories, lightweight view summaries, API paths/status responses,
  and late view A response ignored after switching to B. Assert no `DatasetViewData.spec/columns/rows` dependency remains.

- [ ] **Step 2: Write failing ECharts golden tests**

  For all six models assert series/axes/data, no executable formatter, exact four palettes, legend show/position/orientation,
  title/grid, line width/style/symbol with `smooth=false`, stable colors after rerender, and desktop-right legend downgraded on a
  390px media query without hiding it.

- [ ] **Step 3: Run focused frontend tests and verify red**

  Run: `cd frontend && npm test -- --run src/types/contracts.test.ts src/hooks/useDatasetView.test.ts src/components/datasets/charts.test.ts`

  Expected: FAIL against the old spec/row chart contract.

- [ ] **Step 4: Add TypeScript DTOs and API clients**

  Mirror the spec types exactly. Add `getChartCapabilities`, `previewChart`, `createChartView`, `getChartView`, `getChartViewData`,
  `listChartViews`, and `deleteChartView`; all IDs stay strings and errors use existing `ApiError` handling.

- [ ] **Step 5: Replace the view hook and renderer input**

  Implement `useChartView(id)` with `AbortController`/generation isolation. Make `ChartRenderer` accept only `ChartModel`; remove
  source schema and raw spec/rows from all builders.

- [ ] **Step 6: Implement six safe ECharts adapters**

  Map only typed model/presentation fields, tagged categories, palette constants and responsive legend intent. Keep animation off,
  shared grid/tooltip/aria/resize/dispose, and never pass through unknown JSON.

- [ ] **Step 7: Verify focused frontend contracts**

  Run: `cd frontend && npm test -- --run src/types/contracts.test.ts src/hooks/useDatasetView.test.ts src/components/datasets/charts.test.ts`

  Expected: PASS for all six models and presentation settings.

- [ ] **Step 8: Commit typed chart rendering**

  ```bash
  git add frontend/src/types/chart.ts frontend/src/types/dataset.ts frontend/src/types/message.ts frontend/src/types/index.ts frontend/src/types/contracts.test.ts frontend/src/api/client.ts frontend/src/hooks/useChartView.ts frontend/src/hooks/useDatasetView.ts frontend/src/hooks/useDatasetView.test.ts frontend/src/components/datasets/ChartRenderer.tsx frontend/src/components/datasets/charts/shared.ts frontend/src/components/datasets/charts/line.ts frontend/src/components/datasets/charts/bar.ts frontend/src/components/datasets/charts/scatter.ts frontend/src/components/datasets/charts/heatmap.ts frontend/src/components/datasets/charts/boxplot.ts frontend/src/components/datasets/charts/histogram.ts frontend/src/components/datasets/charts.test.ts
  git diff --cached --check
  git commit -m "feat: render typed chart models"
  ```

### Task 9: Build the fixed-template chart configurator and preview flow

**Files:**
- Create: `frontend/src/hooks/useChartCapabilities.ts`
- Create: `frontend/src/hooks/useChartPreview.ts`
- Create: `frontend/src/components/datasets/ChartBuilder.tsx`
- Create: `frontend/src/components/datasets/ChartFieldSelect.tsx`
- Create: `frontend/src/components/datasets/ChartPreview.tsx`
- Create: `frontend/src/components/datasets/ChartBuilder.test.tsx`
- Modify: `frontend/src/components/datasets/DatasetWorkbench.tsx`
- Modify: `frontend/src/components/datasets/DatasetTable.test.tsx`

**Interfaces:**
- Consumes: Task 8 DTO/API/renderer and Task 6 capability/preview routes.
- Produces: explicit “新建图表” flow, fixed six-template forms, collapsed advanced/display settings, preview freshness state, and save request callbacks.

- [ ] **Step 1: Write failing builder behavior tests**

  Assert table remains default; “新建图表” loads capabilities; incompatible fields are disabled with reasons; `v_mag[0..2]`
  creates A/B/C measures; basic fields precede collapsed advanced/display settings; List truncation is not selectable; line-only
  controls stay hidden for other kinds; style changes invalidate preview; save is disabled until the current config previews; and
  preview errors keep the form editable.

- [ ] **Step 2: Write failing stale-request tests**

  Resolve an old dataset capability/preview after switching datasets/session/config and assert it cannot replace current options,
  preview, diagnostics or save eligibility. Assert aborted requests do not show an error toast.

- [ ] **Step 3: Run component tests and verify red**

  Run: `cd frontend && npm test -- --run src/components/datasets/ChartBuilder.test.tsx src/components/datasets/DatasetTable.test.tsx`

  Expected: FAIL because builder components do not exist.

- [ ] **Step 4: Implement capability and preview hooks**

  Expose `{data,loading,error,reload}` and `{model,previewHash,loading,error,run,clear}` with abort/generation guards. Compute a
  client config fingerprint solely for UI freshness; the backend remains authoritative for config/request hashes.

- [ ] **Step 5: Implement fixed-template forms and controlled defaults**

  Provide exact slots/rules from the compatibility matrix, deterministic presentation defaults, generated names/labels, and only
  the approved advanced fields. Keep aggregation/sampling collapsed unless a server diagnostic requires them; never emit arbitrary
  option keys.

- [ ] **Step 6: Integrate builder, preview, and save into `DatasetWorkbench`**

  Keep table tab first, existing pagination/download/delete behavior intact, and saved view tabs lightweight. On successful save,
  refresh metadata, open the new view through Task 8's hook, and notify “图表已保存”.

- [ ] **Step 7: Verify workbench component behavior**

  Run: `cd frontend && npm test -- --run src/components/datasets/ChartBuilder.test.tsx src/components/datasets/DatasetTable.test.tsx src/hooks/useDataset.test.ts`

  Expected: PASS with table behavior unchanged and preview/save gated correctly.

- [ ] **Step 8: Commit the chart builder**

  ```bash
  git add frontend/src/hooks/useChartCapabilities.ts frontend/src/hooks/useChartPreview.ts frontend/src/components/datasets/ChartBuilder.tsx frontend/src/components/datasets/ChartFieldSelect.tsx frontend/src/components/datasets/ChartPreview.tsx frontend/src/components/datasets/ChartBuilder.test.tsx frontend/src/components/datasets/DatasetWorkbench.tsx frontend/src/components/datasets/DatasetTable.test.tsx
  git diff --cached --check
  git commit -m "feat: add fixed-template chart builder"
  ```

### Task 10: Add saved-chart discovery, independent opening, copying, and deletion

**Files:**
- Create: `frontend/src/hooks/useSavedCharts.ts`
- Create: `frontend/src/components/datasets/SavedChartPanel.tsx`
- Create: `frontend/src/components/datasets/ChartViewWorkbench.tsx`
- Create: `frontend/src/components/datasets/SavedChartPanel.test.tsx`
- Modify: `frontend/src/components/ResultPanel.tsx`
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/components/datasets/DatasetWorkbench.tsx`
- Modify: `frontend/src/types/modelDataframe.ts`
- Create: `frontend/src/components/ResultPanel.test.tsx`

**Interfaces:**
- Consumes: Tasks 8–9 view APIs/renderer/builder.
- Produces: fourth `WorkbenchTab='charts'`, cursor-paged saved charts, `OpenChartView`, source-independent modal, copy-to-builder, and delete/invalidation flow.

- [ ] **Step 1: Write failing saved-chart panel tests**

  Assert the fourth “图表” tab; cursor paging/de-duplication; loading/empty/error/retry; source name/deleted badge; session switch
  clearing prior cards; late-page isolation; open-by-view-ID without `getDataset`; delete confirmation and refresh; and copy loading
  frozen config into a new builder with a new operation ID/name.

- [ ] **Step 2: Write failing independent workbench/responsive tests**

  Open a source-deleted chart and assert metadata/data load only through view routes. At 390px assert no horizontal overflow and a
  saved right legend renders at the bottom while staying visible and keyboard reachable.

- [ ] **Step 3: Run focused UI tests and verify red**

  Run: `cd frontend && npm test -- --run src/components/datasets/SavedChartPanel.test.tsx src/components/ResultPanel.test.tsx`

  Expected: FAIL because the charts tab and independent workbench are absent.

- [ ] **Step 4: Implement saved chart state and panel**

  Add user/session-scoped generation guards, opaque cursor loading, stable ID de-duplication, delete refresh and explicit source
  deleted presentation. Do not cache chart cards across session changes.

- [ ] **Step 5: Wire the fourth tab and independent modal into `App`**

  Track `openChartViewId` separately from `OpenDataFrame`. Dataset tabs and saved cards both open `ChartViewWorkbench`; only live
  dataset builder paths open `DatasetWorkbench`. Close a deleted chart and increment chart-list revision without disturbing run or
  sibling result state.

- [ ] **Step 6: Implement copy-as-new**

  Fetch metadata config, seed `ChartBuilder`, clear persisted IDs/hashes, allocate a fresh `clientOperationId`, and require a unique
  name plus a fresh preview before save.

- [ ] **Step 7: Verify saved-chart lifecycle UI**

  Run: `cd frontend && npm test -- --run src/components/datasets/SavedChartPanel.test.tsx src/components/ResultPanel.test.tsx src/components/datasets/ChartBuilder.test.tsx`

  Expected: PASS with source-independent opening and narrow-screen legend behavior.

- [ ] **Step 8: Commit saved-chart discovery**

  ```bash
  git add frontend/src/hooks/useSavedCharts.ts frontend/src/components/datasets/SavedChartPanel.tsx frontend/src/components/datasets/ChartViewWorkbench.tsx frontend/src/components/datasets/SavedChartPanel.test.tsx frontend/src/components/datasets/DatasetWorkbench.tsx frontend/src/components/ResultPanel.tsx frontend/src/components/ResultPanel.test.tsx frontend/src/App.tsx frontend/src/types/modelDataframe.ts
  git diff --cached --check
  git commit -m "feat: add saved chart workbench"
  ```

### Task 11: Replace browser journeys, update current docs, and close release gates

**Files:**
- Modify: `integration-tests/e2e/v3-workflows.spec.ts`
- Modify: `integration-tests/e2e/resilience.spec.ts`
- Modify: `integration-tests/e2e/v4-managed-studies.spec.ts` if result fixtures need List values
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `docs/development-status.md`
- Modify: `docs/api-design.md`
- Modify: `docs/db-design.md`
- Modify: `docs/frontend-design.md`
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/v4-deployment.md` if reset/release counts are enumerated
- Modify: `README.md` only if it currently enumerates routes/tools/views
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/contract_manifest.rs`

**Interfaces:**
- Consumes: Tasks 1–10 complete feature.
- Produces: deterministic browser evidence, updated machine-readable coverage, current design/status documentation, and full candidate-quality verification.

- [ ] **Step 1: Replace the Agent-created-view Playwright journey**

  Change the old “创建持久折线视图” scripted-LLM flow into a real UI journey: run a fixture containing
  `bus: Utf8View` and `v_mag: LargeList(Float64)`; open a non-first dataset; confirm table default; create a line with
  `v_mag[0..2]`; set A/B/C labels, legend, colorblind palette and dashed/symbol style; preview/save; assert three rendered series;
  refresh; delete the source; reopen from “图表”; copy; delete; and verify logical storage release.

- [ ] **Step 2: Add cross-user and 390px evidence**

  Exercise all seven routes for a second user and assert 404. Repeat saved-chart opening at a 390px viewport, assert right legend
  downgrade, no horizontal overflow, and no axe serious/critical violations.

- [ ] **Step 3: Run focused Playwright and verify behavior**

  Run: `cd frontend && npm run test:e2e -- v3-workflows.spec.ts -g "user-configured charts survive source deletion"`

  Expected: PASS through browser → Vite → Axum → SQLite/CAS without a chart Agent tool call.

- [ ] **Step 4: Update machine-readable feature coverage**

  Replace `tool:create_dataset_view` evidence, bind all seven route contracts to Rust and real Playwright evidence, retain six
  `view:*` kinds, add every chart stable error, and update route/tool counts to 82/23. Ensure old resilience entries do not name
  `initialViews`.

- [ ] **Step 5: Reconcile current docs without overwriting unrelated user edits**

  Merge the approved spec into the current status/API/DB/frontend/Agent/testing/deployment docs. State empty DB/DATA_DIR reset,
  `v4.4.0`, no LLM chart authority, ChartModel rather than ECharts JSON, style controls, Plot Data lifecycle, source-deletion
  discovery, and all limits/errors. Stage only the feature's hunks.

- [ ] **Step 6: Run deterministic backend gates**

  Run:

  ```bash
  cd backend
  cargo fmt --all -- --check
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --features e2e-test
  ```

  Expected: PASS with zero fmt diff, zero Clippy warnings, and all backend tests passing.

- [ ] **Step 7: Run deterministic frontend gates**

  Run:

  ```bash
  cd frontend
  npm test -- --run
  npm run build
  npm run test:e2e
  ```

  Expected: PASS for Vitest, TypeScript/Vite production build, and the full Playwright suite.

- [ ] **Step 8: Run functional coverage**

  Run: `./scripts/check-functional-coverage --require-pr`

  Expected: PASS with 100% route/message/tool/skill/upload/view/error traceability and no stale `create_dataset_view` contract.

- [ ] **Step 9: Commit documentation and verification evidence**

  ```bash
  git status --short
  git diff --check
  git diff --cached --check
  git add -p integration-tests/catalog/features.yaml integration-tests/e2e/v3-workflows.spec.ts integration-tests/e2e/resilience.spec.ts integration-tests/e2e/v4-managed-studies.spec.ts docs/development-status.md docs/api-design.md docs/db-design.md docs/frontend-design.md docs/agent-prompt-skills-design.md docs/integration-testing-design.md docs/v4-deployment.md README.md backend/src/contract_manifest.rs backend/tests/contract_manifest.rs
  git diff --cached --name-only
  git diff --cached --check
  git commit -m "test: cover user-configured chart workbench"
  ```

  Before committing, unstage any listed file that has no feature-owned hunk; never include unrelated pre-existing edits.

## Self-Review Result

- Spec coverage: all 15 spec sections map to Tasks 1–11; no uncovered requirement remains.
- Step scan: Tasks 1–10 start with a failing focused test and end with green verification plus an isolated commit; Task 11 replaces the browser evidence after the feature exists, then runs every release gate and commits only documentation/test hunks.
- Type consistency: Tasks 1–7 own the Rust contracts and API; Task 8 mirrors exactly those names for Tasks 9–10; Task 11 only consumes them.
- Review focus: nested Lists (Tasks 2–3), deletion races (Tasks 5–6/10), big integers/non-finite values (Tasks 1–3), idempotency (Tasks 5–6), and stale/responsive UI (Tasks 9–10) each have named tests.
- Proportion: the plan specifies interfaces, tests, commands, and decisions without transcribing compiler/repository bodies.
