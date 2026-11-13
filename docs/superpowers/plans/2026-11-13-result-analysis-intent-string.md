# Result Analysis Intent String Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Bailian `run_result_analysis` calls reliably serialize the audit-only intent as a bounded string instead of an unconstrained JSON object.

**Architecture:** Narrow only the LLM tool boundary: `Args.intent` becomes `String`, is trimmed and validated before any analysis side effect, then is passed into the existing application service as `Value::String`. The application/database/API keep their generic JSON representation because comparison-generated analysis records also use structured intent objects. Agent guidance receives one complete JSON/RSS example; execution inputs, script semantics, expected-output enforcement and adapters remain unchanged.

**Tech Stack:** Rust, serde/serde_json, rig tool JSON Schema, Axum application services, Markdown Agent bundle, Vitest/Playwright release evidence.

**Spec:** `docs/superpowers/specs/2026-11-13-result-analysis-intent-string-design.md`

## Global Constraints

- `intent` is trimmed and must contain 1 through 1000 Unicode characters.
- `intent` remains audit metadata and never controls script execution, input selection or output publication.
- `script`, `inputs` and `expectedOutputs` retain their current contracts.
- No malformed-JSON repair, provider-specific parser or global retry-policy change is introduced.
- All tensoreval execution remains behind `backend/src/adapters/`; no server path, result row, SQL or Python script enters the tool contract.
- A public tool/skill behavior change must update deterministic behavior tests, Agent resource hashes, the feature catalog and current design documentation.
- Work directly on `master` as required by `AGENTS.md`; preserve unrelated user changes if any appear.

## Review Focus

- Object-valued `intent` must be rejected by deserialization while a string is accepted.
- Empty/whitespace-only and 1001-character strings must fail before an analysis row or artifact is created.
- A valid 1000-character Unicode string must be preserved after trimming and stored as a JSON string.
- Agent guidance must show RSS syntax and a complete tool argument object; it must not suggest Python imports.
- Agent manifest, skill index and `result-analysis` content hashes must agree after the version bump.

---

### Task 1: String tool contract, Agent guidance and traceability

**Files:**
- Modify: `backend/src/tools/run_result_analysis.rs`
- Modify: `backend/tests/tool_manifest.rs`
- Modify: `backend/tests/scripted_agent.rs`
- Modify: `backend/tests/agent_knowledge.rs`
- Modify: `backend/resources/agent/v4/skills/result-analysis.md`
- Modify: `backend/resources/agent/v4/skill-index.json`
- Modify: `backend/resources/agent/v4/manifest.json`
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/api-design.md`
- Modify: `docs/development-status.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `integration-tests/release/real-dependencies.spec.ts`
- Test: inline unit tests in `backend/src/tools/run_result_analysis.rs`

**Interfaces:**
- Consumes: existing `AnalysisRequest { intent: serde_json::Value, ... }` and `AnalysisService::run` unchanged.
- Produces: `run_result_analysis::Args { intent: String, ... }`; private `normalize_intent(String) -> Result<String, DomainToolError>`; tool schema `intent.type = "string"`, `minLength = 1`, `maxLength = 1000`.

- [ ] **Step 1: Write failing contract and boundary tests**

Add assertions that the tool schema describes a bounded string, `Args` accepts `"intent":"copy voltage"`, rejects object-valued intent, and `normalize_intent` trims valid Unicode while rejecting whitespace-only and 1001-character values with `analysis_intent_invalid`.

- [ ] **Step 2: Run the focused tests and capture RED**

Run: `cd backend && cargo test --features e2e-test run_result_analysis`

Expected: FAIL because the schema still declares an object, object input is still accepted, and `normalize_intent` does not exist.

- [ ] **Step 3: Implement the minimal string boundary**

Change `Args.intent` to `String`; implement `normalize_intent` with trimmed Unicode length `1..=1000`; call it before skill lookup or analysis side effects; pass `Value::String(intent)` into the unchanged `AnalysisRequest`.

- [ ] **Step 4: Update deterministic Agent behavior tests**

Change every scripted `run_result_analysis` call to a string intent; assert the successful unique-current-result journey persists a JSON string intent and still freezes exactly one dataset. Keep explicit multi-input coverage and object rejection in `tool_manifest.rs`.

- [ ] **Step 5: Update Agent guidance and immutable resource hashes**

Bump `result-analysis` from `1.2.0` to `1.2.1`. Add one complete call example using a quoted intent string, RSS `filter`/`select` syntax, one stable dataset input and matching `expectedOutputs`; explicitly forbid Python imports. Recompute the skill SHA-256, skill-index SHA-256 and manifest references.

- [ ] **Step 6: Synchronize current docs, catalog and real-provider assertion**

Document the audit-only string contract and stable error. Add release-journey assertions that the successful `run_result_analysis` trace has a non-empty string `intent`, RSS script text and one expected output. Update AGENT/ANALYSIS catalog evidence without adding a route or message type.

- [ ] **Step 7: Run focused GREEN tests**

Run: `cd backend && cargo test --features e2e-test run_result_analysis`

Run: `cd backend && cargo test --features e2e-test --test scripted_agent natural_language_analysis_uses_the_unique_current_result_dataset`

Run: `cd backend && cargo test --features e2e-test --test agent_knowledge --test tool_manifest`

Expected: all pass, and resource hash verification succeeds.

- [ ] **Step 8: Commit the implementation checkpoint**

Stage only the files listed in this task plus the approved spec and this plan. Commit message: `fix(agent): stabilize result analysis intent arguments`.

### Task 2: Repository verification and real-provider evidence

**Files:**
- Verify only; modify implementation files only if a new failing test proves an implementation defect.

**Interfaces:**
- Consumes: Task 1 string contract and updated Agent bundle.
- Produces: deterministic gate results plus clearly separated optional Bailian runtime evidence.

- [ ] **Step 1: Run deterministic backend gates**

Run: `cd backend && cargo fmt --all -- --check`

Run: `cd backend && cargo clippy --all-targets --all-features -- -D warnings`

Run: `cd backend && cargo test --features e2e-test -- --skip current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count`

Expected: all commands exit 0; the skip remains limited to the already documented external eig-rs snapshot assertion.

- [ ] **Step 2: Run frontend and traceability gates**

Run: `cd frontend && npm test -- --run`

Run: `cd frontend && npm run build`

Run: `./scripts/check-functional-coverage --require-pr`

Expected: Vitest/build pass; report the coverage gate exactly as produced rather than reusing stale route evidence.

- [ ] **Step 3: Restart and optionally reproduce against Bailian**

Restart backend/frontend with the configured local `.env`. If the existing owned session and dataset 22 are still live, resubmit the same explicit 0.99–1.01 p.u. threshold request once and verify one successful `run_result_analysis` call creates an analysis run. Do not loop or automatically repair a provider failure; record the exact provider response separately from deterministic test results.

- [ ] **Step 4: Audit final state**

Run: `git diff --check`, `git status --short`, and inspect the final commit/resource hashes. Confirm no unrelated files were included and both services remain healthy if restarted.
