# API、前端状态与界面整合实施计划

> **For Codex:** 使用 `superpowers:executing-plans` 逐项实施；行为变化先写失败测试并观察 RED。

**Goal:** 用最小共享原语统一下载响应、请求代次与 impact confirmation，同时保留领域状态所有权和所有公共合同。

**Architecture:** 后端 API helper 只映射 attachment response；前端 request gate 只管理取消/代次；共享 dialog 只管理可访问交互。领域 hooks/components 继续拥有 DTO、分页、轮询、错误文案与 mutation。

**Tech Stack:** Rust/Axum、React 19、TypeScript、Vitest、Testing Library、Playwright。

## Global Constraints

- 直接在 `master` 修改，不创建 worktree。
- 不改变 HTTP/SSE、DTO、错误码、数据库、Agent、engine 或图表合同。
- 所有编辑使用 `apply_patch`；按 TDD 观察 RED→GREEN。
- 保持 390px 无横向溢出、键盘可达和跨用户 404。
- 阶段末运行后端聚焦测试、全量 Vitest、生产构建、Playwright 与功能覆盖。

## Review Focus

- 共享层是否只承载完全相同的语义，没有吞掉领域 stale/retry/分页规则。
- 新请求是否使所有旧 response、finally 和 mutation side effect 失效。
- dialog 是否有可靠 focus、Escape、backdrop、busy 防重入和 error/retry。
- impact token 过期/变化后是否重新预览而不是重放旧 token。
- 删除死 hook/别名后是否无当前消费者或公共导出。

### Task 1: Characterize request races and impact interaction

**Files:**
- Add: `frontend/src/hooks/useComparison.test.ts`
- Add: `frontend/src/components/common/ImpactConfirmationDialog.test.tsx`
- Modify: `frontend/src/components/workspace/WorkspacePanel.test.tsx`
- Modify: `frontend/src/components/resources/DataResourcePanel.test.tsx`
- Modify: `frontend/src/components/runs/RunHistory.test.tsx`

1. 写 comparison A 迟于 B 返回时不得覆盖 B、失败可重试的测试。
2. 写 dialog 初始 focus、Tab trap、Escape/backdrop、busy 禁止关闭的测试。
3. 写版本删除先显示结构化影响、只携 preview token 删除且失败刷新 preview 的测试。
4. 写数据资源/运行删除失败后保留 dialog 并刷新 token 的测试。
5. 运行这些测试并确认因当前行为缺失而 RED。

### Task 2: Consolidate backend attachment responses

**Files:**
- Add: `backend/src/api/download.rs`
- Modify: `backend/src/api/mod.rs`
- Modify: `backend/src/api/datasets.rs`
- Modify: `backend/src/api/data_resources.rs`
- Modify: `backend/src/api/model_dataframes.rs`
- Modify: relevant backend HTTP tests only if coverage is missing

1. 提取内部 attachment response helper。
2. 三个 handler 保留 service 查询，只委托最终 response 映射。
3. 运行 dataset、data resource、model DataFrame HTTP/下载聚焦测试。

### Task 3: Unify request generations and remove pass-through hooks

**Files:**
- Add: `frontend/src/hooks/useRequestGate.ts`
- Add: `frontend/src/state/mergeByStableId.ts`
- Modify: `frontend/src/hooks/useComparison.ts`
- Modify: `frontend/src/hooks/useChartView.ts`
- Modify: `frontend/src/hooks/useDataResources.ts`
- Modify: `frontend/src/hooks/useSavedCharts.ts`
- Modify: `frontend/src/components/resources/DataResourceWorkbench.tsx`
- Modify: `frontend/src/components/comparisons/ComparisonView.tsx`
- Modify: `frontend/src/components/datasets/DatasetWorkbench.tsx`
- Delete: `frontend/src/hooks/useAnalysisRun.ts`
- Delete: `frontend/src/hooks/useDatasetView.ts`

1. 实现 request gate 与纯 stable-ID merge。
2. 修复 comparison 迟到覆盖并增加 loading/error/retry。
3. 让 chart/data-resource 读取在切换和卸载时统一取消；让分页合并使用同一纯函数。
4. 删除无消费者/纯转发 hook，更新直接 imports。
5. 运行 hooks 与 workbench 聚焦测试确认 GREEN。

### Task 4: Share accessible impact dialogs

**Files:**
- Add: `frontend/src/components/common/ImpactConfirmationDialog.tsx`
- Modify: `frontend/src/components/Sidebar.tsx`
- Modify: `frontend/src/components/runs/RunHistory.tsx`
- Modify: `frontend/src/components/resources/DataResourceDeleteDialog.tsx`
- Modify: `frontend/src/components/resources/DataResourcePanel.tsx`
- Modify: `frontend/src/components/workspace/WorkspacePanel.tsx`

1. 实现只负责 interaction shell 的 dialog。
2. 迁移会话、运行和数据资源删除，保留各自影响正文与成功 side effect。
3. 为版本删除建立 preview/loading/error/confirm 状态，移除 `window.confirm` 删除路径。
4. mutation 失败后刷新 impact preview；旧请求不能改写新目标。
5. 运行 Task 1 与相关组件全测试确认 GREEN。

### Task 5: Documentation and verification

**Files:**
- Modify: `docs/frontend-design.md`
- Modify: `docs/development-status.md`

1. 记录 request generation、分页合并与 impact dialog 所有权。
2. 运行 Rust fmt/聚焦测试、全量 Vitest、生产 build、31 条 Playwright 和功能覆盖。
3. 机械审计公共合同和 dead imports；确认无需功能目录迁移。
4. 提交实现与文档，完成自我代码审查后自动进入 8.9。

