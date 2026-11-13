# Current Result Natural-Language Analysis Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the latest successful simulation result available as bounded Agent context so a natural-language result-processing request selects exactly one matching DataFrame and executes the existing controlled analysis path.

**Architecture:** Add an application-owned current-result resolver that reads only the current user's session, skips unusable runs, and builds a 64 KiB bounded metadata summary. Inject that summary into the per-turn system prompt while keeping `run_result_analysis` unchanged: the model must still submit one stable dataset ID and alias for the default case. Tighten the V4 system/skill policy and cover the real regression with deterministic Agent and release tests.

**Tech Stack:** Rust, Axum application services, Diesel/SQLite repositories, rig Agent prompts/tools, Snappy Parquet metadata, scripted Agent tests, Playwright release journey, YAML functional coverage.

**Spec:** `docs/superpowers/specs/2026-11-13-current-result-natural-language-analysis-design.md`

## Global Constraints

- All `tensoreval` execution remains behind `backend/src/adapters/`; this change adds no process launch.
- Repository reads remain explicitly user-scoped; no physical path, result row, SQL, command, or ECharts option enters the prompt or tool boundary.
- `run_result_analysis` continues to require explicit stable `datasetId + alias`; its JSON schema and tool count remain unchanged.
- Default matching binds one dataset only; explicit existing multi-input analysis remains supported up to its current 20-input limit.
- Prompt context is at most 64 KiB, includes at most 32 column summaries per dataset, truncates text fields at 256 Unicode characters, and exposes truncation flags.
- Only the latest successful run among the latest 20 live runs that still has at least one live dataset becomes current context.
- Work directly on `master` per `AGENTS.md`; do not create a worktree.
- Preserve the user's existing uncommitted `backend/Cargo.toml`, `backend/Cargo.lock`, and Agent hash edits. This feature adds no dependency. Do not stage or commit any files unless the user later requests a commit; update overlapping hash files incrementally from their current contents.

## Review Focus

- Oversized schema/result inventories must stay within 64 KiB and mark every omitted dataset/column/field so the Agent cannot infer uniqueness from missing data; Task 1 tests this.
- A newer failed/running/success-with-no-live-result run must not hide the latest usable successful result; Task 1 tests this.
- Current-result lookup failure or absence must not be converted into the false claim “尚未运行”; Tasks 2 and 3 test the null/fallback policy.
- Multiple plausible datasets must produce a choice request and no analysis write; Task 4 tests this.
- Explicit multi-dataset analysis must retain the unchanged tool contract despite the new default-single-result rule; Tasks 3 and 4 assert this in bundle and tool-contract tests.

---

### Task 1: Resolve and bound the current result context

**Files:**
- Create: `backend/src/application/current_result_context.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/tests/scripted_agent.rs`
- Test: `backend/src/application/current_result_context.rs`
- Test: `backend/tests/current_result_context.rs`

**Interfaces:**
- Consumes: `RunService::list(&CurrentUser, session_id, 20)` and `RunService::datasets(&CurrentUser, run_id)`.
- Produces: `CurrentResultContext::resolve(user, session_id, runs) -> Result<Option<CurrentResultContext>, RunError>` and serializable `CurrentResultContext`, `CurrentResultDatasetContext`, and `CurrentResultColumnContext` crate-private DTOs.
- Produces: `CurrentResultContext::prompt_json() -> String`, guaranteed to be at most `64 * 1024` bytes.

- [ ] **Step 1: Write the two failing end-to-end regression tests before production changes**

Add `natural_language_analysis_uses_the_unique_current_result_dataset` and `ambiguous_current_results_require_selection_without_analysis` to `backend/tests/scripted_agent.rs`. The first seeds a successful `bus_voltage` run and asserts its stable dataset facts appear in the next Agent system prompt before a one-input `run_result_analysis` call. The second seeds two voltage-plausible results and asserts both appear in context while no analysis write occurs for an underspecified request.

- [ ] **Step 2: Run the regression tests and verify RED**

