# RSSBundle Case Library Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the flat mixed-source case catalog with a manifest-driven directory tree sourced only from `sparrowzz/dss/rssbundle`.

**Architecture:** `EngineResourceRegistry` discovers `.rssbundle/manifest.json` leaves and builds immutable `EngineExample` records containing normalized paths and manifest metadata. The examples API derives a pruned directory tree from those records, while case import and Agent tools consume the same snapshot objects. React renders that tree as a responsive browser and sends the existing stable example ID plus snapshot ID into the unchanged draft-import endpoint.

**Tech Stack:** Rust, Axum, Serde, SHA-256, Tokio/Diesel tests, React 19, TypeScript, Tailwind CSS, Vitest/Testing Library, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-24-rssbundle-case-library-design.md`

## Global Constraints

- Work directly on `master`, as previously requested; do not create a compatibility layer for the unused older contracts.
- `TENSOREVAL_CWD/sparrowzz/dss/rssbundle` is the only case root.
- A case is a `.rssbundle` directory with a direct child named `manifest.json`; intermediate directories are categories.
- Manifest `entry` and `files` are the only authority for imported content; undeclared files are ignored.
- Browser and LLM responses may contain normalized relative paths but never physical paths.
- Engine API calls remain behind `backend/src/adapters/`; this feature does not add a direct engine call.
- Applying a case creates a model import and mutable draft only; it does not publish, activate, run, or create results.
- Existing resource limits remain: 2,048 cases, 10,000 files and 2 GiB per case unless configuration already supplies a stricter value.
- Resource changes take effect after backend restart and produce a new immutable engine resource snapshot.

## Review Focus

- A `.rssbundle` nested inside another `.rssbundle` must stay content of the outer case and never appear as a second catalog leaf; Task 1 pins this.
- A `.rssbundle` directory without `manifest.json` and empty category directories must not create phantom UI nodes; Tasks 1 and 2 pin this.
- Unknown fields in a supported schema-1 manifest must not break discovery, while an unsupported schema or artifact must produce a visible invalid leaf; Task 1 pins this.
- Search matches manifest metadata such as result names and source entry while retaining the full ancestor path; Task 2 pins this.
- A source file changed after snapshot publication must fail before staging any partial model import; Task 3 pins this.

---

## File Structure

- `backend/src/engine_resources/examples.rs`: manifest DTOs, normalized case records, validation, hashes and source-file inventory.
- `backend/src/engine_resources/mod.rs`: rssbundle-only discovery, snapshot identity and publication inventory.
- `backend/src/application/mod.rs`: deterministic schema-1 test snapshot shared by backend and browser tests.
- `backend/src/main.rs`: startup counts for discovered, importable and invalid rssbundle cases.
- `backend/src/api/examples.rs`: catalog filtering, deterministic directory-tree response and detail serialization.
- `backend/src/application/case_import.rs`: verified manifest-declared staging and source-summary provenance.
- `backend/src/tools/search_examples.rs`, `inspect_example.rs`: Agent-facing manifest metadata without physical paths.
- `backend/tests/engine_resources.rs`, `case_import.rs`, `workspace_tools.rs`: deterministic Rust contract coverage.
- `frontend/src/types/workspace.ts`, `api/client.ts`, `hooks/useExamples.ts`: tree and manifest API types/state.
- `frontend/src/components/examples/ExampleTree.tsx`: recursive accessible directory/case browser.
- `frontend/src/components/examples/ExampleLibrary.tsx`, `ExampleDetail.tsx`: responsive selection, detail and apply flow.
- `frontend/src/components/examples/ExampleLibrary.test.tsx`: tree, search, invalid case and narrow-flow behavior.
- `integration-tests/e2e/bundle-examples.spec.ts`: browser-to-Axum-to-draft case journey.
- `backend/tests/release_engine_cases.rs`: current eig-rs discovery/import/replay acceptance gate.
- Current V3 docs and `integration-tests/catalog/features.yaml`: public contract and evidence alignment.

### Task 1: Parse only manifest-backed RSSBundle cases

**Files:**
- Modify: `backend/src/engine_resources/examples.rs`
- Modify: `backend/src/engine_resources/mod.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/main.rs`
- Modify: `backend/tests/engine_resources.rs`

**Interfaces:**
- Produces: public serializable `ExampleManifest`, `ExampleStudy`, `ExampleSource`, `ExampleManifestFile`, and `EngineExample::{category_path,bundle_path,entry_path,manifest}`; `entry_path` and `manifest` are `Option` values for invalid leaves.
- Produces: `ExampleSourceFile { virtual_path, source_path, sha256, role }`, with `manifest.json` using role `manifest`.
- Removes: root-RSS dependency discovery and duplicated `study_type`, `solver`, and `required_features` fields; `source_kind` remains the fixed value `rssbundle`, while valid study metadata comes from `manifest`.
- Consumes: existing `ResourceLimits`, safe-path helpers and immutable snapshot publication.

- [ ] **Step 1: Replace the engine-resource fixture with a schema-1 manifest and write failing discovery assertions**

  Update `fixture()` to create `rssbundle/Grid/123Bus/demo.rssbundle/{manifest.json,main.rss,dss/model.dss}` with artifact `mems.dss-rss-bundle`, source metadata, study metadata, file roles and hashes. Keep a root `demo.rss` as an explicit ignored input. In `discovers_current_tree_deterministically_and_snapshot_changes_with_loaded_bytes`, assert one example, `category_path == ["Grid", "123Bus"]`, `bundle_path == "Grid/123Bus/demo.rssbundle"`, `entry_path.as_deref() == Some("main.rss")`, and every manifest metadata field.

- [ ] **Step 2: Add failing boundary and validation tests**

  Add focused tests named `discovers_only_manifest_backed_rssbundle_leaves`, `supported_manifest_unknown_fields_are_ignored_but_contract_mismatches_are_invalid`, and `bundle_manifest_rejects_duplicate_missing_unsafe_symlink_and_hash_mismatch_files`. Assert root RSS exclusion, missing-manifest exclusion, nested-bundle exclusion, schema/artifact diagnostics, duplicate-path diagnostics, redacted unsafe paths and no absolute path in serialized examples.

- [ ] **Step 3: Run the focused Rust tests and observe RED**

  Run: `cd backend && cargo test --test engine_resources -- --nocapture`

  Expected: failures show the old two-source discovery and missing manifest/path fields.

- [ ] **Step 4: Implement the schema-1 manifest DTOs and rssbundle leaf loader**

  In `examples.rs`, define the public DTOs with `#[serde(rename_all = "camelCase")]` for API output and private raw manifest DTOs with snake-case field mapping. Change `load_examples` to accept only bundle manifest specs, derive `category_path` relative to `rssbundle`, stop traversal at each `.rssbundle` boundary, validate schema `1`, artifact `mems.dss-rss-bundle`, entry membership, unique safe file paths, ordinary files and SHA-256, and retain an invalid leaf for per-case failures. Unknown JSON fields remain accepted.

