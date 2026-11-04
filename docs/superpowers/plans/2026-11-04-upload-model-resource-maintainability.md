# Upload, Model Version, and Data Resource Maintainability Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. Use `superpowers:test-driven-development` for each behavior change and `superpowers:verification-before-completion` before reporting completion.

**Goal:** Give upload confirmation, upload candidate policy, model draft import, data resource lifecycle, alias policy, and preview tokens one clear owner each without changing the public HTTP/JSON contract.

**Architecture:** Introduce a dedicated `UploadConfirmationService` and `UploadConfirmationRepo`, select an explicit ordinary-upload or case-switch strategy once per confirmation, and leave `DataResourceService`/`DataResourceRepo` responsible only for resources. Extract pure alias/token/candidate policies before moving orchestration, then split parsing and resource lifecycle implementation into child modules. Preserve existing SQL, transaction order, stable errors, DTOs, and frontend behavior.

**Tech Stack:** Rust 2024, Axum, Diesel/SQLite, Tokio, Arrow/Parquet, React/TypeScript, Vitest, Playwright.

**Design:** `docs/superpowers/specs/2026-11-04-upload-model-resource-maintainability-design.md`

---

## Execution rules

- Work directly on `master`; do not create a worktree.
- Before each task, inspect `git status --short` and preserve unrelated user changes.
- Use red/green/refactor for every behavior-affecting step. Pure ownership moves may start with characterization tests and must run those tests before and after the move.
- Do not change public routes, JSON fields, database schema, stable error codes, or frontend interaction in this phase.
- Keep all `tensoreval` calls in `backend/src/adapters/`.
- Commit each task separately. Do not combine a failing intermediate state with the next task.
- Maintain an execution ledger in this plan or the implementation report with command, result, and any justified deviation.

## Task 1: Freeze compatibility behavior at the current seams

**Files:**

- Modify: `backend/tests/data_resources.rs`
- Modify: `backend/tests/upload_batches.rs`
- Modify: `backend/tests/case_import.rs`

### Step 1: Add the missing characterization tests

Add focused tests proving:

1. ordinary confirmation applies model candidates to the draft before attempting resource publication, so a resource alias conflict may leave a model receipt and `partially_confirmed` state;
2. case `cancel` discards the batch;
3. case `replace` and `save_as_new` remain compatibility no-ops for publication, but different actions remain different idempotency payloads;
4. an unexpired legacy `dr-delete_...` or `dr-replace_...` token whose hash is stored in the preview table still confirms successfully;
5. startup recovery never promotes an unstarted confirmation claim and does finish a started, committed child publication.

Use existing fixtures and direct SQL only to arrange legacy persisted state; do not bypass the application service for the assertion.

### Step 2: Run the narrow tests and confirm they pass before refactoring

Run:

```bash
cd backend
cargo test --features e2e-test --test data_resources
cargo test --features e2e-test --test upload_batches
cargo test --features e2e-test --test case_import
```

Expected: all characterization tests pass against the current implementation. If a test exposes behavior different from the approved design, stop and update the design before implementation.

### Step 3: Commit

```bash
git add backend/tests/data_resources.rs backend/tests/upload_batches.rs backend/tests/case_import.rs
git commit -m "test: characterize upload confirmation compatibility"
```

## Task 2: Extract alias, opaque token, and candidate policy primitives

**Files:**

- Create: `backend/src/application/resource_alias.rs`
- Create: `backend/src/application/opaque_token.rs`
- Create: `backend/src/application/upload_candidate_policy.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/application/uploads.rs`
- Modify: `backend/src/application/data_resources.rs`
- Modify: `backend/src/application/versions.rs`
- Modify: `backend/tests/data_resources.rs`
- Modify: `backend/tests/version_deletion.rs`
- Test: inline unit tests in each new module

### Step 1: Write failing unit tests for the pure policies

Cover:

- alias normalization preserves the current backend rule exactly: trim, ASCII lowercase, spaces/hyphens to underscores;
- `default_alias` strips the extension and falls back to `data` when the result is not an RSS identifier;
- identifier validation accepts only ASCII letter/underscore first, then ASCII alphanumeric/underscore;
- `ResourceAliasPolicy` permits a matching Data/Struct facade input and rejects model, function, result, `__*`, `tmp`, `result`, and `output` names;
- opaque token generation produces distinct 64-character lowercase hex values;
- token hashing is stable and never stores plaintext;
- presented-token validation accepts bounded legacy prefixed values for existing previews but rejects empty or oversized input;
- candidate policy covers legal mutable states, selected/purpose combinations, replacement action requirements, and child-status-to-final-status cases.

