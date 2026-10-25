# 运行记录详情 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把只显示内部 ID 的运行列表升级为可展开的执行摘要，失败可读、成功可选择全部结果、运行中可见阶段和耗时。

**Architecture:** 后端在现有运行 JSON 上增加用户作用域解析的不可变脚本名称，不新增路由或数据库字段；前端用纯展示函数解释冻结的 study-plan 审计、状态、阶段和时间戳，并用可展开卡片按需调用现有 run datasets API。运行列表与单运行结果分别维护有界请求状态，数据集删除用 revision 失效已展开详情。

**Tech Stack:** Rust 2024、Axum、Diesel/SQLite、React 19、TypeScript、Tailwind、Vitest、Testing Library、Playwright

**Spec:** `docs/superpowers/specs/2026-10-25-run-history-details-design.md`

## Global Constraints

- 不新增路由、数据库迁移、运行删除、自动重跑或固定领域 metrics。
- 结果正文继续只从 Snappy Parquet 数据集读取；运行卡只展示元数据并用稳定 dataset ID 打开工作台。
- 运行列表最多 50 条；成功记录展开后必须列出该运行所有未删除数据集，不固定打开第一项。
- 失败界面只渲染公共 error code/message、阶段和退出码，不渲染未知 JSON、HTML、原始路径或命令。
- 脚本名称必须在服务端按同一用户、同一会话解析；跨用户与不存在资源继续统一 404。
- 更新公共 DTO、当前设计文档、行为测试和 `integration-tests/catalog/features.yaml`；注册枚举不替代行为覆盖。
- 直接在 `master` 修改；提交时保留并排除用户已有的旧潮流设计/计划未跟踪文件。

## Review Focus

- 一个成功运行含多个结果时，展开卡片列出每个结果且点击第二个结果打开正确 dataset，而不是仍打开第一个（Task 3 unit + Playwright）。
- 结果加载请求晚于会话切换、卡片收起或新 revision 返回时，不得写回陈旧详情（Task 2 hook tests）。
- 数据集删除后仍展开的运行卡必须刷新，不能保留指向已删除 dataset 的按钮（Task 3 component test）。
- 失败记录的 `error` 为 null、非对象或超长未知 JSON 时只能显示安全 fallback，不得崩溃或渲染对象（Task 2 presentation tests）。
- 时间戳缺失、无效或受时钟偏差影响时耗时显示 `—`，不能出现 `NaN` 或负时长（Task 2 presentation tests）。

---

### Task 1: Add script names to the frozen run API summary

**Files:**
- Modify: `backend/src/application/runs.rs`
- Modify: `backend/src/api/runs.rs`
- Modify: `backend/tests/simulation_runs.rs`

**Interfaces:**
- Produces: `RunSummary { run: SimulationRun, script_name: String }` in the application layer.
- Produces: `RunService::find_summary(&CurrentUser, i32) -> Result<RunSummary, RunError>` and `list_summaries(&CurrentUser, i32, i64) -> Result<Vec<RunSummary>, RunError>`; list resolves all names with one session-scoped script list, not one query per run.
- Extends: `RunSubmission` with `script_name: String`, captured from the immutable script already loaded during create.
- Extends: create/get/list run JSON with required camelCase `scriptName`; existing fields and status codes stay unchanged.
- Preserves: existing `RunService::find/list` for internal callers and tests that need the database model.

- [ ] **Step 1: Write failing API behavior assertions**

  Extend `only_explicit_click_route_creates_idempotent_frozen_runs_and_results` to assert create and replay return `scriptName == "study.rss"`, then request list and detail and assert the same value. Extend the scope test to retain uniform 404 behavior.

- [ ] **Step 2: Run the focused test and verify RED**

  Run: `cd backend && cargo test --test simulation_runs only_explicit_click --features e2e-test -- --nocapture`

  Expected: FAIL because run JSON has no `scriptName`.

- [ ] **Step 3: Implement application summaries and API serialization**

  Add the interfaces above. `list_summaries` builds a `script_resource_id -> name` map from one owned session script query and returns `RunError::NotFound` if a frozen run cannot resolve its immutable script. Change `run_json` to accept `RunSummary`; create uses `RunSubmission.script_name`, while get/list use summary methods.

