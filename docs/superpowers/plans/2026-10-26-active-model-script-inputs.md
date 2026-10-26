# Active Model Script Inputs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make planned Agent scripts consume the current active model's actual DataFrame aliases and reject re-importing or overwriting those inputs.

**Architecture:** `inspect_workspace` remains the first source of active table evidence. `StudyPlanningService` freezes the actual active table names into the plan and refuses to issue a ready plan when the canonical model inputs required by governed studies are absent; `ScriptService` then treats that frozen list as the authority when accepting a planned Agent script. Prompt and skill resources explain the same contract without embedding a fixed table list.

**Tech Stack:** Rust, Axum application services, rig tools, serde, embedded Markdown Agent bundle, Cargo integration tests.

**Spec:** `docs/superpowers/specs/2026-10-26-active-model-script-inputs-design.md`

## Global Constraints

- Available aliases come from the current session's active version, never a Prompt constant, draft, history, example, or server path.
- A ready plan binds model version ID, content hash, engine snapshot and actual input aliases.
- Planned Agent scripts never import, read files, redefine active inputs, run automatically, or persist after deterministic validation fails.
- Manual text and uploaded scripts keep their current validation and persistence behavior.
- Work directly on `master`; preserve all existing uncommitted frontend and backend changes and do not commit until the user asks.

## Review Focus

- Active version changes after planning: existing stale-plan checks must still reject the script.
- An active version has only a subset of canonical model tables: return all missing aliases in one `missing_model_data` plan.
- A planned script calls the right function but contains `import_dss`: reject it without a script record or script message.
- A planned script assigns to an active input alias: reject it even if it has no external path.
- A valid planned script derives `pf_para` and result variables while reading active aliases: accept it and keep explicit-click execution.

---

### Task 1: Freeze actual active aliases in study plans

**Files:**
- Modify: `backend/src/application/study_planning/contracts.rs`
- Modify: `backend/src/application/study_planning/facts.rs`
- Modify: `backend/src/application/study_planning/policy.rs`
- Test: `backend/tests/study_planning.rs`
- Test: `backend/tests/workspace_tools.rs`

**Interfaces:**
- Produces: `ModelStudyFacts.available_aliases: BTreeSet<String>`.
- Produces: `StudyPlan.input_aliases: Vec<String>` serialized as `inputAliases` for every plan result.
- Produces: `ReadyStudyPlan.input_aliases: Vec<String>` frozen into `studyPlanId` state and audit hash.

- [x] **Step 1: Write failing planning tests**

Add tests asserting that `inputAliases` equals the sorted actual `active.tables[].tableName` list, a ready plan carries the same list, and a governed study with missing canonical model aliases returns one `missing_model_data` result listing every missing alias without a `studyPlanId`.

- [x] **Step 2: Run the focused tests and verify they fail because alias fields and missing-table handling do not exist**

Run: `cargo test --test study_planning --test workspace_tools --features e2e-test`

- [x] **Step 3: Implement dynamic alias collection and ready-plan gating**

Collect table names from the `VersionSnapshot` already read by `extract_active_model_facts`; sort through `BTreeSet`; copy them into `StudyPlan` and `ReadyStudyPlan`. Before `finish_ready` signs a governed plan, compare the active aliases with the application/adapter canonical model contract and return `MissingParameter` entries for all absent tables.

- [x] **Step 4: Run the focused tests and verify they pass**

Run: `cargo test --test study_planning --test workspace_tools --features e2e-test`

### Task 2: Reject invalid planned Agent scripts before persistence

**Files:**
- Modify: `backend/src/application/scripts.rs`
- Test: `backend/tests/scripted_agent.rs`
- Test: `backend/tests/scripts.rs`

**Interfaces:**
- Consumes: `ReadyStudyPlan.input_aliases` from Task 1.
- Produces: planned Agent script validation that requires `StaticScriptValidation.valid`, requires every detected model input to belong to the frozen aliases, and rejects output bindings that overwrite a frozen input alias.

- [x] **Step 1: Write failing behavior tests**

Add a scripted Agent scenario with a valid current `studyPlanId` whose script calls the planned function after `import_dss`; assert the tool call fails and no script/message is persisted. Add a planned script assigning to `bus`; assert rejection. Preserve tests proving a correct direct-alias script succeeds and generic/manual/upload compatibility remains unchanged.

- [x] **Step 2: Run the focused tests and verify the unsafe planned scripts are currently persisted**

Run: `cargo test --test scripted_agent --test scripts --features e2e-test`

- [x] **Step 3: Implement fail-closed planned-script validation**

In `ScriptService::create`, after static validation and before any artifact commit, reject planned Agent scripts when validation is invalid, when `required_aliases` is outside `plan.input_aliases`, or when `outputs` intersects `plan.input_aliases`. Return the existing stable `script_error` boundary with a bounded actionable message so the Agent can regenerate in the same turn.

- [x] **Step 4: Run the focused tests and verify they pass**

Run: `cargo test --test scripted_agent --test scripts --features e2e-test`

### Task 3: Teach the Agent the dynamic input contract

**Files:**
- Modify: `backend/resources/agent/v3/system.md`
- Modify: `backend/resources/agent/v3/skills/study-planning.md`
- Modify: `backend/resources/agent/v3/skills/rss-generation.md`
- Modify: `backend/resources/agent/v3/skill-index.json`
- Modify: `backend/resources/agent/v3/manifest.json`
- Modify: `backend/tests/agent_knowledge.rs`
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/development-status.md`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Consumes: `inspect_workspace.active.tables` and `plan_study.inputAliases` from Task 1.
- Produces: Agent bundle `v3.3.0`; `study-planning` and `rss-generation` skills `v1.2.0` with regenerated SHA-256 values.

- [x] **Step 1: Write failing bundle behavior assertions**

Assert that the embedded skills require comparing `inspect_workspace.active.tables` with `plan_study.inputAliases`, directly reference those aliases, forbid imports/file reads/input rebinding, and never enumerate a fixed model table list.

- [x] **Step 2: Run the focused knowledge tests and verify they fail on the old bundle**

Run: `cargo test --test agent_knowledge --features e2e-test`

- [x] **Step 3: Update system policy and skills**

State the runtime injection contract and dynamic workflow using the field names above. Bump bundle/skill versions, regenerate skill/index/system hashes, and keep tool count and engine contract unchanged.

- [x] **Step 4: Update current design/status documentation and the feature catalog**

Document dynamic aliases, missing-table planning, pre-persistence rejection, and the new behavior-test names; do not change REST route counts or tool counts.

- [x] **Step 5: Run bundle and contract tests**

Run: `cargo test --test agent_knowledge --test contract_manifest --test tool_manifest --features e2e-test`

### Task 4: Full verification and runtime restart

**Files:**
- Verify all files above; no new runtime interface.

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces: a running backend serving bundle `v3.3.0` against the current eig-rs snapshot.

- [x] **Step 1: Run format, lint and full backend tests**

Run: `cargo fmt --all -- --check`

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Run: `cargo test --features e2e-test`

- [x] **Step 2: Run functional coverage validation**

Run: `./scripts/check-functional-coverage --require-pr`

- [x] **Step 3: Restart the backend and check both services**

Run the release backend with the existing `.env`, verify `/health` is `ok`, verify the frontend returns HTTP 200, and confirm startup reports Agent bundle `v3.3.0` with the current engine snapshot.

- [x] **Step 4: Review the final diff**

Confirm no server path enters tool output, no fixed alias list enters Prompt/skills, all pre-existing uncommitted work remains present, and no commit was created.
