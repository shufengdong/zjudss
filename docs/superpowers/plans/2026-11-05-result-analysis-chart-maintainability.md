# Result Analysis and Chart Maintainability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate user-scoped result reading and split the analysis, comparison, chart compiler, chart lifecycle, and chart builder responsibilities without changing public behavior.

**Architecture:** Add one application-layer `ResultSourceService` that resolves a live owned dataset, its artifact identity, parsed schema, session, and `DataFrameReadSource`. Keep analysis, comparison, and chart state machines independent, but make them consume this shared read-only fact; split their pure policy and lifecycle helpers into focused modules. Keep the frontend server-controlled by separating pure chart configuration state from React effects and presentation controls while preserving the deterministic `chart-model-v1` to ECharts adapters.

**Tech Stack:** Rust 2024, Axum, Diesel/SQLite, Arrow/Parquet, Tokio; React 18, TypeScript, Vitest, Testing Library, ECharts.

**Spec:** `docs/superpowers/specs/2026-11-05-result-analysis-chart-maintainability-design.md`

## Global Constraints

- Do not change public routes, DTO JSON shapes, messages, stable error codes, migrations, or the six supported chart configurations.
- All result and derived data remain Snappy Parquet; do not add CSV or fixed-metric result paths.
- LLM and frontend inputs remain stable resource IDs plus controlled specs; never accept paths, commands, SQL, or arbitrary ECharts option.
- Every repository query continues to accept `UserId` explicitly; cross-user, deleted, and absent resources remain indistinguishable 404s.
- Keep analysis, comparison, and chart state machines and their transaction boundaries independent.
- All `tensoreval` execution remains behind `backend/src/adapters/`.
- Work directly on `master`, preserve unrelated user changes, use TDD, and commit each completed task separately.

## Review Focus

- A dataset deleted between source resolution and chart publication must return `chart_source_deleted` and leave no live view; Task 6 pins the publication race.
- A dataset deleted or replaced after an analysis input snapshot must not silently switch the artifact staged for execution; Task 3 pins the one-resolution frozen input.
- Cross-user dataset IDs must remain 404-equivalent in table, analysis, comparison, and chart paths; Tasks 1, 2, 3, and 6 pin each caller.
- Nested fixed-size-list field paths and user labels must survive edit-state/config round trips; Task 7 pins field identity and label preservation.
- Empty/null-heavy data, incompatible units, excessive scan rows, and plot budgets must retain existing deterministic diagnostics; Tasks 4 and 5 pin these policy edges.

---

## Planned File Structure

### Backend result and analysis boundaries

- Create `backend/src/application/result_sources.rs`: owned live result resolution and immutable source description.
- Modify `backend/src/application/datasets.rs`: dataset DTO/lifecycle facade consuming `ResultSourceService` for page/download.
- Create `backend/src/application/analysis/inputs.rs`: input resolution, frozen snapshot rows, and staged input descriptors.
- Create `backend/src/application/analysis/outputs.rs`: output validation, artifact commit cleanup, and result dataset drafts.
- Modify `backend/src/application/analysis.rs`: analysis state-machine facade and event projection only.

### Backend comparison and chart boundaries

- Create `backend/src/application/comparisons/compatibility.rs`: pure compatible dataset selection and schema policy.
- Create `backend/src/application/comparisons/script.rs`: controlled RSS rendering from compatibility facts.
- Modify `backend/src/application/comparisons.rs`: run validation, analysis dispatch, persistence, and events.
- Keep `backend/src/application/charting/compiler.rs` as the public compiler facade.
- Create `backend/src/application/charting/compiler/{source,common,schema,line,bar,scatter,heatmap,boxplot,histogram}.rs`: focused compiler internals.
- Create `backend/src/application/charting/service/{source,materialization,presentation}.rs`: source error mapping, Plot Data publication preparation, and stored DTO/cursor projection.
- Modify `backend/src/application/charting/service.rs`: `ChartService` facade and lifecycle orchestration.

### Frontend chart builder

- Create `frontend/src/components/datasets/chart_builder/fieldMapping.ts`: pure field identity/key/name mapping.
- Create `frontend/src/components/datasets/chart_builder/state.ts`: edit state, seeding, and controlled config construction.
- Create `frontend/src/components/datasets/chart_builder/useChartBuilder.ts`: capabilities, preview freshness, and save-operation effects.
- Create `frontend/src/components/datasets/chart_builder/TemplateControls.tsx`: chart-kind-specific controls.
- Create `frontend/src/components/datasets/chart_builder/PresentationControls.tsx`: legend, title, grid, palette, and line styling.
- Modify `frontend/src/components/datasets/ChartBuilder.tsx`: composition and layout facade.