- [ ] **Step 4: Run backend run tests and verify GREEN**

  Run: `cd backend && cargo test --test simulation_runs --features e2e-test`

  Expected: all run lifecycle, idempotency, result and isolation tests pass.

- [ ] **Step 5: Commit Task 1**

  ```bash
  git add backend/src/application/runs.rs backend/src/api/runs.rs backend/tests/simulation_runs.rs
  git commit -m "feat: enrich run API summaries"
  ```

### Task 2: Define safe run presentation and request states

**Files:**
- Create: `frontend/src/components/runs/runPresentation.ts`
- Create: `frontend/src/components/runs/runPresentation.test.ts`
- Modify: `frontend/src/types/run.ts`
- Modify: `frontend/src/hooks/useRunHistory.ts`
- Create: `frontend/src/hooks/useRunHistory.test.ts`

**Interfaces:**
- Extends: `SimulationRun` with required `scriptName: string`.
- Produces: `runTitle(run)`, `runStatusLabel(status)`, `runStageLabel(stage)`, `runDurationMs(run, nowMs)`, `formatDuration(ms)`, `formatBytes(bytes)` and `runFailure(run)`; the last returns `{code?: string; message: string}` using only bounded public strings.
- Produces: `RunHistoryState { runs: SimulationRun[]; status: 'idle'|'loading'|'ready'|'error'; error: string|null; reload(): void }` from `useRunHistory(sessionId, revision)`.
- Preserves: request generation protection across session/revision changes; a failed list request is `error`, never an empty ready list.

- [ ] **Step 1: Write failing pure presentation tests**

  Assert all seven governed study functions map to five Chinese study labels, manual runs fall back to `scriptName`, status/stage labels are localized, completed/running/queued duration math is exact, invalid/negative dates return null and format as `—`, bytes format deterministically, and null/non-object/oversized errors return a bounded safe fallback.

- [ ] **Step 2: Run presentation tests and verify RED**

  Run: `cd frontend && npm test -- --run src/components/runs/runPresentation.test.ts`

  Expected: FAIL because the module and `scriptName` DTO field do not exist.

- [ ] **Step 3: Implement the pure presentation boundary**

  Implement the declared functions without HTML interpretation. Recognize only `agentSkillRefs` arrays containing an object with `kind === 'study-plan'` and one of the seven exact function names. Clamp displayed error strings to 500 Unicode scalar values with an ellipsis.

- [ ] **Step 4: Write failing run history hook tests**

  With mocked `listRuns`, assert loading→ready, rejection→error with retained empty data, retry→ready, and a late response for session A cannot replace session B.

- [ ] **Step 5: Implement and verify the stateful hook**

  Replace the bare array hook with `RunHistoryState`, keep the generation guard, and expose a monotonic reload token internal to the hook.

  Run: `cd frontend && npm test -- --run src/components/runs/runPresentation.test.ts src/hooks/useRunHistory.test.ts`

  Expected: both test files pass.

- [ ] **Step 6: Commit Task 2**

  ```bash
  git add frontend/src/types/run.ts frontend/src/components/runs/runPresentation.ts frontend/src/components/runs/runPresentation.test.ts frontend/src/hooks/useRunHistory.ts frontend/src/hooks/useRunHistory.test.ts
  git commit -m "feat: model readable run history states"
  ```

### Task 3: Build expandable run cards with all result datasets

**Files:**
- Modify: `frontend/src/components/runs/RunHistory.tsx`
- Modify: `frontend/src/components/runs/RunHistory.test.tsx`
- Modify: `frontend/src/components/ResultPanel.tsx`
- Modify: `frontend/src/App.tsx`
- Test: `integration-tests/e2e/v3-workflows.spec.ts`

**Interfaces:**
- Consumes: Task 2 `RunHistoryState` and presentation helpers.
- Changes: `ResultPanel` accepts `runHistory: RunHistoryState`, `datasetRevision: number`, and `onOpenDataset(datasetId: string)` instead of a run-ID opener.
- Produces: default-collapsed `RunHistory` cards with per-run expand state and per-run `{status,data,error,generation}` dataset detail state; `ResultPanel` keys the history by `sessionId` so session changes clear expansion/cache.
- Produces: one shared one-second clock only while at least one queued/running record is rendered.
- Changes: App increments `datasetRevision` after successful DatasetWorkbench deletion and directly opens the dataset selected in run history.

