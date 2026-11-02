# Managed Study Conversation Order Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Present managed studies as AI acknowledgement and configuration, then workflow, then a unified script-and-results outcome.

**Architecture:** The backend emits a deterministic acknowledgement before the durable study configuration card and suppresses the provider tail after a successful `prepare_study`. The frontend keeps persisted message order unchanged, but projects a successful workflow and its matching script message into one visual timeline item; the embedded script retains its durable message anchor.

**Tech Stack:** Rust/Axum/rig/Diesel, React/TypeScript/Vitest/Testing Library, YAML functional catalog.

**Spec:** `docs/superpowers/specs/2026-11-02-managed-study-conversation-order-design.md`

## Global Constraints

- Messages remain ordered by stable numeric ID; do not sort by mutable timestamps or message type.
- Do not change the messages schema, public routes, stable message types, or script/run resource contracts.
- Preserve `scriptMessageId` location, focus, and highlight behavior.
- Only a successful workflow and an in-window script whose resource ID equals `workflow.card.scriptId` may be visually grouped.
- Keep ordinary, unmatched, failed-workflow, upload, and manual script messages standalone.

## Review Focus

- A provider tail arriving after successful `prepare_study` must not create a late assistant message; covered by Task 1 scripted-agent test.
- Provider finalization failure after successful `prepare_study` must retain the deterministic acknowledgement without a generic failure; covered by Task 1 scripted-agent test.
- An anchored history window containing only one side of the workflow/script pair must not hide it; covered by Task 2 projection test.
- Failed workflow scripts must remain standalone so retry and diagnosis behavior do not move; covered by Task 2 projection test.
- Script location from run history must focus the embedded `data-message-id`; covered by Task 2 component test.

---

### Task 1: Deterministic study acknowledgement ordering

**Files:**
- Modify: `backend/tests/scripted_agent.rs`
- Modify: `backend/tests/workspace_tools.rs`
- Modify: `backend/src/tools/prepare_study.rs`
- Modify: `backend/src/application/chat.rs`

**Interfaces:**
- Consumes: `EventWriter::assistant`, `EventWriter::study_config`, and rig `MultiTurnStreamItem` ordering.
- Produces: one deterministic assistant message before each successful `study-config` message; `TurnOutcome.configuration_prepared` suppresses later provider text and generic finalization errors.

- [ ] **Step 1: Write failing backend behavior tests**

Assert that a successful scripted `prepare_study` produces an assistant acknowledgement with the facade title and configuration ID before the `study-config` message, and that the scenario's unique provider tail is absent. Extend the provider-finalization failure test to assert the same ordering and no generic Agent failure. Extend the direct tool test to pin the acknowledgement/card order.

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `cargo test --features e2e-test --test scripted_agent study_power_flow_searches_then_prepares_a_configuration_without_a_script -- --exact`

Run: `cargo test --features e2e-test --test workspace_tools study_tools_search_explicit_functions_and_prepare_persisted_cards -- --exact`

Expected: FAIL because the acknowledgement is currently emitted after the configuration card and the provider tail is persisted.

- [ ] **Step 3: Emit the acknowledgement before the card**

In `PrepareStudyTool::call`, after `StudyConfigurationService::prepare` returns, call `writer.assistant` with `已为您创建“{title}”研究配置卡（ID: {id}），请在卡片中检查并确认输入、参数和结果。`, then publish the `StudyConfigCard`. Map acknowledgement persistence failure to a stable internal tool error.

- [ ] **Step 4: Suppress only post-prepare provider tails**

In `ChatService::consume`, retain assistant text emitted before successful `prepare_study`, ignore assistant text emitted after that tool result, and remove the old end-of-turn fallback acknowledgement. Keep successful preparation authoritative over provider finalization failures.

- [ ] **Step 5: Run focused and complete backend tests**

Run the two focused commands from Step 2, then `cargo test --features e2e-test`.

Expected: PASS with no duplicate or late acknowledgement.

### Task 2: Causal workflow outcome projection

**Files:**
- Create: `frontend/src/components/messages/timelineProjection.ts`
- Create: `frontend/src/components/messages/timelineProjection.test.ts`
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/components/messages/MessageItem.tsx`
- Modify: `frontend/src/components/messages/WorkflowProgressCard.tsx`
- Modify: `frontend/src/components/messages/WorkflowProgressCard.test.tsx`

**Interfaces:**
- Produces: `projectMessageTimeline(messages: Message[]): TimelineItem[]`, where `TimelineItem` contains `message` and optional `linkedScript`.
- Consumes: `WorkflowProgressMessage.card.state`, `WorkflowProgressMessage.card.scriptId`, and `ScriptMessage.script.id`.

- [ ] **Step 1: Write failing projection tests**

Test that a successful workflow absorbs exactly its matching later script, while unmatched scripts, failed-workflow scripts, and a script without its workflow remain standalone and in stable relative order.

- [ ] **Step 2: Run the projection test and verify RED**

Run: `npm test -- --run src/components/messages/timelineProjection.test.ts`

Expected: FAIL because the projector does not exist.

- [ ] **Step 3: Implement the minimal pure projector**

Implement `projectMessageTimeline` without mutating input. Build matches only from successful in-window workflow messages with non-empty `scriptId`; attach the exact script message and omit only that matched standalone script.

- [ ] **Step 4: Write failing outcome component tests**

Extend `WorkflowProgressCard.test.tsx` to pass a linked script and assert a `研究成果` region whose embedded script anchor precedes `运行结果`. Assert the embedded anchor retains `data-message-id` and highlight styling.

- [ ] **Step 5: Run the component test and verify RED**

Run: `npm test -- --run src/components/messages/WorkflowProgressCard.test.tsx`

Expected: FAIL because linked scripts and the outcome region are not rendered.

- [ ] **Step 6: Render projected timeline items and the outcome region**

Use `projectMessageTimeline` in `App.tsx`. Pass the optional linked script and current highlight ID through `MessageItem`. In `WorkflowProgressCard`, render successful linked scripts and `WorkflowResultsCard` under a labelled `研究成果` section, retaining `data-testid="message-script"`, `data-message-id`, focusability, run action, and highlight class.

- [ ] **Step 7: Run frontend tests and build**

Run: `npm test -- --run`

Run: `npm run build`

Expected: PASS with no TypeScript errors.

### Task 3: Current contracts, traceability, and final verification

**Files:**
- Modify: `docs/frontend-design.md`
- Modify: `docs/api-design.md`
- Modify: `docs/development-status.md`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Consumes: implemented behavior and named tests from Tasks 1–2.
- Produces: current design and functional coverage entries describing deterministic acknowledgement ordering and grouped successful outcomes.

- [ ] **Step 1: Update current design documents and catalog assertions**

Document the deterministic assistant-before-card ordering, suppression of the provider tail, successful workflow outcome grouping, and preservation of durable script anchors. Add the new Vitest projection test and updated scripted-agent assertions to the existing managed-study features.

- [ ] **Step 2: Run formatting and static checks**

Run: `cargo fmt --all -- --check`

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Run: `npm test -- --run`

Run: `npm run build`

Expected: PASS.

- [ ] **Step 3: Run functional coverage gate**

Run: `./scripts/check-functional-coverage --require-pr`

Expected: PASS with full feature/contract traceability.

- [ ] **Step 4: Review the final diff and commit**

Confirm no unrelated user files are included, then commit the spec, plan, implementation, tests, current docs, and catalog together with a focused message.