---

### Task 1: Characterize Result, Analysis, Comparison, and Chart Boundaries

**Files:**
- Modify: `backend/tests/result_deletion.rs`
- Modify: `backend/tests/chart_views.rs`
- Modify: `backend/src/application/comparisons.rs` (unit tests only)
- Modify: `backend/src/application/charting/compiler.rs` (unit tests only)

**Interfaces:**
- Consumes: existing public HTTP routes, `compatible_pair`, `comparison_script`, and `ChartCompiler::compile`.
- Produces: behavior locks only; no production interface.

- [ ] **Step 1: Add result lifecycle characterization cases**

Add assertions to the existing deletion scenario proving: cross-user metadata/page return not found; a deleted source cannot start a new analysis; a derived dataset still reports the frozen source schema/content hash/tombstone; and a saved chart remains readable with `sourceDeleted=true`.

- [ ] **Step 2: Run the focused result test before refactoring**

Run: `cd backend && cargo test --features e2e-test --test result_deletion -- --nocapture`  
Expected: PASS, establishing the current behavior baseline.

- [ ] **Step 3: Add chart and compiler edge characterizations**

Extend existing tests for: cross-user capability/preview/save; null-heavy series diagnostics; aggregate/order determinism; scan/point budget error codes; and every saved chart kind reading after source deletion.

- [ ] **Step 4: Add comparison pure-policy characterizations**

Add unit cases asserting stable-key selection, incompatible-unit rejection, invalid RSS identifiers being excluded, empty compatible values rejecting with `comparison_incompatible`, and generated RSS containing only the supported frame functions.

- [ ] **Step 5: Run focused chart and comparison tests**

Run: `cd backend && cargo test --features e2e-test --test chart_views -- --nocapture && cargo test --features e2e-test application::comparisons && cargo test --features e2e-test application::charting::compiler`  
Expected: PASS.

- [ ] **Step 6: Commit the characterization checkpoint**

```bash
git add backend/tests/result_deletion.rs backend/tests/chart_views.rs backend/src/application/comparisons.rs backend/src/application/charting/compiler.rs
git commit -m "test: characterize result analysis and chart boundaries"
```

### Task 2: Introduce the Shared Owned Result Source

**Files:**
- Create: `backend/src/application/result_sources.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/application/datasets.rs`
- Test: `backend/src/application/result_sources.rs`
- Test: `backend/tests/result_deletion.rs`

**Interfaces:**
- Consumes: `ResultRepo::find_live(&UserId, i32)`, `ResultRepo::session_id(&UserId, i32)`, `ArtifactService::find_owned(i32, i32)`, `DataFrameReadSource`, and `DataFrameColumn`.
- Produces: `pub(crate) struct ResultSource { dataset: RunDataset, session_id: i32, artifact: ArtifactRef, columns: Vec<DataFrameColumn>, read_source: DataFrameReadSource }`; `ResultSource::producer() -> Value`; `pub(crate) struct ResultSourceService`; `ResultSourceService::new(ResultRepo, ArtifactService) -> Self`; `ResultSourceService::resolve(&self, &CurrentUser, i32) -> Result<ResultSource, ResultSourceError>`.

- [ ] **Step 1: Write failing source-resolution tests**

Add module tests that create a live dataset/artifact and assert `resolve` returns the artifact SHA-256 as `read_source.content_hash`, the owning session, parsed columns, and the correct run/analysis producer. Add absent, cross-user, deleted-dataset, missing-artifact, and malformed-stored-schema cases with their internal error variants.

- [ ] **Step 2: Run the new module test and verify failure**

Run: `cd backend && cargo test --features e2e-test application::result_sources`  
Expected: FAIL because `result_sources` and its types do not exist.

- [ ] **Step 3: Implement `ResultSourceService` and register the module**

Resolve live ownership through `ResultRepo`, session through its scoped query, artifact through the principal storage scope, parse the stored schema once, and construct `DataFrameReadSource { resource_key: "dataset:{id}", artifact_id, schema_json, row_count, content_hash: artifact.sha256 }`. Keep error mapping internal and path-free.

- [ ] **Step 4: Make `DatasetService::page` and `download` consume the source**