Run: `cd backend && cargo test --features e2e-test --test scripted_agent current_result -- --nocapture`

Expected: FAIL because the per-turn prompt has no current-result context.

- [ ] **Step 3: Write failing bounded-context unit tests**

Add tests named `prompt_context_caps_columns_text_and_total_bytes` and `prompt_context_marks_dataset_and_column_truncation`. Construct result schemas with more than 32 columns, 300-character names/dtypes/units, and enough datasets to pressure the 64 KiB budget. Assert UTF-8-safe 256-character field truncation, explicit truncation flags, dataset IDs/names retained in stable order where included, and serialized length `<= 65_536`.

- [ ] **Step 4: Run the unit tests and verify RED**

Run: `cd backend && cargo test --features e2e-test current_result_context::tests -- --nocapture`

Expected: FAIL because `current_result_context` and its DTO/budget builder do not exist.

- [ ] **Step 5: Implement the bounded DTO builder**

Implement constants `CURRENT_RESULT_RUN_LIMIT: i64 = 20`, `CURRENT_RESULT_PROMPT_MAX_BYTES: usize = 65_536`, `CURRENT_RESULT_MAX_COLUMNS: usize = 32`, `CURRENT_RESULT_MAX_TEXT_CHARS: usize = 256`, and `CURRENT_RESULT_MAX_DATASETS: usize = 100`. Build dataset identities before columns, append column summaries only while the complete JSON remains inside the global budget, and mark all local/global truncation without reading Parquet rows.

- [ ] **Step 6: Run the unit tests and verify GREEN**

Run: `cd backend && cargo test --features e2e-test current_result_context::tests -- --nocapture`

Expected: PASS.

- [ ] **Step 7: Write failing resolver integration tests**

Add tests named `latest_usable_success_is_selected_for_the_owned_session` and `resolver_returns_none_when_no_success_has_live_results`. Seed user-owned runs so newer entries include running, failed, and succeeded-with-deleted-results states; include an older successful run with a live `bus_voltage` dataset and a foreign user's newer success. Assert only the owned live dataset is returned with its persisted schema/row count/size and foreign state is invisible.

- [ ] **Step 8: Run the resolver tests and verify RED**

Run: `cd backend && cargo test --features e2e-test --test current_result_context -- --nocapture`

Expected: FAIL because `CurrentResultContext::resolve` is absent.

- [ ] **Step 9: Implement the resolver and module ownership**

Add `pub(crate) mod current_result_context` in `application/mod.rs`. Iterate the user-scoped latest 20 runs newest-first, consider only `status == "succeeded"`, fetch live datasets, return the first non-empty bounded context, and propagate repository/storage errors so ChatService can distinguish lookup failure from a real empty result.

- [ ] **Step 10: Run Task 1 tests**

Run: `cd backend && cargo test --features e2e-test current_result_context -- --nocapture`

Expected: all matching unit and integration tests PASS.

### Task 2: Inject current result facts into each Agent turn

**Files:**
- Modify: `backend/src/agent/prompt.rs`
- Modify: `backend/src/application/chat.rs`
- Modify: `backend/tests/agent_knowledge.rs`
- Test: `backend/src/agent/prompt.rs`

**Interfaces:**
- Consumes: Task 1 `CurrentResultContext` and `prompt_json()`.
- Produces: `PromptAssembler::assemble(knowledge, resources, current_result: Option<&CurrentResultContext>) -> String` with a `# Current result` section containing either the bounded object or JSON `null`.
- ChatService resolves the context before `AgentFactory::build`; lookup errors are logged without paths and inject `null` rather than failing the user turn.

- [ ] **Step 1: Write failing PromptAssembler tests**

Add tests `prompt_includes_bounded_current_result_facts` and `prompt_uses_null_when_current_result_is_unavailable`. Assert the prompt contains the exact run/dataset IDs, result/schema metadata and truncation fields, contains no storage path or row values, and its current-result section is `null` when absent.