Run:

```bash
cd backend
cargo test --lib resource_alias
cargo test --lib opaque_token
cargo test --lib upload_candidate_policy
```

Expected: fail because the modules do not yet exist.

### Step 2: Implement the three modules minimally

Implement:

- an `AliasError`/policy result that callers map to their existing stable codes;
- `generate_token`, `hash_token`, and bounded `validate_presented_token` helpers using `getrandom` and SHA-256;
- pure candidate/state functions without repository or service dependencies.

Do not add a generic utilities module. Keep current string values at database boundaries.

### Step 3: Replace duplicate callers

- Replace both Rust `normalize_alias` functions with the shared implementation.
- Replace data-resource deletion/replacement token generation and hashing.
- Replace version deletion token generation/hashing, retaining the version endpoint's existing 64-hex requirement for newly issued tokens.
- Keep legacy prefixed token acceptance only for already persisted data-resource previews.
- Route existing candidate checks through the pure policy where behavior is identical; keep repository CAS checks as the concurrency backstop.

### Step 4: Run narrow regressions

```bash
cd backend
cargo fmt --all -- --check
cargo test --lib resource_alias
cargo test --lib opaque_token
cargo test --lib upload_candidate_policy
cargo test --features e2e-test --test data_resources
cargo test --features e2e-test --test version_deletion
cargo test --features e2e-test --test upload_batches
```

Expected: pass; legacy and new preview tokens both work as specified.

### Step 5: Commit

```bash
git add backend/src/application backend/tests/data_resources.rs backend/tests/version_deletion.rs backend/tests/upload_batches.rs
git commit -m "refactor: centralize resource policies and preview tokens"
```

## Task 3: Give confirmation transactions a dedicated repository

**Files:**

- Create: `backend/src/db/repositories/upload_confirmation.rs`
- Modify: `backend/src/db/repositories/mod.rs`
- Modify: `backend/src/db/repositories/data_resource.rs`
- Modify: `backend/src/db/repositories/case_switch.rs`
- Modify: `backend/src/application/uploads.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/tests/upload_batches.rs`
- Modify: `backend/tests/data_resources.rs`
- Modify: any test importing confirmation methods from `DataResourceRepo`

### Step 1: Change one repository behavior test to the target owner

Update the single-executor confirmation test to instantiate `UploadConfirmationRepo`. Run:

```bash
cd backend
cargo test --features e2e-test --test upload_batches one_confirmation_claim_cannot_acquire_the_execution_lock_twice
```

Expected: compile failure because `UploadConfirmationRepo` is not defined.

### Step 2: Introduce `UploadConfirmationRepo`

Move without semantic rewrite:

- `claim_confirmation`;
- `begin_confirmation`;
- `end_confirmation`;
- `mark_child_failed`;
- `finish_batch`;
- `publish_resources` and its transaction helper/types;
- rename `publish_models`/`publish_models_on` to `apply_model_candidates_to_draft`/`apply_model_candidates_to_draft_on`;
- `switch_case` and `case_switch_state` ownership.

Keep exact transaction boundaries, SQL predicates, user scoping, artifact ref changes, receipt JSON, and child status updates. Move transaction-only input/result types with the new repository. Leave resource query/update/replacement/deletion types in `data_resource.rs`.

### Step 3: Rewire upload recovery

Change `UploadBatchServiceDependencies.confirmation_repo` from `DataResourceRepo` to `UploadConfirmationRepo`. Startup recovery, discard, expiry, and settle must use the new owner without behavior change.

### Step 4: Migrate direct repository tests

Replace only ownership references in tests. Preserve every assertion around execution locks, claims, expiry, recovery, reservations, and artifacts.

### Step 5: Verify the move

```bash
cd backend
cargo fmt --all -- --check
cargo test --features e2e-test --test upload_batches
cargo test --features e2e-test --test data_resources
cargo test --features e2e-test --test case_import
```

Also run:

```bash
rg -n "claim_confirmation|begin_confirmation|end_confirmation|finish_batch|apply_model_candidates_to_draft" backend/src/db/repositories
```