Add `sources: ResultSourceService` to `DatasetService`, construct it from existing dependencies, remove duplicate `DataFrameReadSource` construction and the dead `file` method, and leave metadata/delete behavior unchanged.

- [ ] **Step 5: Run source, dataset, and deletion tests**

Run: `cd backend && cargo test --features e2e-test application::result_sources && cargo test --features e2e-test --test simulation_runs --test result_deletion`  
Expected: PASS with unchanged HTTP payloads and deletion behavior.

- [ ] **Step 6: Commit the shared source**

```bash
git add backend/src/application/result_sources.rs backend/src/application/mod.rs backend/src/application/datasets.rs backend/tests/result_deletion.rs
git commit -m "refactor: centralize owned result sources"
```

### Task 3: Split Analysis Input and Output Responsibilities

**Files:**
- Create: `backend/src/application/analysis/inputs.rs`
- Create: `backend/src/application/analysis/outputs.rs`
- Modify: `backend/src/application/analysis.rs`
- Test: `backend/tests/result_deletion.rs`
- Test: `backend/tests/data_resources.rs`

**Interfaces:**
- Consumes: Task 2 `ResultSourceService::resolve`, existing `DataResourceRepo`, `ArtifactService`, `ParquetInspector`, `AnalysisRepo`, and `SimEngine::run_analysis`.
- Produces: `ResolvedAnalysisInputs::resolve(...) -> Result<Self, AnalysisError>`; `ResolvedAnalysisInputs::snapshot_rows() -> Vec<ResultAnalysisInput>`; `ResolvedAnalysisInputs::stage(&self, &ArtifactService, i32, &Path) -> Result<Vec<NamedInput>, AnalysisError>`; `collect_outputs(...) -> Result<CommittedAnalysisOutputs, AnalysisError>`; `CommittedAnalysisOutputs::drafts() -> Vec<NewResultDataset>` and explicit async release semantics used by the facade on publication failure.

- [ ] **Step 1: Write failing resolved-input unit tests**

Add unit tests against the planned `ResolvedAnalysisInputs` API proving one resolved dataset descriptor produces both its lineage snapshot row and its staged artifact ID/input key without a dataset ID re-query API. Also assert data-resource descriptors retain their normalized content hash and session restriction. Referencing the not-yet-created input module provides the initial compile failure without adding a production synchronization hook.

- [ ] **Step 2: Run the focused analysis tests and verify failure**

Run: `cd backend && cargo test --features e2e-test application::analysis::inputs`  
Expected: FAIL because the `analysis::inputs` module and `ResolvedAnalysisInputs` do not exist.

- [ ] **Step 3: Implement `analysis/inputs.rs`**

Move dataset/data-resource resolution, session validation, producer JSON, immutable snapshot construction, input keys, and artifact descriptors into `ResolvedAnalysisInputs`. Stage only resolved artifact IDs and never re-query a dataset by ID during execution.

- [ ] **Step 4: Implement `analysis/outputs.rs`**

Move declared-name matching, Snappy validation, storage staging/commit, schema hashing, dataset draft creation, and partial-commit release into the output collector. Preserve the exact existing error codes.

- [ ] **Step 5: Reduce `AnalysisService` to state-machine orchestration**

Have `run` resolve inputs once before repository creation, pass their snapshot rows to `AnalysisRepo::create`, pass the same resolved value to staging, call the output collector, atomically publish through `AnalysisRepo::complete`, then emit existing events. Keep stage order and failure recording unchanged.

- [ ] **Step 6: Run analysis and lifecycle coverage**

Run: `cd backend && cargo test --features e2e-test --test result_deletion --test data_resources && cargo test --features e2e-test application::analysis`  
Expected: PASS; no engine call exists outside adapters.

- [ ] **Step 7: Commit the analysis split**

```bash
git add backend/src/application/analysis.rs backend/src/application/analysis backend/tests/result_deletion.rs backend/tests/data_resources.rs
git commit -m "refactor: separate analysis inputs and outputs"
```

### Task 4: Extract Comparison Compatibility and Script Policy

**Files:**
- Create: `backend/src/application/comparisons/compatibility.rs`
- Create: `backend/src/application/comparisons/script.rs`
- Modify: `backend/src/application/comparisons.rs`
- Test: `backend/src/application/comparisons/compatibility.rs`
- Test: `backend/src/application/comparisons/script.rs`
- Test: `backend/tests/result_deletion.rs`, which contains current comparison integration coverage.

