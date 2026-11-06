# Dynamic V13 Projection Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Accept native-result supersets with exact ordered managed projections and regenerate trustworthy real-engine release evidence.

**Architecture:** Keep the profile and manifest boundaries separate: profile parsing validates the new selector source, while example validation checks managed results as an ordered subset of native results with exact profile mappings. The release wrapper invalidates its prior report before Cargo starts, and current documents record only freshly verified evidence.

**Tech Stack:** Rust, serde/serde_json, shell, Cargo integration tests, Markdown.

**Spec:** `docs/superpowers/specs/2026-11-06-dynamic-v13-projection-contract-design.md`

## Global Constraints

- All tensoreval execution remains inside `backend/src/adapters/`.
- LLM and frontend boundaries continue to expose only stable IDs and controlled specs.
- Case projection schema v2 remains fail closed; no schema v1 compatibility is added.
- Work directly on `master` as required by repository `AGENTS.md`; preserve unrelated user changes.
- The adjacent eig-rs tree is read-only to zjudss and is consumed only through `TENSOREVAL_BIN` and `SPARROWZZ_DIR` during verification.

## Review Focus

- A native result omitted from the managed subset must not cause rejection.
- A managed result absent from native results must still be rejected.
- Reordered managed results must still be rejected even when every name maps.
- Wrong targets or bindings must still be rejected.
- A failed release-engine Cargo invocation must leave no stale report.

---

### Task 1: Parse the published selector contract

**Files:**
- Modify: `backend/src/engine_resources/projections.rs`
- Modify: `backend/src/engine_resources/snapshot.rs`
- Modify: `backend/tests/fixtures/engine_resources/catalog/case_projections.json`
- Test: `backend/tests/engine_resources.rs`

**Interfaces:**
- Consumes: catalog `resultSelector.valueSource` string.
- Produces: enabled `CaseProjectionProfile` values using `profile.resultMap(manifest.projection.results)`.

- [ ] Change the fixture catalog to the new selector string and run the focused profile test; expect the profile to be disabled before production parsing changes.
- [ ] Update `parse_profile` and the embedded snapshot fixture to require the new selector string.
- [ ] Run `cargo test --test engine_resources publishes_enabled_case_projection_with_ordered_outputs`; expect PASS.
- [ ] Commit the selector parser, fixtures, spec, and plan as `fix: accept managed projection selectors`.

### Task 2: Validate managed results as an ordered native subset

**Files:**
- Modify: `backend/src/engine_resources/examples.rs`
- Test: `backend/tests/engine_resources.rs`

**Interfaces:**
- Consumes: `ExampleManifest.study.results`, `ExampleManifest.projection.results`, and `CaseProjectionProfile.bundle_result_map`.
- Produces: `projection_matches(&ExampleManifest, &CaseProjectionProfile) -> bool` accepting only exact ordered managed subsets.

- [ ] Add a fixture behavior test where native results contain an additional mapped result and managed projection selects only the first result; expect it to fail before implementation.
- [ ] Add behavior cases rejecting a managed source absent from native results, reordered managed sources, wrong target, and wrong binding.
- [ ] Implement a single-pass ordered-subsequence check while preserving all existing metadata and mapping comparisons.
- [ ] Run `cargo test --test engine_resources`; expect all tests PASS.
- [ ] Commit the manifest validation and behavior tests as `fix: validate managed result subsets`.

### Task 3: Invalidate stale release reports

**Files:**
- Modify: `scripts/test-release-engine`
- Test: `backend/tests/process_engine_v4.rs`

**Interfaces:**
- Consumes: exact report path `target/release-integration/engine-report.json`.
- Produces: no report when Cargo fails after preflight; a nonempty newly generated report on success.

- [ ] Add a Unix test that runs a copied release script with a valid fake binary/resource directory, a failing fake Cargo executable, and a pre-existing report; expect the stale report to remain before the fix.
- [ ] Remove the exact report after preflight and before invoking Cargo.
- [ ] Run `cargo test --test process_engine_v4 release_script`; expect PASS.
- [ ] Commit the release wrapper and regression test as `fix: invalidate stale engine reports`.

### Task 4: Verify the real contract and update current documentation

**Files:**
- Modify: `docs/v4-deployment.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/v4-tester-guide.md`
- Modify: `docs/development-status.md`

**Interfaces:**
- Consumes: current eig-rs binary, sparrowzz snapshot, and generated `engine-report.json`.
- Produces: dated candidate evidence and documentation of native-versus-managed result semantics.

- [ ] Run formatting, Clippy, focused tests, and full `cargo test --features e2e-test`; expect zero failures.
- [ ] Run `TENSOREVAL_BIN=../eig-rs/target/release/tensoreval SPARROWZZ_DIR=../eig-rs/sparrowzz ./scripts/test-release-engine`; expect zero non-importable examples and zero projection failures.
- [ ] Update current documents with the selector/subset semantics and exact fresh report counts/hashes, retaining old 144/144 values only as dated history.
- [ ] Run `git diff --check` and the relevant documentation/functional coverage gate; expect PASS.
- [ ] Commit current verification evidence as `docs: record Dynamic V13 release evidence`.

