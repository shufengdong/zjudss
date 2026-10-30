# Case Import Card Placement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the model workspace the only full-card location for case imports while keeping compact, locatable durable chat summaries and leaving ordinary uploads in the data-files workspace.

**Architecture:** Route actionable or failed upload batches by the authoritative `UploadBatchView.sourceKind`: `case` batches render in `WorkspacePanel`, all others render in `DataResourcePanel`. `UploadBatchCard` becomes a summary-only durable message, and `App` opens the correct tab from the batch ID.

**Tech Stack:** React 18, TypeScript, Vitest, Testing Library, Tailwind CSS

**Spec:** `docs/superpowers/specs/2026-10-29-case-import-card-placement-design.md`

## Global Constraints

- Use `UploadBatchView.sourceKind`; do not infer placement from names or candidate purposes.
- Keep the existing backend upload and durable-message contracts unchanged.
- Full fixed controls appear in one workbench location; chat retains a visible durable summary.

## Review Focus

- A restored case batch must still route to the model workspace after refresh.
- A normal upload containing user-selected model candidates must remain in the data-files workspace because its source is `upload`.
- Confirmed, discarded, and expired case batches must disappear from the workspace while their durable summary remains in history; failed batches retain diagnostics and actions.
- A message whose batch has not loaded must retain a usable generic navigation action.
- A failed case batch must remain visible with its diagnostic in the model workspace.

---

### Task 1: Route full cards by authoritative batch source

**Files:**
- Modify: `frontend/src/components/workspace/WorkspacePanel.tsx`
- Modify: `frontend/src/components/resources/DataResourcePanel.tsx`
- Modify: `frontend/src/components/ResultPanel.tsx`
- Test: `frontend/src/components/workspace/WorkspacePanel.test.tsx`
- Test: `frontend/src/components/resources/DataResourcePanel.test.tsx`

**Interfaces:**
- Consumes: `UploadBatchView.sourceKind`, `UploadBatchActions`, and the existing closed-status set.
- Produces: `WorkspacePanel` props `batches?: UploadBatchView[]` and `batchActions?: UploadBatchActions`; exclusive full-card placement by source.

- [ ] **Step 1: Write failing component tests**

Add tests proving that an actionable `case` batch appears only in `WorkspacePanel`, a failed case retains its diagnostic there, a confirmed case is hidden, and `DataResourcePanel` renders an ordinary/replacement batch but excludes a case batch even when its candidates include `data_resource`.

- [ ] **Step 2: Run focused tests and verify the placement assertions fail**

Run: `npm test -- --run src/components/workspace/WorkspacePanel.test.tsx src/components/resources/DataResourcePanel.test.tsx`

Expected: FAIL because `WorkspacePanel` does not accept batches and `DataResourcePanel` currently renders every non-closed batch.

- [ ] **Step 3: Implement exclusive routing**

Extend `WorkspacePanel` with optional `batches` and `batchActions`, render actionable or failed `sourceKind === 'case'` batches before the model sections, filter case batches out of `DataResourcePanel`, and pass the shared data through `ResultPanel`.

- [ ] **Step 4: Run focused tests and verify they pass**

Run: `npm test -- --run src/components/workspace/WorkspacePanel.test.tsx src/components/resources/DataResourcePanel.test.tsx`

Expected: PASS.

### Task 2: Replace the duplicate chat card with a compact locator

**Files:**
- Modify: `frontend/src/components/messages/UploadBatchCard.tsx`
- Modify: `frontend/src/components/messages/MessageItem.tsx`
- Modify: `frontend/src/types/message.ts`
- Modify: `frontend/src/App.tsx`
- Test: `frontend/src/components/messages/UploadBatchCard.test.tsx`
- Test: `frontend/src/App.test.tsx`

**Interfaces:**
- Consumes: live `UploadBatchView` when restored and durable `UploadBatchMessage.card` as fallback.
- Produces: `MessageHandlers.onOpenUploadBatch(batchId: string)` and a summary-only message with source-aware navigation text.

- [ ] **Step 1: Write failing message and application tests**

Assert that the message never renders candidate checkboxes or confirmation buttons, reports live status/count/diagnostic, labels case navigation “前往模型工作区”, labels ordinary navigation “前往数据文件”, keeps “查看并确认” when the live batch is unavailable, hides stale actions for closed batches, and that applying a case opens the model-workspace tab.

- [ ] **Step 2: Run focused tests and verify they fail for duplicate controls and wrong routing**

Run: `npm test -- --run src/components/messages/UploadBatchCard.test.tsx src/App.test.tsx`

Expected: FAIL because the message embeds `UploadBatchDecisionCard` and case application currently opens `resources`.

- [ ] **Step 3: Implement compact summaries and ID-aware navigation**

Remove `UploadBatchDecisionCard` from the message component, derive current summary fields from the live batch when present, pass the batch ID to the handler, route by the restored batch `sourceKind`, and open `workspace` immediately after `applyExample` succeeds.

- [ ] **Step 4: Run focused tests and verify they pass**

Run: `npm test -- --run src/components/messages/UploadBatchCard.test.tsx src/App.test.tsx`

Expected: PASS.

### Task 3: Align documentation and functional traceability

**Files:**
- Modify: `docs/frontend-design.md`
- Modify: `docs/v4-deployment.md`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Consumes: final tested UI behavior from Tasks 1 and 2.
- Produces: current V4 behavior and feature-to-test traceability.

- [ ] **Step 1: Update current design and deployment checks**

Document exclusive placement, compact durable summaries, source-aware navigation, and the release check that one case batch has only one full card.

- [ ] **Step 2: Update feature catalog evidence**

Point the upload/case-workspace behavior to the component/application tests added above without changing public route or message-type registries.

- [ ] **Step 3: Run full frontend and repository verification**

Run: `npm test -- --run && npm run build` from `frontend`, then `./scripts/check-functional-coverage --require-pr` from the repository root.

Expected: all commands exit 0.
