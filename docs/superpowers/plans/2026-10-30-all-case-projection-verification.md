# All-Case Projection Verification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the real eig-rs release gate execute and validate `project_case` for every published rssbundle case before representative managed studies run.

**Architecture:** Split the existing release test into an all-case projection phase and a representative managed-run phase. The projection phase materializes every case under a `.rssbundle` acquisition root, validates every catalog-declared output through `ProcessEngine`, records only stable counts/codes, and retains successful named inputs so representative runs do not project the same case twice.

**Tech Stack:** Rust, Tokio, `ProcessEngine`, tensoreval Parquet adapter, TypeScript/Vitest release assertions, YAML functional catalog.

**Spec:** `docs/superpowers/specs/2026-10-30-all-case-projection-verification-design.md`

## Global Constraints

- Every importable case in the immutable engine snapshot must execute the real `ProcessEngine::project_case`.
- Model/Data/Struct output classification comes only from the current projection profile; no fixed table count is allowed.
- Diagnostic output is validated but never becomes a named facade input.
- Physical paths, scripts, secrets and raw stderr must not enter the release report.
- `managed.rss` remains one successful representative per referenced projection profile.
- All tensoreval calls remain inside `backend/src/adapters/`; the release test only calls `SimEngine` methods.

## Review Focus

- A profile with no Model outputs but valid Data/Struct outputs must still count as a successful import — pinned in Task 1 report assertions and Task 2 disposition counting.
- A diagnostic-only projector output must be counted but excluded from representative named inputs — pinned in Task 2.
- Two cases sharing one profile must each execute projection while only one successful case executes `managed.rss` — pinned by projected/example count equality and representative/profile count equality in Tasks 1 and 2.
- A failed case must not prevent later cases from being projected, and the report must retain its stable ID/code — pinned in Task 2 outcome collection.
- Result-selector injection must mutate only the selected representative's Struct artifact — pinned by reusing the existing selector execution after all-case projection in Task 2.

---

### Task 1: Freeze the all-case release report contract

**Files:**
- Modify: `frontend/src/release/semantic-assertions.test.ts`
- Modify: `integration-tests/release/semantic-assertions.ts`
- Modify: `frontend/tests/ci/workflows.test.ts`
- Modify: `scripts/test-release-integration`

**Interfaces:**
- Consumes: the existing camelCase engine report loaded by `expectEngineReleaseEvidence(report)`.
- Produces: required report fields `projectedExamples: number`, `projectionFailures: number`, and `caseProjections: CaseProjectionEvidence[]`, where each evidence item contains `exampleId`, `projectionProfile`, `success`, `modelOutputs`, `dataOutputs`, `structOutputs`, `diagnosticOutputs`, and optional `failureCode`.

- [ ] **Step 1: Write failing semantic assertion tests**

Extend `accepts the bundle-only engine report and rejects inconsistent counts` with 103 successful `caseProjections`, assert the valid report passes, and add failures for `projectedExamples < importableExamples`, nonzero `projectionFailures`, duplicate/missing case evidence, and a failed evidence item.

- [ ] **Step 2: Run the focused Vitest and verify RED**

Run: `cd frontend && npm test -- --run src/release/semantic-assertions.test.ts`

Expected: FAIL because `expectEngineReleaseEvidence` does not require the all-case projection fields.

- [ ] **Step 3: Implement the semantic report invariants**

In `expectEngineReleaseEvidence(report)`, require:

- `projectedExamples === importableExamples === bundleExamples`;
- `projectionFailures === 0`;
- `caseProjections.length === projectedExamples`;
- unique stable `exampleId` values, non-empty `projectionProfile`, `success === true`, non-negative integer disposition counts, and no `failureCode` on successful evidence.

- [ ] **Step 4: Pin the shell release gate to the same fields**

Update the inline report validation in `scripts/test-release-integration` and its workflow test so a candidate release fails unless all three counts agree, failures are zero, case evidence has unique IDs, and every evidence item succeeded.

- [ ] **Step 5: Run focused tests and verify GREEN**

Run: `cd frontend && npm test -- --run src/release/semantic-assertions.test.ts tests/ci/workflows.test.ts`