- [ ] **Step 2: Run prompt tests and verify RED**

Run: `cd backend && cargo test --features e2e-test prompt::tests -- --nocapture`

Expected: FAIL because `PromptAssembler::assemble` has no current-result parameter/section.

- [ ] **Step 3: Extend PromptAssembler and ChatService minimally**

Update both existing call sites to pass `Some(&context)` or `None`. In `ChatService::run`, resolve using the request principal/session before building the Agent; on error emit only a bounded server diagnostic and continue with `None`. Do not persist this context as a message or add it to historical messages.

- [ ] **Step 4: Run prompt and existing knowledge tests**

Run: `cd backend && cargo test --features e2e-test --test agent_knowledge -- --nocapture`

Expected: PASS, including the existing catalog inventory assertions with `None` supplied by the test.

### Task 3: Tighten natural-language routing policy without changing tool authority

**Files:**
- Modify: `backend/resources/agent/v4/system.md`
- Modify: `backend/resources/agent/v4/skills/result-analysis.md`
- Modify: `backend/resources/agent/v4/skill-index.json`
- Modify: `backend/resources/agent/v4/manifest.json`
- Modify: `backend/tests/agent_knowledge.rs`
- Modify: `backend/tests/tool_manifest.rs`

**Interfaces:**
- Consumes: prompt JSON field `currentResultContext` from Task 2.
- Produces: `result-analysis@1.2.0` policy requiring unique matching, one default input, truncation fallback, threshold/unit/component clarification, and same-turn analysis execution when inputs are unambiguous.
- Preserves: 23 tools, five skills, `result-analysis` required tool set, and the existing `run_result_analysis` argument schema with explicit `inputs`.

- [ ] **Step 1: Add failing embedded-bundle policy assertions**

Assert the result-analysis version is `1.2.0`; the system/skill text contains `currentResultContext`, unique-result matching, one-input default, truncation fallback to `list_result_datasets`, “越限” as a derived-analysis trigger, and prohibition on claiming unsupported threshold/point chart controls. Assert explicit multi-input analysis remains allowed and the tool parameter schema is unchanged.

- [ ] **Step 2: Run bundle/tool tests and verify RED**

Run: `cd backend && cargo test --features e2e-test --test agent_knowledge --test tool_manifest -- --nocapture`

Expected: FAIL on the new policy assertions while existing tool schema assertions remain green.

- [ ] **Step 3: Update system and result-analysis skill policy**

Set `result-analysis` to version `1.2.0` and document the exact routing matrix: pure chart request uses workbench; derived/filter/aggregate/over-limit requests use result analysis; missing threshold/unit/List component asks one necessary question; unique complete context executes; ambiguous or truncated evidence queries tools/asks; no context calls history then datasets and never invents “尚未运行”. Preserve explicit multi-input joins.

- [ ] **Step 4: Recompute bundle hashes from current working-tree bytes**

Use `shasum -a 256` only to read hashes. Apply the new result-analysis content hash to the current `skill-index.json`, then its complete-file hash and the changed `system.md` hash to the current `manifest.json` using `apply_patch`. Do not restore or overwrite the pre-existing hash changes for other skills.

- [ ] **Step 5: Run bundle/tool tests and verify GREEN**

Run: `cd backend && cargo test --features e2e-test --test agent_knowledge --test tool_manifest -- --nocapture`

Expected: PASS with embedded hash verification, 23 tools, five skills, and unchanged analysis tool schema.

### Task 4: Prove the regression and ambiguity behavior end to end

**Files:**
- Modify: `backend/tests/scripted_agent.rs`
- Modify: `integration-tests/release/real-dependencies.spec.ts`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Consumes: Tasks 1-3 prompt context and unchanged `run_result_analysis` tool.
- Produces: deterministic evidence that one matching dataset is bound and ambiguous datasets do not trigger analysis; release evidence that a real provider can use current-result context without an explicit dataset ID in the user message.

- [ ] **Step 1: Complete the unique-result scripted scenario using production behavior**

