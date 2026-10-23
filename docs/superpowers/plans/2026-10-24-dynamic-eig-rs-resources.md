# Dynamic eig-rs Resources Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace all eig-rs release pins with a validated runtime snapshot of the currently configured `TENSOREVAL_BIN` and `TENSOREVAL_CWD`.

**Architecture:** The adapter probes the configured executable and returns verified capabilities plus its startup SHA-256. The resource registry dynamically discovers catalogs, Markdown, RSS files, and bundle manifests, computes a canonical snapshot, and publishes the exact loaded case files before application services can consume them. Release automation builds the current default eig-rs branch and reports evidence without accepting a pinned commit, case count, artifact run, or binary hash.

**Tech Stack:** Rust 2024, Axum application bootstrap, SHA-256, Parquet adapter probes, shell release gates, GitHub Actions, YAML/Markdown contracts.

**Spec:** `docs/superpowers/specs/2026-10-24-dynamic-eig-rs-resources-design.md`

## Global Constraints

- Work directly on `master` as explicitly approved; preserve and exclude the user's `backend/resources/init_users.csv` edit.
- All tensoreval subprocesses remain under `backend/src/adapters/`.
- Do not checkout, pull, reset, clean, or rewrite the configured eig-rs worktree.
- Keep `ENGINE_CONTRACT_VERSION = "tensoreval-parquet-v3"`; it identifies the application protocol, not an eig-rs revision.
- Support catalog schema version `2`; reject unknown versions rather than adding a compatibility shim.
- Limits are 512 Markdown documents, 4 MiB per document, 64 MiB total documentation, 2,048 examples, 10,000 files and 2 GiB per example, and dependency depth 64.
- Runtime may expose zero examples; the candidate release gate requires at least one root RSS and one bundle.
- No database migration and no claim that binary/resource hashes are persisted with historical runs.

## Review Focus

- A source file changes between discovery and publish: startup fails with `engine_resource_changed_during_scan`, and no mixed snapshot is activated (Task 2 test).
- An existing published snapshot directory is corrupt or incomplete: publish verifies it and fails closed instead of reusing it (Task 2 test).
- Documentation or examples contain symlinks, path escapes, excessive depth, count, or bytes: global resources fail safely and individual examples receive redacted diagnostics according to ownership (Task 1 tests).
- The configured executable is replaced after startup: every adapter spawn returns `engine_binary_changed` before execution (Task 3 test).
- Current eig-rs changes example counts or has an individually broken example: runtime remains dynamic, public cases stay modelizable-only, and release evidence reports counts without exact-number assertions (Tasks 1 and 4 tests).

---

### Task 1: Dynamic resource discovery and agent manifest

**Files:**
- Modify: `backend/src/agent/knowledge.rs`
- Modify: `backend/resources/agent/v3/manifest.json`
- Delete: `backend/resources/agent/v3/examples-source.json`
- Modify: `backend/src/engine_resources/{mod.rs,examples.rs,snapshot.rs}`
- Modify: `backend/tests/{agent_knowledge.rs,engine_resources.rs}`
- Modify: `backend/tests/common/mod.rs`

**Interfaces:**
- Produces: `EngineResourceRegistry::load(tensoreval_cwd: &Path, capabilities: &HashSet<String>) -> Result<EngineResourceSnapshot, EngineResourceError>`.
- Produces: code-owned `SUPPORTED_CATALOG_SCHEMA_VERSIONS: &[u32] = &[2]` and canonical, sorted runtime discovery.
- Produces: `EngineResourceSnapshot` containing loaded catalog/document bytes and exact example file inventories, without embedded eig-rs commit/hash contracts.
- Consumes: verified adapter feature names supplied later by Task 3; tests may pass an explicit `HashSet<String>`.

- [ ] **Step 1: Write failing manifest and dynamic-discovery tests.** Remove pinned-field expectations from `agent_knowledge`; assert the embedded manifest contains only bundle-owned hashes. Rewrite `engine_resources` fixtures to call `load`, then cover recursive sorted Markdown, top-level RSS and bundle discovery, snapshot stability/change, supported/unknown catalog schema, missing required catalog shape, empty examples, individual broken bundle/dependency diagnostics, symlink/path rejection, duplicate IDs, and every resource limit.

- [ ] **Step 2: Run focused tests and verify RED.** Run `cd backend && cargo test --test agent_knowledge --test engine_resources`; expected failure is missing dynamic `load` behavior and still-required embedded pin fields.

- [ ] **Step 3: Implement minimal dynamic registry.** Remove eig-rs fields from `AgentManifest`; remove the embedded example source. Discover only regular contained files in sorted portable-path order, validate catalogs, statically resolve literal RSS/DSS dependencies to depth 64, retain bundle self-hashes, redact diagnostics to relative paths, enforce the exact limits, and generate a versioned length-prefixed SHA-256 snapshot ID from all loaded bytes and normalized diagnostics.