- [ ] **Step 5: Remove root RSS discovery and include normalized manifest state in snapshot identity**

  In `mod.rs`, delete `discover_root_rss`, call bundle discovery with the full `max_examples`, ensure discovery does not recurse below a `.rssbundle` directory, and hash ID, category path, bundle path, normalized manifest metadata, declared file role/hash, importability and diagnostics. Publication copies only manifest plus declared files. Update the `application/mod.rs` test snapshot to a valid `E2E/Demo/E2E demo.rssbundle` plus one invalid sibling leaf. Update `main.rs` startup output to report discovered/importable/invalid counts.

- [ ] **Step 6: Run formatting and engine-resource tests GREEN**

  Run: `cd backend && cargo fmt --all -- --check && cargo test --test engine_resources`

  Expected: all engine-resource tests pass; published fixture has no `source/sparrowzz/dss/demo.rss`.

- [ ] **Step 7: Commit the resource contract**

  ```bash
  git add backend/src/engine_resources/examples.rs backend/src/engine_resources/mod.rs backend/src/application/mod.rs backend/src/main.rs backend/tests/engine_resources.rs
  git commit -m "feat: discover manifest-backed rssbundle cases"
  ```

### Task 2: Expose a deterministic, searchable example tree

**Files:**
- Modify: `backend/src/engine_resources/examples.rs`
- Modify: `backend/src/api/examples.rs`
- Modify: `backend/tests/case_import.rs`