- [ ] **Step 1: Replace the shallow component test with failing behavior tests**

  Mock `listRunDatasets` and assert: collapsed headers show title/status/script/model/duration without delete; expanding a succeeded run lists every dataset with name/rows/size and clicking the second calls its ID; failed/running runs do not request datasets and show their respective detail; list and dataset errors have retry buttons; stale dataset responses do not overwrite a newer `datasetRevision`; the expanded card refreshes after revision change.

- [ ] **Step 2: Run component tests and verify RED**

  Run: `cd frontend && npm test -- --run src/components/runs/RunHistory.test.tsx`

  Expected: FAIL because current cards have no expansion, result list, localized state or retry behavior.

- [ ] **Step 3: Implement card, panel and App integration**

  Use native buttons with `aria-expanded`/`aria-controls`, `role=alert` for failures and `aria-live=polite` for loading. Fetch datasets only for expanded succeeded runs; accept only the latest generation for that run and revision. Preserve expanded IDs across run list refreshes, render `RunHistory key={sessionId}` in `ResultPanel`, and invalidate the request generation when a card is collapsed.

- [ ] **Step 4: Extend the existing multi-result browser journey**

  In `model import stays a draft until publish and a script runs only after its explicit button`, switch to “运行记录”, expand the succeeded `long-script.rss` record, assert both `voltage` and `loss` appear, click `查看 loss`, and assert the opened DatasetWorkbench is for `loss`. Keep the explicit-run and centered-dialog assertions.

- [ ] **Step 5: Run Task 3 tests and verify GREEN**

  Run:

  ```bash
  cd frontend
  npm test -- --run src/components/runs/RunHistory.test.tsx
  npm run build
  npm run test:e2e -- ../integration-tests/e2e/v3-workflows.spec.ts
  ```

  Expected: unit tests, TypeScript build and the full workflow file pass; no run delete or automatic run appears.

- [ ] **Step 6: Commit Task 3**

  ```bash
  git add frontend/src/components/runs/RunHistory.tsx frontend/src/components/runs/RunHistory.test.tsx frontend/src/components/ResultPanel.tsx frontend/src/App.tsx integration-tests/e2e/v3-workflows.spec.ts
  git commit -m "feat: show expandable run results"
  ```

### Task 4: Publish documentation and functional traceability

**Files:**
- Modify: `docs/api-design.md`
- Modify: `docs/frontend-design.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/development-status.md`
- Modify: `integration-tests/catalog/features.yaml`
- Add: `docs/superpowers/specs/2026-10-25-run-history-details-design.md`
- Add: `docs/superpowers/plans/2026-10-25-run-history-details.md`

**Interfaces:**
- Documents: required `scriptName`, lazy all-dataset expansion, failure reason, live/terminal duration, deletion invalidation and unchanged no-delete/no-auto-run boundaries.
- Extends: existing `RUN-001` behavior evidence and assertions; no duplicate route/view contracts.

- [ ] **Step 1: Update current design and coverage sources**

  Add the exact DTO/interaction behavior to the four current docs. Add the new RunHistory unit symbols and updated Playwright journey to `RUN-001`; retain the four existing run route contracts exactly once.

- [ ] **Step 2: Run full deterministic verification**

  Run:

  ```bash
  cd backend
  cargo fmt --all -- --check
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --features e2e-test
  cd ../frontend
  npm test -- --run
  npm run build
  npm run test:e2e
  cd ..
  ./scripts/check-functional-coverage --require-pr
  git diff --check
  ```

  Expected: zero Rust failures/warnings, all Vitest and Playwright tests pass, build succeeds, functional coverage is 100%, and diff check is silent.

- [ ] **Step 3: Commit Task 4**

  ```bash
  git add docs/api-design.md docs/frontend-design.md docs/integration-testing-design.md docs/development-status.md integration-tests/catalog/features.yaml docs/superpowers/specs/2026-10-25-run-history-details-design.md docs/superpowers/plans/2026-10-25-run-history-details.md
  git commit -m "docs: publish detailed run history contract"
  ```