Expected: confirmation methods are implemented only by `UploadConfirmationRepo`; `DataResourceRepo` no longer mutates confirmation state or workspace drafts.

### Step 6: Commit

```bash
git add backend/src/db/repositories backend/src/application backend/tests
git commit -m "refactor: isolate upload confirmation repository"
```

## Task 4: Move confirmation orchestration out of `DataResourceService`

**Files:**

- Create: `backend/src/application/upload_confirmation.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/application/data_resources.rs`
- Modify: `backend/src/application/data_mapping.rs`
- Modify: `backend/src/api/data_resources.rs`
- Modify: `backend/src/api/error.rs` or the actual `ApiError` conversion module
- Modify: `backend/tests/common/mod.rs` and/or `backend/tests/common/fixtures.rs`
- Modify: all tests calling `data_resource_service.confirm_upload`, `publish_candidates`, or `publish_replacement`

### Step 1: Write the target service test first

Convert one concurrent confirmation test to call:

```rust
app.state.upload_confirmation_service.confirm(...)
```

Run the exact test and confirm a compile failure because the state field/service does not exist.

### Step 2: Define the service and stable error mapping

Move into `upload_confirmation.rs`:

- `ReplacementAction`;
- `UploadConfirmation` DTO unchanged;
- internal request/claim structures;
- payload hashing;
- claim/replay/wait logic;
- original source artifact commit;
- child publication orchestration;
- persisted receipt construction.

Create `UploadConfirmationError` with conversions from upload, resource, artifact, repository, and IO errors. Its `code()` must map exactly to the stable codes currently emitted through `DataResourceError`. Add/adjust `ApiError` conversion without changing HTTP statuses or response bodies.

### Step 3: Implement the explicit publication strategy

Select once from batch source kind:

```rust
enum PublicationStrategy {
    OrdinaryUpload,
    CaseSwitch,
}
```

Rules:

- `Cancel` discards before strategy execution;
- ordinary upload applies model candidates first, then publishes resources;
- ordinary retry skips already published children;
- case switch calls the single atomic repository transaction;
- case `Replace`/`SaveAsNew` are included in the payload hash but deliberately ignored by the case strategy;
- only display/lineage code may inspect source kind after strategy selection.

### Step 4: Assemble and route the service

- Construct `DataResourceService` before `UploadConfirmationService`.
- Add `upload_confirmation_service` to `AppState`.
- Give `DataMappingService` the new service for candidate publication.
- Update `POST /api/upload-batches/:id/confirm` to call it.
- Preserve the subsequent upload message publication and JSON wrapper.

### Step 5: Remove old ownership

Delete confirmation methods and fields (`upload_repo`, `uploads`, confirmation-only `data_dir`) from `DataResourceService` when no remaining resource operation uses them. Keep `ArtifactService`, snapshot, reader, and resource repo as required by resource behavior and study overlays.

### Step 6: Migrate all callers and run behavior tests

```bash
cd backend
rg -n "data_resource_service\.(confirm_upload|publish_candidates|publish_replacement)" src tests
cargo fmt --all -- --check
cargo test --features e2e-test --test data_resources
cargo test --features e2e-test --test upload_batches
cargo test --features e2e-test --test case_import
cargo test --features e2e-test --test model_imports
cargo test --features e2e-test --test study_configurations
cargo test --features e2e-test --test workflow_retries
cargo test --features e2e-test --test managed_workflows
cargo test --features e2e-test --test scripted_agent
cargo test --features e2e-test --test result_deletion
cargo test --features e2e-test --test tool_manifest
```

Expected: the `rg` command returns no old service calls; all tests pass with identical DTOs and codes.

### Step 7: Commit

```bash
git add backend/src/application backend/src/api backend/tests
git commit -m "refactor: isolate upload confirmation orchestration"
```

## Task 5: Split upload format processing from batch lifecycle

**Files:**

- Create: `backend/src/application/upload_processing.rs`
- Modify: `backend/src/application/uploads.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/tests/upload_batches.rs`
- Modify: `backend/tests/case_import.rs`
- Modify: `backend/tests/model_imports.rs`

### Step 1: Add/identify characterization coverage per processor

Ensure the existing tests cover at least one success and one relevant failure for:

- DSS;
- case projection;
- archive/DSS tree;
- CSV/Excel/Parquet tabular parsing;
- ambiguous parse settings;
- artifact cleanup after parse failure.