- [ ] **Step 4: Update shared test fixtures to the produced interface.** Replace `ResourceContract`/`SourceFileSpec` construction in `backend/tests/common/mod.rs` with a real dynamic resource tree and `EngineResourceRegistry::load(..., &HashSet::new())`.

- [ ] **Step 5: Run focused tests and verify GREEN.** Run `cd backend && cargo test --test agent_knowledge --test engine_resources`; expected all tests pass.

- [ ] **Step 6: Commit.** Commit only Task 1 files with message `feat: discover current eig-rs resources dynamically`.

### Task 2: Immutable published case sources

**Files:**
- Modify: `backend/src/engine_resources/{mod.rs,snapshot.rs}`
- Modify: `backend/src/application/case_import.rs`
- Modify: `backend/src/main.rs`
- Modify: `backend/tests/{engine_resources.rs,case_import.rs}`
- Modify: `backend/tests/common/mod.rs`

**Interfaces:**
- Consumes: Task 1's complete loaded-byte/file inventory.
- Produces: `EngineResourceRegistry::publish(snapshot: EngineResourceSnapshot, data_dir: &Path) -> Result<(EngineResourceSnapshot, PathBuf), EngineResourceError>`; the returned snapshot's `engine_root` points at the immutable published source tree.
- Produces: verified `manifest.json` inventory for safe reuse of an existing snapshot directory.
- Consumed by: `CaseImportService::accept`, which reads only the returned published snapshot.

- [ ] **Step 1: Write failing publish tests.** Assert catalogs, docs, manifests, and every case dependency are copied; mutate a source after load and expect `engine_resource_changed_during_scan`; corrupt an existing destination and expect failure; assert case import succeeds after deleting/changing the live source tree because it uses published bytes.

- [ ] **Step 2: Run focused tests and verify RED.** Run `cd backend && cargo test --test engine_resources --test case_import`; expected failure shows examples are not published and case import still reads the live eig-rs root.

- [ ] **Step 3: Implement atomic verified publishing.** Re-read and hash each source before copy, write the full inventory and metadata in staging, atomically rename, verify all declared files on reuse, and return a rebased snapshot. Never follow symlinks or overwrite an existing corrupt destination.

- [ ] **Step 4: Wire startup and fixtures.** Publish before wrapping the snapshot in `Arc`; construct `SchemaCatalog` from the published snapshot and make all test bootstraps follow the same invariant.

- [ ] **Step 5: Run focused and application tests GREEN.** Run `cd backend && cargo test --test engine_resources --test case_import --test app_bootstrap`; expected all tests pass.

- [ ] **Step 6: Commit.** Commit Task 2 files with message `feat: freeze dynamic engine case snapshots`.

### Task 3: Functional engine preflight and binary identity

**Files:**
- Modify: `backend/src/adapters/{contract.rs,mod.rs}`
- Modify: `backend/src/adapters/process/mod.rs`
- Modify: `backend/src/main.rs`
- Modify: `backend/tests/process_engine_v3.rs`

**Interfaces:**
- Produces: `EngineConfig::probe(bin: PathBuf, default_timeout: Duration) -> Result<EngineConfig, AdapterError>` with `binary_sha256` and actual named-Parquet/static-import/single-Solve/script-validation results.
- Produces: `EngineCapabilities::resource_features() -> HashSet<String>` for Task 1's registry.
- Produces: `ProcessEngine::capabilities()` and `ProcessEngine::binary_sha256()` read-only accessors.
- Consumes: Task 1's registry load takes `resource_features()`; Task 2 startup ordering publishes only after engine preflight succeeds.

- [ ] **Step 1: Write failing adapter tests.** Use executable fixtures that independently fail named Parquet, static import, single-Solve import, and validation probes; verify each stable error. Probe a good fixture, replace its bytes, then assert all direct and shared spawn paths return `engine_binary_changed` without running the replacement.

- [ ] **Step 2: Run focused test and verify RED.** Run `cd backend && cargo test --test process_engine_v3`; expected failure shows inferred import/validation flags and no binary replacement check.

- [ ] **Step 3: Implement functional probes and identity guard.** Keep every command in the adapter, generate bounded private probe trees, validate real Snappy Parquet/model outputs and the static script validator, calculate startup SHA-256, and verify that hash immediately before each process spawn.

- [ ] **Step 4: Reorder production startup.** Construct and validate `ProcessEngine` once, pass `resource_features()` to registry loading, publish resources, then inject the same engine instance into `AppState`; log only binary hash and dynamic snapshot metadata, never a commit.

