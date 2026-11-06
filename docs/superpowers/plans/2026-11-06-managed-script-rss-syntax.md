# Managed Script RSS Syntax Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ensure managed multi-result scripts receive authoritative RSS syntax docs, fail closed without bracketed destructuring, and never re-execute unchanged repairs.

**Architecture:** Extend snapshot knowledge with a bounded ScriptAuthor document selector, pass those refs to generation and repair, and strengthen the existing structural contract before the adapter boundary. Preserve candidate audit attempts while short-circuiting same-hash repairs before engine execution.

**Tech Stack:** Rust 2024, Tokio, serde, Diesel/SQLite, existing ScriptAuthor/WorkflowWorker/SimEngine test doubles.

**Spec:** `docs/superpowers/specs/2026-11-06-managed-script-rss-syntax-design.md`

## Global Constraints

- All tensoreval calls remain inside `backend/src/adapters/`.
- No REST/SSE DTO, database schema, frontend, Agent tool, or eig-rs resource-tree change.
- Engine documents come only from the published immutable snapshot and remain bounded.
- Every production change follows RED → GREEN before commit.

## Review Focus

- A single selected result remains valid without brackets.
- Two or more selected results require brackets and exact ordered result names.
- Missing syntax docs cannot make an invalid script reach the engine.
- Same-hash repairs remain auditable but never consume another engine call.
- Changed valid repairs retain the existing maximum execution budget.

---

### Task 1: Authoritative ScriptAuthor document context

**Files:**
- Modify: `backend/src/engine_resources/knowledge.rs`
- Modify: `backend/src/agent/script_author.rs`
- Modify: `backend/src/application/workflow_worker.rs`
- Modify: `backend/src/testing/engine_fixture.rs`
- Modify: `backend/src/testing/scripted_author.rs`
- Test: `backend/tests/script_repair.rs`

**Interfaces:**
- Produces: `EngineKnowledge::script_author_docs(function, max_chars) -> Vec<DocSearchHit>`.
- Produces: `ScriptRepairRequest.engine_docs: Vec<EngineDocRef>`, matching generation request semantics.
- Consumes: current snapshot `EngineKnowledge` and `locked.function`.

- [ ] Add failing knowledge and workflow tests proving required syntax headings appear in both generation and repair requests.
- [ ] Run focused tests; expect missing syntax refs/request capture failure.
- [ ] Implement bounded required-section-first selection, request propagation, and explicit single/multiple result author rules.
- [ ] Run focused tests; expect PASS.
- [ ] Commit as `fix: provide RSS syntax to script author`.

### Task 2: Multi-result and unchanged-repair execution guards

**Files:**
- Modify: `backend/src/application/managed_rss.rs`
- Modify: `backend/src/application/script_contract.rs`
- Modify: `backend/src/application/workflow_worker.rs`
- Test: `backend/tests/script_repair.rs`

**Interfaces:**
- Consumes: Task 1 repair context and existing `WorkflowAttempt.script_content_hash`.
- Produces: bracket metadata on `AssignmentCall`; same-hash repair uses existing `contract_rejected` and `repair_contract_violated`.

- [ ] Add failing tests: unbracketed multi-result binding is rejected; unchanged repairs create attempts but only the initial candidate reaches the engine.
- [ ] Update existing budget tests so changed candidates still exercise four engine calls.
- [ ] Run focused tests; expect the new assertions to fail for current behavior.
- [ ] Implement bracket tracking/validation and parent-hash short-circuit before execution.
- [ ] Run focused tests; expect PASS.
- [ ] Commit as `fix: reject invalid managed result bindings`.

### Task 3: Documentation, coverage, and full verification

**Files:**
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/development-status.md`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Consumes: Task 1 document context and Task 2 execution guards.
- Produces: current behavior/evidence documentation and machine-readable test traceability.

- [ ] Update current docs and the existing managed-workflow feature evidence without adding routes, DTOs, errors, tools, or migrations.
- [ ] Run `cargo fmt --all -- --check`, all-target/all-feature Clippy, full `cargo test --features e2e-test`, and `./scripts/check-functional-coverage --require-pr`.
- [ ] Run `git diff --check`; expect PASS.
- [ ] Commit as `docs: record managed RSS syntax guards`.