**Interfaces:**
- Consumes: Task 1 `EngineExample` manifest/path fields.
- Produces: `EngineExample::matches_catalog_query(&self, query: &str, study_type: Option<&str>) -> bool`, shared by HTTP and Agent search.
- Produces: `GET /api/examples` response `{ engineResourceSnapshot, tree }` where `tree` recursively contains `{type:"directory",name,path,children}` or `{type:"example",example}`.
- Produces: `GET /api/examples/{id}` with `categoryPath`, `bundlePath`, `manifest`, role-aware `files`, `treeHash` and `diagnostics`.

- [ ] **Step 1: Write failing API tree and detail tests**

  Replace `case_catalog_hides_non_modelizable_entries_and_apply_is_user_scoped` catalog assertions with `case_catalog_preserves_directories_and_includes_invalid_leaves`. Assert deterministic ancestor nodes, importable and invalid leaves, no `examples` flat array, and no physical paths. Add `case_detail_exposes_normalized_manifest_without_source_paths` covering study, source, inputs and file roles.

- [ ] **Step 2: Write the failing metadata-search test**

  Call `/api/examples?query=voltage` and `?query=<source entry fragment>`; assert each response contains only matching leaves plus their ancestors. Assert an unmatched query returns `tree: []`, and an empty category never appears.

- [ ] **Step 3: Run API tests RED**

  Run: `cd backend && cargo test --test case_import case_catalog -- --nocapture`

  Expected: failures show the old `{examples,nextCursor}` flat response and hidden invalid examples.

- [ ] **Step 4: Implement tree construction and filtering in `api/examples.rs`**

  Add `EngineExample::matches_catalog_query` in `engine_resources/examples.rs` and private serializable `ExampleTreeNode`, `filtered_examples` and `build_tree` helpers in the API. Match title, bundle path, study kind, solver, compatibility, result names and source entry with Unicode lowercase normalization. Sort directories and leaves deterministically and retain invalid examples. Keep `studyType` filtering; ignore/remove `sourceKind` and `limit` from the effective contract.

- [ ] **Step 5: Expand summary/detail serialization**

  Summary includes `categoryPath`, `bundlePath`, `studyType`, `solver`, `compatibility`, `results`, source entry, counts and validation. Detail includes normalized manifest and role-aware declared files. Neither response serializes `source_path` or `engine_root`.

- [ ] **Step 6: Run case API tests GREEN**

  Run: `cd backend && cargo test --test case_import`

  Expected: all case catalog, detail, apply scope and draft tests pass.

- [ ] **Step 7: Commit the HTTP tree contract**

  ```bash
  git add backend/src/engine_resources/examples.rs backend/src/api/examples.rs backend/tests/case_import.rs
  git commit -m "feat: expose hierarchical case catalog"
  ```

### Task 3: Stage exactly the manifest-declared case files

**Files:**
- Modify: `backend/src/application/case_import.rs`
- Modify: `backend/tests/case_import.rs`

**Interfaces:**
- Consumes: Task 1 role-aware `EngineExample.files` and normalized manifest fields.
- Produces: case model-import source summary containing `exampleId`, `title`, `categoryPath`, `bundlePath`, `engineResourceSnapshot`, `manifestSchemaVersion`, `sourceTreeHash`, `caseTreeHash`, and `fileCount`.
- Preserves: existing `accept(...) -> Result<ModelImportView, CaseImportError>` and background `process(...)` signatures.