- [ ] **Step 5: Run adapter and bootstrap tests GREEN.** Run `cd backend && cargo test --test process_engine_v3 --test app_bootstrap`; expected all tests pass.

- [ ] **Step 6: Commit.** Commit Task 3 files with message `feat: probe current tensoreval capabilities`.

### Task 4: Unpinned real-dependency release gate

**Files:**
- Modify: `backend/tests/release_engine_cases.rs`
- Modify: `scripts/test-release-engine`
- Modify: `.github/workflows/release-integration.yml`
- Modify: `integration-tests/release/semantic-assertions.ts`
- Modify: `backend/tests/process_engine_v3.rs`

**Interfaces:**
- Consumes: Tasks 1–3 dynamic snapshot, published case sources, binary hash, and adapter preflight.
- Produces: release report with runtime `engineResourceSnapshot`, binary SHA-256, discovered/importable/non-importable counts, and representative outcomes; no expected commit/hash/count.
- Produces: candidate workflow that checks out the current default eig-rs branch and recursively initializes submodules before building its `tensoreval` binary.

- [ ] **Step 1: Write failing release-script/semantic tests.** Require non-empty root RSS and bundle sets, assert report counts are internally consistent, and remove exact 84/97 and pinned SHA/artifact inputs. Add executable behavior coverage for missing/incompatible current engine configuration.

- [ ] **Step 2: Run deterministic release-focused tests RED.** Run `cd backend && cargo test --test process_engine_v3 --features e2e-test` and the semantic assertion test command discovered from `frontend/package.json`; expected failures identify the old fixed workflow/report contract.

- [ ] **Step 3: Implement dynamic release evidence.** Publish the discovered snapshot before importing all examples; report non-importable diagnostics without physical paths; require at least one RSS and bundle; keep representative execution checks but select existing cases by capability/study rather than fixed total counts.

- [ ] **Step 4: Replace pinned artifact workflow.** Checkout eig-rs's default branch at workflow runtime with submodules, build release `tensoreval`, set `TENSOREVAL_BIN/TENSOREVAL_CWD`, and record binary/snapshot evidence without `engine_artifact_run_id` or `EXPECTED_TENSOREVAL_SHA256`.

- [ ] **Step 5: Run deterministic release-focused tests GREEN.** Repeat Step 2; expected all tests pass.

- [ ] **Step 6: Run current local real-engine gate.** Load `backend/.env` through the existing release command and run `./scripts/test-release-engine`; expected pass when current eig-rs is compatible, otherwise preserve the exact current-engine error as the task failure to fix or report.

- [ ] **Step 7: Commit.** Commit Task 4 files with message `ci: test the current eig-rs release`.

### Task 5: Contracts, documentation, and full verification

**Files:**
- Modify: `AGENTS.md`
- Modify: `docs/{architecture.md,agent-prompt-skills-design.md,v3-deployment.md,integration-testing-design.md,development-status.md}` as applicable
- Modify: `integration-tests/catalog/features.yaml`
- Modify: other current design documents that still describe pinned eig-rs hashes/counts
- Include: `docs/superpowers/specs/2026-10-24-dynamic-eig-rs-resources-design.md`
- Include: `docs/superpowers/plans/2026-10-24-dynamic-eig-rs-resources.md`

**Interfaces:**
- Consumes: final runtime/release behavior from Tasks 1–4.
- Produces: one consistent operator and feature contract for latest configured eig-rs, restart-based snapshot refresh, immutable published sources, and explicit errors.

- [ ] **Step 1: Update authoritative docs and feature traceability.** Remove pinned commit/hash/exact-count language; document dynamic startup discovery, limits, restart semantics, published case tree, binary replacement error, release workflow, and no automatic eig-rs mutation.

- [ ] **Step 2: Run formatting and backend suite.** Run `cd backend && cargo fmt --all -- --check && cargo test --features e2e-test`; expected all tests pass.

- [ ] **Step 3: Run frontend and browser suites.** Run `cd frontend && npm ci && npm test -- --run && npm run build && npm run test:e2e`; expected all tests pass.

- [ ] **Step 4: Run contract and repository checks.** Run `./scripts/check-functional-coverage --require-pr`, `./scripts/check-functional-coverage --require-release`, `git diff --check`, and inspect `git status --short`; expected 100% required coverage, clean diff checks, and only the user's `backend/resources/init_users.csv` remains outside task commits.

- [ ] **Step 5: Commit.** Commit documentation/catalog/plan/spec files with message `docs: define dynamic eig-rs release contract`.

- [ ] **Step 6: Request whole-branch review and verify fixes.** Build the review package from the pre-plan base through HEAD, dispatch one fresh reviewer, apply at most one RED→GREEN fix pass for Critical/Important findings, and rerun the complete deterministic suite plus the current real-engine gate.

