# Workspace Model Section Collapse Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the active-model and mutable-draft sections independently collapsible, collapsed by default, while retaining useful model summaries.

**Architecture:** Keep two controlled booleans inside `WorkspacePanel`, reset them only when `sessionId` changes, and derive table count plus total row count from the existing workspace DTO. Reuse the existing historical-version button pattern and preserve all table/open/publish behavior inside the expanded content.

**Tech Stack:** React 19, TypeScript, Tailwind CSS, Testing Library, Vitest.

**Spec:** `docs/superpowers/specs/2026-10-25-workspace-model-section-collapse-design.md`

## Global Constraints

- Work directly on `master`; preserve and exclude unrelated user changes from every commit.
- “当前模型”和“可变草稿” default to collapsed and expand independently.
- The collapsed active summary contains version ID, table count, and total row count; the draft summary contains revision, table count, and total row count.
- A workspace refresh within the same session preserves the user's choices; changing `sessionId` resets both sections to collapsed.
- Empty active/draft states retain their current copy and do not expose meaningless expand buttons.
- Public API, DTO, backend behavior, model lifecycle, and table workbench behavior remain unchanged.
- The frontend design and functional coverage catalog must describe and trace the changed view behavior.

## Review Focus

- Active or draft has zero tables: show `0 张表 · 0 行` and allow expansion without crashing (Task 1 component test).
- Large row totals: use JavaScript number addition and locale formatting without concatenating strings (Task 1 component test).
- Workspace data refreshes in the same session: keep each section's current expanded state (Task 1 component test).
- Session changes while both sections are expanded: collapse both sections and hide stale table controls (Task 1 component test).
- Only one section is toggled: leave the other section's state untouched (Task 1 component test).

---

### Task 1: Add independently collapsible model summaries

**Files:**
- Modify: `frontend/src/components/workspace/WorkspacePanel.test.tsx`
- Modify: `frontend/src/components/workspace/WorkspacePanel.tsx`
- Modify: `docs/frontend-design.md`
- Modify: `integration-tests/catalog/features.yaml`
- Include: `docs/superpowers/specs/2026-10-25-workspace-model-section-collapse-design.md`
- Include: `docs/superpowers/plans/2026-10-25-workspace-model-section-collapse.md`

**Interfaces:**
- Consumes: existing `Workspace.active`, `Workspace.draft`, and each table's numeric `rowCount`.
- Produces: two accessible toggle buttons named `展开当前模型`/`收起当前模型` and `展开可变草稿`/`收起可变草稿`, each with `aria-expanded`; no public TypeScript interface changes.

- [ ] **Step 1: Write failing component tests**

  In `WorkspacePanel.test.tsx`, add `collapses active and draft by default with independent summaries` and `preserves expansion during refresh but resets it after a session switch`. Assert:

  - initial buttons have names `展开当前模型` and `展开可变草稿` with `aria-expanded="false"`;
  - summaries show `版本 v1 · 2 张表 · 3,002 行` and `修订 3 · 1 张表 · 4 行`;
  - active/draft table view buttons and `生成版本` are absent while collapsed;
  - expanding active does not expand draft, and expanding draft restores its table controls and `生成版本`;
  - rerendering updated workspace data with `sessionId="s"` preserves expansion;
  - rerendering with `sessionId="next"` resets both sections to collapsed;
  - a zero-table active or draft summary renders `0 张表 · 0 行` without error.

  Update `opens every active and draft table through named buttons` to expand both sections before clicking the existing table buttons.

- [ ] **Step 2: Run the focused test and verify red**

  Run: `cd frontend && PATH="$HOME/.nvm/versions/node/v22.23.4/bin:$PATH" npm test -- --run src/components/workspace/WorkspacePanel.test.tsx`

  Expected: FAIL because the active/draft toggle buttons do not exist and table controls render immediately.

- [ ] **Step 3: Implement the controlled collapse behavior**

  In `WorkspacePanel.tsx`, add local booleans `activeExpanded` and `draftExpanded`, both initialized to `false`. Reset both in the existing `sessionId` effect without resetting them for workspace refreshes. Add a small local summary calculation using `tables.reduce((total, table) => total + table.rowCount, 0)` and `toLocaleString()`.

  When active/draft exists, render each section heading as a full-width button with its exact accessible name and `aria-expanded`. Keep the summary visible next to or directly below the heading. Render existing table cards and the draft publish button only while that section is expanded. Keep current empty-state rendering when the corresponding resource is absent.

- [ ] **Step 4: Run the focused test and verify green**

  Run: `cd frontend && PATH="$HOME/.nvm/versions/node/v22.23.4/bin:$PATH" npm test -- --run src/components/workspace/WorkspacePanel.test.tsx`

  Expected: PASS with all `WorkspacePanel` tests green.

- [ ] **Step 5: Synchronize current documentation and functional coverage**

  Update `docs/frontend-design.md` to state that active and draft default to independently collapsed summaries and reset on session changes. Add both new Vitest test names under `MODEL-DATAFRAME-READ-001` in `integration-tests/catalog/features.yaml`; no new route, message type, tool, upload format, or error code is introduced.

- [ ] **Step 6: Run the full frontend and coverage verification**

  Run:

  ```bash
  cd frontend
  PATH="$HOME/.nvm/versions/node/v22.23.4/bin:$PATH" npm test -- --run
  PATH="$HOME/.nvm/versions/node/v22.23.4/bin:$PATH" npm run build
  cd ..
  ./scripts/check-functional-coverage --require-pr
  git diff --check
  ```

  Expected: all Vitest tests pass, the production build succeeds, functional coverage exits 0, and `git diff --check` emits no output.

- [ ] **Step 7: Commit the feature**

  ```bash
  git add frontend/src/components/workspace/WorkspacePanel.tsx frontend/src/components/workspace/WorkspacePanel.test.tsx docs/frontend-design.md integration-tests/catalog/features.yaml docs/superpowers/specs/2026-10-25-workspace-model-section-collapse-design.md docs/superpowers/plans/2026-10-25-workspace-model-section-collapse.md
  git commit -m "feat: collapse workspace model sections"
  ```

