# V4 Case Projection Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consume eig-rs `case_projections.json` and rssbundle manifest v2 so every enabled case is imported through its declared projector into Model/Data/Struct upload candidates without exposing source paths at runtime.

**Architecture:** The engine resource snapshot parses and hashes the projection catalog, validates each manifest against an enabled profile and facade, and publishes normalized projection metadata. A new adapter operation executes only the validated projector against the private staged source tree and returns the declared ordered Parquet outputs. Case upload parsing uses those outputs to create model or ordinary resource candidates from catalog roles, while ordinary DSS uploads retain the existing static import classifier.

**Tech Stack:** Rust, Axum, async-trait, serde, Diesel/SQLite, Parquet, tensoreval, existing upload batch and engine resource modules.

**Spec:** `docs/v4-deployment.md` and `docs/superpowers/specs/2026-10-26-independent-data-resources-design.md`

## Global Constraints

- Every tensoreval process call remains inside `backend/src/adapters/`.
- The browser, LLM and managed RSS never receive a server path, source root or projector invocation.
- `projectorOutputs` is authoritative for output order, role, schema and disposition; no function-name or fixed-table inference is allowed.
- `inputKind=model` creates model candidates; `data|struct` creates ordinary data-resource candidates; diagnostic outputs do not become facade inputs.
- Invalid profiles and manifests fail closed with stable diagnostics; unrelated facades and cases remain available.
- Existing published artifacts stay immutable and ordinary user DSS uploads keep their current behavior.
- Work directly on `master` as authorized by `AGENTS.md`, preserving all unrelated staged and unstaged changes.

## Review Focus

- Duplicate, missing or reordered projector output files must fail atomically before any candidate is committed.
- A manifest that names a different projector, target function, result mapping or managed entry than its profile must remain visible but non-importable.
- Diagnostic outputs must be consumed as bounded import diagnostics and must never enter the active model or resource selector.
- A Case batch must use the snapshot contract it was created from even if the live eig-rs tree changes afterward.
- Path-like catalog values, aliases and manifest entries must be rejected before a script is generated.

---

### Task 1: Projection Catalog and Snapshot Contract

**Files:**
- Create: `backend/src/engine_resources/projections.rs`
- Modify: `backend/src/engine_resources/mod.rs`
- Modify: `backend/src/engine_resources/snapshot.rs`
- Modify: `backend/src/catalog/schema.rs`
- Modify: `backend/src/db/repositories/engine_resource_snapshot.rs`
- Modify: `backend/src/db/models.rs`
- Modify: `backend/src/db/schema.rs`
- Modify: `backend/migrations/00000000000000_initial/up.sql`
- Test: `backend/tests/engine_resources.rs`
- Test: `backend/tests/v4_schema.rs`

**Interfaces:**
- Produces: `CaseProjectionRegistry`, `CaseProjectionProfile`, `ProjectorOutputContract`, snapshot `case_projections_json`, hash and schema version.

- [ ] Write failing tests for catalog schema v2 loading, snapshot identity/hash publication, enabled/disabled profile isolation, `contiguous_zero_based`, and persisted schema version/hash.
- [ ] Run the focused tests and confirm the new contract is absent.
- [ ] Implement the projection registry and snapshot/database fields, including strict identifier, ordering, disposition, schema-kind and facade-input mapping checks.
- [ ] Run focused tests and `cargo fmt --all -- --check`.

### Task 2: Manifest v2 Validation and Public Case Metadata

**Files:**
- Modify: `backend/src/engine_resources/examples.rs`
- Modify: `backend/src/api/examples.rs`
- Modify: `backend/src/tools/search_examples.rs`
- Modify: `backend/src/tools/inspect_example.rs`
- Modify: `backend/tests/common/mod.rs`
- Modify: `backend/tests/fixtures/engine_resources/catalog/`
- Test: `backend/tests/engine_resources.rs`
- Test: `backend/tests/case_import.rs`
- Test: `backend/tests/workspace_tools.rs`

**Interfaces:**
- Consumes: `CaseProjectionRegistry` from Task 1.
- Produces: normalized `ExampleProjection` on `ExampleManifest`; `EngineExample.importable` means manifest, projection and source files are all valid.

- [ ] Write failing tests for valid schema v2, mismatched profile fields/results, disabled profiles, missing managed entry, unsafe paths and public DTO path redaction.
- [ ] Run the tests and confirm schema v2 is rejected by the old loader.
- [ ] Implement manifest v2 parsing and profile/facade validation without retaining schema v1 compatibility.
- [ ] Upgrade isolated fixtures to schema v2 and run focused tests.

### Task 3: Controlled Projector Adapter

**Files:**
- Modify: `backend/src/adapters/import.rs`
- Modify: `backend/src/adapters/mod.rs`
- Modify: `backend/src/adapters/process/mod.rs`
- Modify: `backend/src/adapters/stub.rs`
- Test: `backend/tests/process_engine_v4.rs`
- Test: `backend/tests/model_import_adapter.rs`

**Interfaces:**
- Consumes: ordered projector/output data copied from a validated profile.
- Produces: `CaseProjectionSpec` and `ProjectedCase`; `SimEngine::project_case`.

- [ ] Write failing adapter tests for safe script construction, exact output set/order, nonzero exit diagnostics, unsafe symbols/paths and cleanup.
- [ ] Run tests and confirm `project_case` does not exist.
- [ ] Implement projector execution using the existing ProcessEngine execution boundary and Parquet validation.
- [ ] Implement deterministic StubEngine projector outputs and run focused adapter tests.

### Task 4: Case Upload Projection and Candidate Split

**Files:**
- Modify: `backend/src/application/uploads.rs`
- Modify: `backend/src/application/case_import.rs`
- Test: `backend/tests/case_import.rs`
- Test: `backend/tests/upload_batches.rs`
- Test: `backend/tests/failure_atomicity.rs`

**Interfaces:**
- Consumes: manifest/profile metadata from Task 2 and `SimEngine::project_case` from Task 3.
- Produces: one recoverable upload batch whose candidates exactly match facade-input projector outputs.

- [ ] Write failing tests for dynamic model/data/struct splitting, diagnostic exclusion, exact schema refs/aliases, projector failure atomicity and unchanged ordinary DSS import.
- [ ] Run focused tests and confirm Case still enters the fixed 20+1 classifier.
- [ ] Route only Case sources through the profile projector, commit outputs atomically, and preserve profile evidence in candidate/batch diagnostics.
- [ ] Run case/upload/failure tests.

### Task 5: Release Gate, Coverage and Documentation

**Files:**
- Modify: `backend/tests/release_engine_cases.rs`
- Modify: `scripts/test-release-integration`
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `docs/v4-deployment.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/development-status.md`

**Interfaces:**
- Consumes: all prior tasks.
- Produces: dynamic all-manifest coverage counts and one real import/run per current projection profile/study family.

- [ ] Write failing release assertions for zero facade diagnostics, every valid manifest mapped to an enabled executable profile, all cases importable, and representative projector/run evidence.
- [ ] Implement dynamic release reporting without fixed facade/case counts.
- [ ] Update the current design/status documents and machine feature trace.
- [ ] Run focused tests, the backend suite, frontend tests/build if public DTOs changed, functional coverage, and `git diff --check`.