- [ ] **Step 1: Add failing exact-inventory and provenance tests**

  In `case_import.rs`, create an undeclared file beside the published bundle, apply the case, and assert the accepted import inventory contains only `manifest.json` plus `manifest.files`. Query `model_imports.source_summary_json` and assert the exact provenance fields. Confirm the existing 21-table draft result still succeeds.

- [ ] **Step 2: Add the failing post-publication mutation atomicity test**

  Modify one declared source file in `app.state.engine_snapshot.engine_root`, call the apply endpoint, assert HTTP 422 with `case_not_modelizable`, and assert no model import row or staged artifact was created for the client import ID.

- [ ] **Step 3: Run focused tests RED**

  Run: `cd backend && cargo test --test case_import applying_a_case -- --nocapture`

  Expected: fixture construction or source-summary assertions fail before implementation.

- [ ] **Step 4: Implement strict staging and provenance**

  Iterate the immutable `example.files` inventory only; verify all hashes before staging the first file, then stage them in manifest order with `manifest.json` first. Persist the specified manifest/path provenance in `accept_case`. Do not read the live eig-rs tree.

- [ ] **Step 5: Run model-import and case-import regressions GREEN**

  Run: `cd backend && cargo test --test case_import --test model_imports --test bundle_ingestion`

  Expected: all tests pass and existing upload bundle behavior remains unchanged.

- [ ] **Step 6: Commit strict case staging**

  ```bash
  git add backend/src/application/case_import.rs backend/tests/case_import.rs
  git commit -m "feat: import cases from manifest inventory"
  ```

### Task 4: Align Agent example tools with the catalog

**Files:**
- Modify: `backend/src/tools/search_examples.rs`
- Modify: `backend/src/tools/inspect_example.rs`
- Modify: `backend/tests/workspace_tools.rs`

**Interfaces:**
- Consumes: Task 1 normalized example fields and Task 2 `matches_catalog_query`.
- Produces: `search_examples` results with snapshot ID, stable ID, category/bundle paths, study metadata, source entry and validation.
- Produces: `inspect_example` detail with normalized manifest and role-aware file hashes, without `source_path`.

- [ ] **Step 1: Write failing tool contract tests**

  Add `example_tools_return_manifest_metadata_and_never_physical_paths` to `workspace_tools.rs`. Search by a result name, inspect the returned stable ID, assert category/bundle paths and study/source fields, assert invalid leaves include diagnostics, and recursively assert serialized output excludes the temporary engine root.

- [ ] **Step 2: Run the tool test RED**

  Run: `cd backend && cargo test --test workspace_tools example_tools -- --nocapture`

  Expected: missing path/manifest fields or leaked internal serialization causes failure.

- [ ] **Step 3: Implement explicit Agent DTO serialization**

  Stop serializing `EngineExample` wholesale. Map only the approved fields in each tool, call `matches_catalog_query`, cap search results at 50, and update descriptions to mention directory paths and manifest metadata.

- [ ] **Step 4: Run Agent contract tests GREEN**

  Run: `cd backend && cargo test --test workspace_tools --test tool_manifest --test scripted_agent`

  Expected: all tool registration, invocation and redaction tests pass.

- [ ] **Step 5: Commit Agent tool alignment**

  ```bash
  git add backend/src/tools/search_examples.rs backend/src/tools/inspect_example.rs backend/tests/workspace_tools.rs
  git commit -m "feat: expose case manifest metadata to agent"
  ```

### Task 5: Build the responsive frontend case browser

**Files:**
- Modify: `frontend/src/types/workspace.ts`
- Modify: `frontend/src/api/client.ts`
- Modify: `frontend/src/hooks/useExamples.ts`
- Create: `frontend/src/components/examples/ExampleTree.tsx`
- Modify: `frontend/src/components/examples/ExampleLibrary.tsx`
- Modify: `frontend/src/components/examples/ExampleDetail.tsx`
- Create: `frontend/src/components/examples/ExampleLibrary.test.tsx`