Expected: both files pass with no failures.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/release/semantic-assertions.test.ts integration-tests/release/semantic-assertions.ts frontend/tests/ci/workflows.test.ts scripts/test-release-integration
git commit -m "test: require every case projection in release evidence"
```

### Task 2: Project every published case with the real engine

**Files:**
- Modify: `backend/tests/release_engine_cases.rs`

**Interfaces:**
- Consumes: `EngineResourceSnapshot`, `CaseProjectionProfile`, `ProcessEngine::project_case(CaseProjectionSpec)`, and `ProjectedCase { dataframes, diagnostics }`.
- Produces: private `PreparedCaseProjection` values reusable by representative runs and serializable `CaseProjectionReport` evidence matching Task 1.

- [ ] **Step 1: Add the report types and make the existing release test require complete evidence**

Add `CaseProjectionReport` and `PreparedCaseProjection`. Extend `ReleaseReport` with `projected_examples`, `projection_failures`, and `case_projections`. Rename the test to `real_engine_projects_all_examples_and_runs_representative_studies` and assert that evidence count equals `importable_examples`, IDs are unique, and failures are empty.

- [ ] **Step 2: Run the release test and verify RED**

Run:

```bash
cd backend
TENSOREVAL_BIN=../../eig-rs/target/release/tensoreval \
SPARROWZZ_DIR=../../eig-rs/sparrowzz \
cargo test --release --features release-test --test release_engine_cases \
  real_engine_projects_all_examples_and_runs_representative_studies -- --exact --nocapture
```

Expected: FAIL because the existing representative-only loop cannot supply one projection report per importable case.

- [ ] **Step 3: Implement `project_example`**

Add:

```rust
async fn project_example(
    engine: &ProcessEngine,
    snapshot: &EngineResourceSnapshot,
    example: &EngineExample,
    root: &Path,
) -> (CaseProjectionReport, Option<PreparedCaseProjection>)
```

It must materialize `run-<stable-id>.rssbundle`, invoke `project_case` once, classify each non-diagnostic alias by its matching `ProjectorOutputContract.input_kind`, count diagnostics separately, copy only facade inputs into `inputs/*.parquet`, and return stable public failure evidence without a physical path.

- [ ] **Step 4: Project all importable cases without early exit**

Iterate all importable snapshot examples in deterministic ID order, call `project_example` for each, collect every report, and retain successful prepared values by example ID. Compute `projected_examples` from successful reports and `projection_failures` from failed reports; do not stop at the first error.

- [ ] **Step 5: Reuse prepared projections for representative managed runs**

Change `run_representative` to accept `PreparedCaseProjection` instead of invoking `project_case`. For each referenced profile, choose the first successful prepared candidate in the existing size/file-count order, inject its manifest-derived `resultSelector`, and execute exact `managed.rss`.

- [ ] **Step 6: Write the report before asserting failures**

Serialize all case evidence plus representative runs, then assert zero projection failures and zero representative failures. Keep detailed failure text in process-local assertion diagnostics only; serialize only `failureCode`. Mark the existing representative-run `failure_message` field with `#[serde(skip)]` (or move it to a non-serialized local failure collection) so the report cannot expose raw adapter diagnostics or paths.

- [ ] **Step 7: Run the real engine test and verify GREEN**

Run the command from Step 2.

Expected: PASS with `projectedExamples == importableExamples == bundleExamples == 144`, `projectionFailures == 0`, and six successful representative profile runs.

- [ ] **Step 8: Commit**

```bash
git add backend/tests/release_engine_cases.rs
git commit -m "test: project every published engine case"
```

### Task 3: Align feature trace, documentation and full verification

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/v4-deployment.md`
- Modify: `docs/development-status.md`

**Interfaces:**
- Consumes: Task 1 report fields and Task 2 renamed release test.
- Produces: current machine trace and operator documentation that distinguish all-case import verification from representative managed execution.

- [ ] **Step 1: Update machine-readable test references and assertions**

Replace the old release test name in every feature entry. State explicitly that every manifest case executes its projector and validates Model/Data/Struct/diagnostic outputs, while `managed.rss` remains representative per referenced profile.

- [ ] **Step 2: Update current documentation**

Document the new 144/144 real projection evidence, report fields, failure semantics, and the boundary that only six profile representatives continue to final simulation.

- [ ] **Step 3: Run deterministic suites**

Run:

```bash
cd backend
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --features e2e-test
cd ../frontend
npm test -- --run
npm run build
npm run test:e2e
cd ..
./scripts/check-functional-coverage --require-pr
git diff --check
```

Expected: backend suite, 181 Vitest tests, production build, 29 Playwright tests, 333/333 PR contracts and diff check pass; adjust exact totals only when the machine catalog legitimately adds a new test contract.

- [ ] **Step 4: Re-run the real release engine gate**

Run: `CARGO_TARGET_DIR=/private/tmp/zjudss-release-engine-target ./scripts/test-release-engine`

Expected: process adapter suite passes and the all-case release test reports 144 successful projections plus six successful representative managed runs.

- [ ] **Step 5: Commit**

```bash
git add integration-tests/catalog/features.yaml docs/integration-testing-design.md docs/v4-deployment.md docs/development-status.md
git commit -m "docs: require real projection for every case"
```