**Interfaces:**
- Consumes: `RunDataset`, stored `DatasetColumn` schema, and `AnalysisService::run`.
- Produces: `Compatibility::select(Vec<RunDataset>, Vec<RunDataset>) -> Result<CompatiblePair, ComparisonError>`; serializable `CompatiblePair { baseline, candidate, facts }`; `render_script(&CompatibilityFacts) -> String`.

- [ ] **Step 1: Move Task 1 policy tests beside the planned modules and make imports fail**

Keep the assertions unchanged while referring to `compatibility::Compatibility::select` and `script::render_script`.

- [ ] **Step 2: Run comparison tests and verify compile failure**

Run: `cd backend && cargo test --features e2e-test application::comparisons`  
Expected: FAIL because the submodules do not exist.

- [ ] **Step 3: Implement typed compatibility facts**

Parse stored schema once per candidate pair, select sorted stable keys and compatible numeric values, enforce unit equality, exclude invalid RSS identifiers, and serialize exactly the current compatibility JSON keys and string dataset IDs.

- [ ] **Step 4: Move controlled RSS rendering**

Render only `select`, `alias`, `join`, `with_columns`, `col`, and arithmetic from typed facts. Keep the exact result variable and declared output name used by analysis.

- [ ] **Step 5: Update the comparison facade and run tests**

Run: `cd backend && cargo test --features e2e-test application::comparisons && cargo test --features e2e-test --test result_deletion`  
Expected: PASS with unchanged snapshots, errors, records, and events.

- [ ] **Step 6: Commit the comparison split**

```bash
git add backend/src/application/comparisons.rs backend/src/application/comparisons backend/tests
git commit -m "refactor: isolate comparison compatibility policy"
```

### Task 5: Split the Chart Compiler by Shared Primitives and Chart Kind

**Files:**
- Modify: `backend/src/application/charting/compiler.rs`
- Create: `backend/src/application/charting/compiler/source.rs`
- Create: `backend/src/application/charting/compiler/common.rs`
- Create: `backend/src/application/charting/compiler/schema.rs`
- Create: `backend/src/application/charting/compiler/line.rs`
- Create: `backend/src/application/charting/compiler/bar.rs`
- Create: `backend/src/application/charting/compiler/scatter.rs`
- Create: `backend/src/application/charting/compiler/heatmap.rs`
- Create: `backend/src/application/charting/compiler/boxplot.rs`
- Create: `backend/src/application/charting/compiler/histogram.rs`
- Test: compiler module tests in the same files.

**Interfaces:**
- Consumes: existing `ChartConfig`, `ChartSource`, field-path readers, `ChartBudgets`, and sampling helpers.
- Produces: unchanged `ChartCompiler::compile(&ChartConfig, &ChartSource) -> Result<CompiledChart, ChartError>` and unchanged `CompiledChart`; internal `CompileContext`, candidate/series/aggregation/order helpers, and one `compile` function per kind.

- [ ] **Step 1: Add missing pure compiler edge tests from Review Focus**

Pin empty batches, all-null selected values, mixed valid/null values, fixed-size-list paths, duplicate X aggregation, source/xAsc/xDesc ordering, LTTB limits, histogram boundaries, boxplot quartiles/outliers, heatmap count, series/point/scan budget errors, and stable diagnostics.

- [ ] **Step 2: Run compiler tests before moving code**

Run: `cd backend && cargo test --features e2e-test application::charting::compiler`  
Expected: PASS.

- [ ] **Step 3: Extract source and common primitives**

Move column/path lookup and typed candidate extraction to `source.rs`; move shared missing-value, series key, aggregation, ordering, unit, and budget logic to `common.rs`; move Plot Data schema constructors to `schema.rs`. Keep helpers private to the compiler tree.

- [ ] **Step 4: Extract all six algorithms without semantic edits**

Move each existing `compile_*` implementation to its named module. Do not merge line/bar output generation, boxplot statistics, histogram binning, heatmap cells, or scatter point logic.

- [ ] **Step 5: Make `compiler.rs` the dispatch facade and run focused tests**

Run: `cd backend && cargo fmt --all && cargo test --features e2e-test application::charting::compiler && cargo test --features e2e-test --test chart_views`  
Expected: PASS with byte-equivalent serialized chart models for the characterization fixtures.

- [ ] **Step 6: Run Clippy on the split before committing**

Run: `cd backend && cargo clippy --all-targets --all-features -- -D warnings`  
Expected: PASS without broad `allow` attributes introduced for the split.