**Interfaces:**
- Consumes: Task 2 `{engineResourceSnapshot,tree}` and detail response.
- Produces: discriminated union `ExampleTreeNode`, `ExampleManifestSummary`, expanded `ExampleSummary`/`ExampleDetail` and hook state `{tree,loading,error,engineResourceSnapshot}`.
- Preserves: `ExampleLibrary.onUse(id,title,snapshot)` so `App.tsx` and the draft-import action remain unchanged.

- [ ] **Step 1: Define failing component fixtures and tree navigation tests**

  In `ExampleLibrary.test.tsx`, mock `listExamples`/`getExample` with `IEEETestCases/123Bus/Run_IEEE123Bus` plus one invalid sibling. Assert top-level categories start expanded, nested folders toggle with `aria-expanded`, leaf selection loads detail, invalid leaves expose diagnostics and cannot be applied, and a valid leaf calls `onUse` with ID/title/snapshot.

- [ ] **Step 2: Add failing search, error, empty and narrow-flow tests**

  Assert query responses render their ancestor path expanded, empty tree shows “没有匹配案例”, rejected fetch shows a retryable error, and detail view has a “返回案例目录” action suitable for the single-column narrow layout.

- [ ] **Step 3: Run component tests RED**

  Run: `cd frontend && npm test -- --run src/components/examples/ExampleLibrary.test.tsx`

  Expected: missing tree types/component and old flat-card UI fail.

- [ ] **Step 4: Add TypeScript API contracts and robust hook state**

  Define the discriminated tree union and normalized manifest types in `workspace.ts`, remove the obsolete `requiredFeatures` property, and make `entryPath`, `studyType`, `solver`, `sourceEntry` and detail `manifest` nullable for invalid leaves. Change `listExamples` to accept only `query` and `studyType` and return the tree response; update `useExamples` to cancel stale queries, expose errors and provide an explicit retry counter/function.

- [ ] **Step 5: Implement `ExampleTree`**

  Create a recursive component using buttons with `aria-expanded` for directories and `aria-current` for the selected example. Key directories by joined normalized path and leaves by stable example ID. Auto-expand top-level nodes and all ancestors present in a non-empty search result.

- [ ] **Step 6: Rebuild library and detail presentation**

  Replace the card grid with responsive directory/detail panes. Detail renders bundle path, source entry, study kind, solver, compatibility, results, generator, file-role counts and diagnostics. Desktop uses two panes; narrow CSS shows the tree or detail with an explicit return action. Keep close and apply actions keyboard accessible.

- [ ] **Step 7: Run frontend tests and build GREEN**

  Run: `cd frontend && npm test -- --run && npm run build`

  Expected: all Vitest files pass and Vite production build succeeds, with only the existing chunk-size warning allowed.

- [ ] **Step 8: Commit the frontend browser**

  ```bash
  git add frontend/src/types/workspace.ts frontend/src/api/client.ts frontend/src/hooks/useExamples.ts frontend/src/components/examples
  git commit -m "feat: browse cases by rssbundle directory"
  ```

### Task 6: Update browser and real-eig-rs acceptance gates

**Files:**
- Modify: `integration-tests/e2e/bundle-examples.spec.ts`
- Modify: `backend/tests/release_engine_cases.rs`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Consumes: Tasks 1–5 complete backend and frontend behavior.
- Produces: deterministic Playwright evidence and release report based only on rssbundle cases.

- [ ] **Step 1: Update the failing Playwright case journey**

  Change `case application uses the fixed action and creates only a mutable draft` to expand `E2E` then `Demo`, select `E2E demo`, assert bundle/source/study manifest detail, apply it, confirm candidate DataFrames, merge, and assert draft-only behavior. Add a narrow viewport pass that selects a leaf, returns to the directory and reopens it.

- [ ] **Step 2: Run the focused browser test RED before rebuilding the e2e fixture contract**

  Run: `cd frontend && npm run test:e2e -- bundle-examples.spec.ts`

  Expected: old browser selectors/API fixture fail until all earlier tasks are integrated.