Use only existing public run/script/analysis services and ScenarioEngine fixtures; do not insert a fake tool result or bypass the real `run_result_analysis` implementation. Assert `result_analysis_inputs` freezes only the selected dataset ID.

- [ ] **Step 2: Complete and run the ambiguity scenario**

Finish the already-red `ambiguous_current_results_require_selection_without_analysis` fixture with two voltage-plausible result names/columns, send an underspecified voltage-analysis request, and script only a choice response. Assert both stable IDs appear in current context, no `run_result_analysis` tool call occurs, and `result_analysis_runs` remains empty.

Run: `cd backend && cargo test --features e2e-test --test scripted_agent ambiguous_current_results_require_selection_without_analysis -- --nocapture`

Expected: PASS after implementation.

- [ ] **Step 3: Run both original regression tests and verify GREEN**

Run: `cd backend && cargo test --features e2e-test --test scripted_agent current_result -- --nocapture`

Expected: both tests that failed in Task 1 now PASS without test-only bypasses.

- [ ] **Step 4: Update real-provider release coverage**

In the existing result-analysis segment, remove the dataset ID and ready-made RSS script from the user's message. Ask the provider to derive a copy from the uniquely named current `bus_voltage` result, while retaining trace assertions for `load_agent_skill`, metadata inspection as needed, and `run_result_analysis`. Keep the deterministic expected output checks and existing source-deletion lineage journey.

- [ ] **Step 5: Register behavior evidence**

Add both Rust test symbols to `AGENT-001` and `ANALYSIS-001`; update assertions/fixtures to cover bounded current context, unique single-result binding, ambiguity without writes, and unchanged explicit multi-input analysis. Do not add or rename routes, messages, tools, skills, views, or stable errors.

- [ ] **Step 6: Run focused behavior and coverage checks**

Run: `cd backend && cargo test --features e2e-test --test scripted_agent natural_language_analysis -- --nocapture`

Run from repository root: `./scripts/check-functional-coverage --require-pr`

Expected: focused tests PASS and coverage reports 100% with no duplicate/missing contract registration.

### Task 5: Reconcile current documentation and verify the repository

**Files:**
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/development-status.md`
- Modify: `docs/integration-testing-design.md`

**Interfaces:**
- Consumes: verified behavior and exact test names from Tasks 1-4.
- Produces: current-state documentation; historical plans/specs remain records and do not replace current design docs.

- [ ] **Step 1: Update current docs**

Document the server-authored current-result context, 64 KiB/32-column/256-character bounds, latest-20 selection, unique-result default, ambiguity/truncation fallback, explicit multi-input preservation, `result-analysis@1.2.0`, and the chart/analysis separation. Record only verification actually run in this implementation.

- [ ] **Step 2: Run formatting and static analysis**

Run: `cd backend && cargo fmt --all -- --check`

Run: `cd backend && cargo clippy --all-targets --all-features -- -D warnings`

Expected: exit 0 with no warnings.

- [ ] **Step 3: Run the full backend suite**

Run: `cd backend && cargo test --features e2e-test`

Expected: exit 0 and zero failed tests.

- [ ] **Step 4: Run frontend regression gates**

Run: `cd frontend && npm test -- --run`

Run: `cd frontend && npm run build`

Expected: all Vitest tests pass and production build exits 0; no frontend source change is expected.

- [ ] **Step 5: Run deterministic browser and traceability gates**

Run: `cd frontend && npm run test:e2e`

Run from repository root: `./scripts/check-functional-coverage --require-pr`

Expected: Playwright suite passes and functional/public-contract coverage remains 100%.

- [ ] **Step 6: Audit the final diff and dirty-file preservation**

Run: `git status --short` and `git diff --check`. Verify `backend/Cargo.toml` and `backend/Cargo.lock` are byte-for-byte untouched by this implementation, unrelated existing Agent hash changes were not reverted, no server path/result rows entered prompt fixtures, and all implementation changes map to the approved spec.