- [ ] **Step 7: Commit the compiler split**

```bash
git add backend/src/application/charting/compiler.rs backend/src/application/charting/compiler
git commit -m "refactor: split chart compiler algorithms"
```

### Task 6: Split Chart Source, Materialization, and Stored View Presentation

**Files:**
- Modify: `backend/src/application/charting/service.rs`
- Create: `backend/src/application/charting/service/source.rs`
- Create: `backend/src/application/charting/service/materialization.rs`
- Create: `backend/src/application/charting/service/presentation.rs`
- Test: `backend/tests/chart_views.rs`
- Test: `backend/tests/chart_view_repository.rs`
- Test: `backend/tests/result_deletion.rs`

**Interfaces:**
- Consumes: Task 2 `ResultSourceService`, Task 5 `ChartCompiler`, `PlotDataCodec`, `ArtifactService::publish_chart_view`, and `ChartViewRepo`.
- Produces: unchanged `ChartService` public methods; internal `compile_source(...)`, `prepare_materialized_view(...) -> PreparedChartView`, and presentation functions for metadata/summary/frozen model/cursors.

- [ ] **Step 1: Add source-integration and publication-race characterizations**

Add unit assertions against the planned chart-source adapter proving capability, preview, and save receive the same Task 2 `ResultSource` content identity. Extend the repository/integration scenario proving a source tombstoned before the atomic publish is rejected with `chart_source_deleted` and leaves no live view or leaked artifact reference; use the existing repository state transition rather than a new production synchronization hook.

- [ ] **Step 2: Run the planned adapter test and verify import failure**

Run: `cd backend && cargo test --features e2e-test application::charting::service::source`  
Expected: FAIL because the chart service source adapter module does not exist. Then run the existing lifecycle characterizations and confirm they remain PASS before moving code.

- [ ] **Step 3: Replace private chart `Source` with `ResultSource`**

Construct `ResultSourceService` in `ChartService::new`; use it for capabilities, preview, and save; map its not-found/artifact errors to the exact existing `not_found` or `chart_source_deleted` semantics at the operation boundary.

- [ ] **Step 4: Extract materialization preparation**

Move config normalization/hash, compiler-to-Plot-Data encoding, diagnostics serialization, integer bounds, and `ChartViewDraft` construction to `materialization.rs`. Keep the repository/artifact atomic publish call in the service facade.

- [ ] **Step 5: Extract stored presentation helpers**

Move summary/metadata/frozen-model/cursor encoding and decoding to `presentation.rs`, preserving JSON, timestamps, cursor binding, and stored-corruption mappings.

- [ ] **Step 6: Run all chart and deletion tests**

Run: `cd backend && cargo test --features e2e-test application::charting && cargo test --features e2e-test --test chart_views --test chart_view_repository --test result_deletion`  
Expected: PASS, including saved chart reads after source deletion.

- [ ] **Step 7: Commit the chart lifecycle split**

```bash
git add backend/src/application/charting/service.rs backend/src/application/charting/service backend/tests/chart_views.rs backend/tests/chart_view_repository.rs backend/tests/result_deletion.rs
git commit -m "refactor: separate chart source and view lifecycle"
```

### Task 7: Split Frontend Chart Builder State, Mapping, and Controls

**Files:**
- Create: `frontend/src/components/datasets/chart_builder/fieldMapping.ts`
- Create: `frontend/src/components/datasets/chart_builder/fieldMapping.test.ts`
- Create: `frontend/src/components/datasets/chart_builder/state.ts`
- Create: `frontend/src/components/datasets/chart_builder/state.test.ts`
- Create: `frontend/src/components/datasets/chart_builder/useChartBuilder.ts`
- Create: `frontend/src/components/datasets/chart_builder/TemplateControls.tsx`
- Create: `frontend/src/components/datasets/chart_builder/PresentationControls.tsx`
- Modify: `frontend/src/components/datasets/ChartBuilder.tsx`
- Modify: `frontend/src/components/datasets/ChartBuilder.test.tsx`

**Interfaces:**
- Consumes: existing `ChartConfig`, `ChartFieldRef`, `ChartCapabilities`, `useChartCapabilities`, `useChartPreview`, and `createChartView`.
- Produces: `fieldIdentity`, `fieldKey`, `fieldName`, `resolveField`; `ChartBuilderState`; `seedChartBuilderState(...)`; `buildChartConfig(...) -> ChartConfig | null`; and `useChartBuilder(...)` returning state/actions/config/freshness/save status for the facade.