- [ ] **Step 3: Replace mixed-source release assertions**

  In `release_engine_cases.rs`, remove `root_rss_examples`, RSS run mode and root-RSS representative selection. Report `bundle_examples`, importable/non-importable totals and discovered study types. Require at least one rssbundle and one importable example; select one importable representative for each actually present supported kind among `execution_plan`, `static_power_flow`, `fault_study`, and `harmonic`.

- [ ] **Step 4: Run deterministic browser and release-test compilation GREEN**

  Run: `cd frontend && npm run test:e2e -- bundle-examples.spec.ts`

  Run: `cd backend && cargo test --no-run --test release_engine_cases`

  Expected: Playwright case journey passes and the real-dependency test compiles without root RSS fields.

- [ ] **Step 5: Run the real eig-rs engine gate**

  Run: `./scripts/test-release-engine`

  Expected: all manifest-backed cases are discovered; every importable case imports; representative runs for present supported study kinds succeed; report contains no root RSS count.

- [ ] **Step 6: Update functional coverage evidence and commit**

  Update `EXAMPLE-001` assertions/fixtures for directory tree, manifest detail and draft application, and update the release gate assertion text. Run `./scripts/check-functional-coverage` and expect `gatePassed: true`.

  ```bash
  git add integration-tests/e2e/bundle-examples.spec.ts backend/tests/release_engine_cases.rs integration-tests/catalog/features.yaml
  git commit -m "test: cover hierarchical rssbundle cases"
  ```

### Task 7: Align current documentation and run the final gate

**Files:**
- Modify: `docs/architecture.md`
- Modify: `docs/api-design.md`
- Modify: `docs/frontend-design.md`
- Modify: `docs/development-status.md`
- Modify: `docs/v3-deployment.md`

**Interfaces:**
- Consumes: final behavior from Tasks 1–6.
- Produces: one consistent current V3 contract; historical V2 plans/specs remain untouched.

- [ ] **Step 1: Update current docs to the final contract**

  Document rssbundle-only discovery, directory tree API, manifest fields, invalid-leaf behavior, strict import inventory, Agent metadata and responsive UI. Remove current-status/deployment requirements for root RSS and replace release report descriptions with discovered/importable/invalid rssbundle counts and present study kinds.

- [ ] **Step 2: Scan for live-document contradictions**

  Run: `rg -n "根 RSS|root RSS|root_rss|扁平.*案例|examples.*20|sourceKind.*rss" docs --glob '!superpowers/plans/**' --glob '!superpowers/specs/2026-10-23-v3-dataframe-model-scripts-runs-design.md'`

  Expected: no current-document statement treats root RSS as a case source or the example API as a flat list. Historical design records may retain old facts.

- [ ] **Step 3: Run deterministic backend and frontend gates**

  Run: `cd backend && cargo fmt --all -- --check && cargo test`

  Run: `cd frontend && npm test -- --run && npm run build && npm run test:e2e`

  Run: `./scripts/check-functional-coverage`

  Expected: Rust, Vitest, build, all deterministic Playwright tests and coverage gate pass.

- [ ] **Step 4: Review security and scope diffs**

  Run: `git diff --check && git diff --stat && git status --short`

  Inspect API/tool JSON evidence to confirm there are no absolute paths, and inspect commits to confirm no adapter-boundary violation or unrelated refactor.

- [ ] **Step 5: Commit documentation**

  ```bash
  git add docs/architecture.md docs/api-design.md docs/frontend-design.md docs/development-status.md docs/v3-deployment.md
  git commit -m "docs: align rssbundle case library contracts"
  ```

- [ ] **Step 6: Restart running services and smoke-test the live UI**

  Restart the backend so it builds a fresh engine resource snapshot, keep/restart Vite on port 5173, verify `/health`, log in, open the case library, expand a real eig-rs directory, inspect a case and start its model import. Do not merge candidates during the smoke test if it would alter a user workspace; use the dedicated test user/session.