Add only genuinely missing behavior tests. Run the relevant exact tests before moving code.

### Step 2: Extract `UploadProcessor`

Move format dispatch and parsing/materialization methods from `uploads.rs` into `upload_processing.rs` behind an internal `UploadProcessor` with explicit dependencies:

- upload repository;
- artifact service;
- engine adapter;
- engine snapshot;
- data directory;
- archive limits.

Keep intake/reservation, lifecycle, decision mutation, recovery, expiry, discard, and public DTO/error facade in `uploads.rs`. The processor returns the same settled candidates/diagnostics and must not start processes directly outside adapters.

### Step 3: Remove helper duplication/dead code

- Reuse shared alias policy.
- Keep source fingerprint/stable ID helpers with the processor if only it uses them.
- Delete only helpers proven unused by `rg` and compiler warnings.

### Step 4: Verify

```bash
cd backend
cargo fmt --all -- --check
cargo test --features e2e-test --test upload_batches
cargo test --features e2e-test --test case_import
cargo test --features e2e-test --test model_imports
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: no process invocation appears outside adapters, and upload behavior remains unchanged.

### Step 5: Commit

```bash
git add backend/src/application backend/tests
git commit -m "refactor: separate upload processing from lifecycle"
```

## Task 6: Split data resource replacement and deletion lifecycle

**Files:**

- Create: `backend/src/application/data_resource_replacement.rs`
- Create: `backend/src/application/data_resource_deletion.rs`
- Create: `backend/src/application/data_resource_parquet.rs`
- Modify: `backend/src/application/data_resources.rs`
- Create: `backend/src/db/repositories/data_resource_replacement.rs`
- Create: `backend/src/db/repositories/data_resource_deletion.rs`
- Modify: `backend/src/db/repositories/data_resource.rs`
- Modify: `backend/tests/data_resources.rs`

### Step 1: Run lifecycle characterization tests

Run exact tests for:

- replacement preview/confirm/cancel/expiry;
- stale content and schema mismatch;
- deletion impact changed/invalid token/idempotent deleted response;
- artifact release and reference retention.

Add a missing exact test before moving any method if one of those invariants is uncovered only indirectly.

### Step 2: Extract application child modules

Declare physical child files from `data_resources.rs` so they can implement the existing `DataResourceService` without widening field visibility:

- replacement owns structured preview, confirm, expiry, and replacement DTOs;
- deletion owns impact preview, confirm, impact value, and deletion DTOs;
- parquet owns Struct preview encoding helpers;
- `data_resources.rs` retains service construction, error mapping, query/update/download/promotion, shared view/read-source helpers, and the temporarily retained study overlay methods.

Do not introduce a second service facade or change API imports.

### Step 3: Extract repository child modules

Declare child files from `data_resource.rs` and move replacement/deletion transaction methods plus their private row/input/result types. Keep one `DataResourceRepo`, one ownership check path, and one artifact reference implementation.

Remove the duplicate session filter in `DataResourceRepo::list`.

### Step 4: Verify

```bash
cd backend
cargo fmt --all -- --check
cargo test --features e2e-test --test data_resources
cargo test --features e2e-test --test study_configurations
cargo test --features e2e-test --test managed_workflows
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: all resource behavior passes; `data_resources.rs` no longer contains confirmation, replacement transaction, deletion transaction, or Struct Parquet codec implementations.

### Step 5: Commit

```bash
git add backend/src/application backend/src/db/repositories backend/tests/data_resources.rs
git commit -m "refactor: split data resource lifecycle modules"
```

## Task 7: Isolate model version deletion without merging publish semantics

**Files:**

- Create: `backend/src/db/repositories/version_deletion.rs`
- Modify: `backend/src/db/repositories/version.rs`
- Modify: `backend/src/application/versions.rs`
- Modify: `backend/tests/version_deletion.rs`
- Modify: `backend/tests/model_versions.rs`
- Modify: `backend/tests/workspace_versions.rs`

### Step 1: Freeze deletion behavior

Ensure exact tests cover active-version rejection, busy-run rejection, impact change, invalid/expired token, artifact release, run/result tombstones, and idempotent public behavior where defined.

Run:

```bash
cd backend
cargo test --features e2e-test --test version_deletion
```

### Step 2: Move repository deletion implementation

Declare `version_deletion.rs` as a child of `version.rs` and move:

- deletion impact calculation;
- deletability assertion;
- preview persistence;
- delete transaction;
- deletion-only row/result helpers.

Keep workspace loading, draft publication, version content hash, activation, and publish receipt semantics in `version.rs`. Do not reuse upload model-candidate application as version publication, and do not merge their idempotency receipts.

### Step 3: Verify

```bash
cd backend
cargo fmt --all -- --check
cargo test --features e2e-test --test version_deletion
cargo test --features e2e-test --test model_versions
cargo test --features e2e-test --test workspace_versions
cargo clippy --all-targets --all-features -- -D warnings
```

### Step 4: Commit

```bash
git add backend/src/db/repositories/version.rs backend/src/db/repositories/version_deletion.rs backend/src/application/versions.rs backend/tests
git commit -m "refactor: isolate model version deletion"
```

## Task 8: Update current design documentation and traceability

**Files:**

- Modify: `docs/development-status.md`
- Modify: `docs/api-design.md`
- Modify: `docs/db-design.md`
- Modify: `docs/frontend-design.md` only if implementation evidence warrants a compatibility note
- Modify: `docs/superpowers/specs/2026-11-04-upload-model-resource-maintainability-design.md` only for justified implementation deviations
- Modify: `integration-tests/catalog/features.yaml` only if public behavior changed (not expected)

### Step 1: Document actual ownership

Record:

- `UploadConfirmationService` as the only confirmation orchestrator;
- `UploadConfirmationRepo` as the claim/lock/child publish/case switch owner;
- ordinary versus case transaction semantics;
- alias/token policy locations;
- compatibility status of routes/DTOs/errors/schema;
- test commands and any known external eig-rs release-gate issue.

Do not copy historical plan status over current documentation.

### Step 2: Run documentation/traceability checks

```bash
cd ..
./scripts/check-functional-coverage --require-pr
git diff --check
```

If no public behavior changed, confirm the feature catalog diff is empty. If it did change unexpectedly, stop and update design, catalog, behavior tests, and the corresponding current design doc before proceeding.

### Step 3: Commit

```bash
git add docs integration-tests/catalog/features.yaml
git commit -m "docs: document upload and resource ownership"
```

## Task 9: Full verification and focused code review

**Files:**

- Modify only files required by verified failures or review findings

### Step 1: Run the complete repository gates

```bash
cd backend
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --features e2e-test

cd ../frontend
npm test -- --run
npm run build

cd ..
./scripts/check-functional-coverage --require-pr
git diff --check
git status --short
```

Record exact counts and distinguish code failures from the known external eig-rs adjacent-case assertion. Do not weaken tests or release gates.

### Step 2: Audit ownership mechanically

```bash
rg -n "data_resource_service\.(confirm_upload|publish_candidates|publish_replacement)" backend
rg -n "impl DataResourceRepo" backend/src/db/repositories/case_switch.rs backend/src/db/repositories/upload_confirmation.rs
rg -n "fn normalize_alias|fn random_token|fn token\(" backend/src/application
rg -n "Command::new|tokio::process|std::process::Command" backend/src --glob '!adapters/**'
```

Expected:

- no old confirmation calls on `DataResourceService`;
- case switch is owned by `UploadConfirmationRepo`;
- one alias implementation and one shared preview-token generator;
- no new direct process invocation outside adapters.

### Step 3: Request code review and address findings

Use `superpowers:requesting-code-review` with the design, plan, commit range, and verification evidence. Fix critical/important findings with a failing test first where behavior is involved, rerun affected narrow tests, then rerun the full gates.

### Step 4: Final completion commit if review produced changes

```bash
git add <review-fix-files>
git commit -m "fix: address upload resource refactor review"
```

Do not create an empty commit if review found nothing.

## Completion criteria

- `DataResourceService` has no upload confirmation orchestration.
- `DataResourceRepo` has no confirmation lock, workspace draft import, batch finish, or case switch ownership.
- ordinary and case confirmation semantics are selected once and covered by behavior tests.
- alias normalization, candidate policy, and preview token generation each have one Rust owner.
- upload parsing and resource deletion/replacement are in focused child modules without weakening privacy.
- version publication remains distinct from upload candidate-to-draft import.
- public routes, DTOs, schema, frontend behavior, and stable error codes are unchanged.
- all available repository gates pass, with any external-resource failure documented rather than suppressed.