- [ ] **Step 1: Write failing pure mapping and state tests**

Cover scalar and nested path identity, disabled field fallback, preservation of original field labels, all six config seeds, all six config builders, heatmap count coercion, line numeric-X sampling, kind capability disablement, measure limits, and edit/config round trips.

- [ ] **Step 2: Run the new frontend tests and verify import failure**

Run: `cd frontend && npm test -- --run src/components/datasets/chart_builder/fieldMapping.test.ts src/components/datasets/chart_builder/state.test.ts`  
Expected: FAIL because the modules do not exist.

- [ ] **Step 3: Implement the pure field and state modules**

Move only deterministic transformations. Represent related values in one `ChartBuilderState` object and use typed actions/patches; do not call React hooks or APIs from these files.

- [ ] **Step 4: Implement `useChartBuilder`**

Own capability seeding, initial-config reseeding, preview clearing/fingerprint freshness, stable client operation ID per `{name,fingerprint}`, save/loading behavior, and notification callbacks. Preserve existing user-visible copy and defaults.

- [ ] **Step 5: Extract template and presentation controls**

Make both components controlled: props are state plus typed change callbacks. Keep accessibility labels and conditional controls unchanged so existing component tests remain meaningful.

- [ ] **Step 6: Reduce `ChartBuilder.tsx` to composition and extend component tests**

Retain loading/error/template-reason states, two-column layout, preview and save buttons. Add component assertions for initial-config cloning, stale preview after any config/style change, operation ID reuse on retry, and nested field display labels.

- [ ] **Step 7: Run frontend unit and build gates**

Run: `cd frontend && npm test -- --run && npm run build`  
Expected: PASS with no TypeScript errors and unchanged ECharts adapter snapshots.

- [ ] **Step 8: Commit the frontend split**

```bash
git add frontend/src/components/datasets/ChartBuilder.tsx frontend/src/components/datasets/ChartBuilder.test.tsx frontend/src/components/datasets/chart_builder
git commit -m "refactor: split chart builder state and controls"
```

### Task 8: Converge Documentation and Run Full Gates

**Files:**
- Modify: `docs/architecture.md`
- Modify: `docs/development-status.md`
- Modify only if a public behavior changed unexpectedly: `docs/api-design.md`, `docs/db-design.md`, `docs/frontend-design.md`, `integration-tests/catalog/features.yaml`

**Interfaces:**
- Consumes: all prior task interfaces and verification output.
- Produces: current architectural ownership map and evidence-backed 8.6 completion status.

- [ ] **Step 1: Audit public-contract and boundary drift**

Run `git diff <8.6-design-commit>..HEAD` plus targeted `rg` checks for direct process spawning outside adapters, duplicate dataset `DataFrameReadSource` construction, arbitrary ECharts option types, new routes, and migration changes. Revert or document any unintended drift before proceeding.

- [ ] **Step 2: Update architecture documentation**

Document `ResultSourceService`, the single physical DataFrame reader, independent analysis/comparison/chart state machines, chart compiler module ownership, frozen Plot Data behavior, and frontend controlled-config/ECharts boundary.

- [ ] **Step 3: Run backend format, lint, and complete tests**

Run:

```bash
cd backend
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --features e2e-test
```

Expected: all commands PASS.

- [ ] **Step 4: Run frontend complete tests and production build**

Run:

```bash
cd frontend
npm test -- --run
npm run build
```

Expected: both commands PASS.

- [ ] **Step 5: Run functional coverage and real-dependency candidate checks**

Run: `./scripts/check-functional-coverage --require-pr`, then the candidate-release commands specified in `docs/v4-deployment.md` when the real configured dependencies are available.  
Expected: functional coverage PASS; record the exact real-dependency result. Do not call the known external `case_projection_invalid` condition a pass.

- [ ] **Step 6: Update development status with exact evidence**

Record completed ownership changes, commit range, command results, and any external dependency blocker. State explicitly that routes, DTOs, migrations, chart contract, and agent permissions did not change.

- [ ] **Step 7: Commit documentation and status**

```bash
git add docs/architecture.md docs/development-status.md docs/api-design.md docs/db-design.md docs/frontend-design.md integration-tests/catalog/features.yaml
git commit -m "docs: record result and chart maintainability phase"
```

- [ ] **Step 8: Verify the final worktree**

Run: `git status --short && git log --oneline --decorate -10`  
Expected: clean worktree and a reviewable sequence of 8.6 commits.
