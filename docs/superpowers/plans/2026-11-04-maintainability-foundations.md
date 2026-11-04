# 维护性基础层实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立全仓库维护性重构的第一阶段基线，并消除应用装配、公共错误响应、HTTP 下载和测试夹具中已经证实的重复代码。

**Architecture:** 使用具名依赖与配置对象替代 `AppState` 的长位置参数构造链；把 E2E/test app 组装限制在测试模块并用 builder 表达差异；API 错误仍按领域显式映射，但共享统一响应构造；前端 REST 与文件下载共用同一受保护请求边界；测试数据 builders 只覆盖至少两个测试文件共享的稳定 DTO。领域事务、repository SQL、图表算法和业务状态机不在本阶段抽象。

**Tech Stack:** Rust 2024、Axum、Diesel/SQLite、React 19、TypeScript、Vitest、Playwright、Cargo tests。

**Spec:** `docs/superpowers/specs/2026-11-04-repository-maintainability-refactor-design.md`

## Global Constraints

- 直接在 `master` 工作，不创建 feature worktree；每个任务只提交自己拥有的文件。
- 所有 tensoreval 调用继续只经过 `backend/src/adapters/`；本计划不修改引擎命令或结果合同。
- 所有 repository 查询继续显式接收 `UserId`；跨用户和不存在资源统一 404。
- 浏览器和 LLM 不得接收服务器路径、命令、SQL、artifact 内部 ID 或任意 ECharts option。
- 发布资源继续不可变，仿真和派生结果继续以 Snappy Parquet 为事实来源。
- 不新增旧 API、旧数据库或旧测试构造器兼容层；仓库内消费者在所属任务中一次迁移完成。
- 不建立全局领域错误枚举；领域错误仍在各自边界定义，HTTP 层只负责显式映射。
- 新共享抽象必须至少有两个同语义消费者；相似但失败语义不同的代码保留独立。
- 公共合同若发生变化，必须同步功能目录、行为测试和当前设计文档。
- 当前相邻 eig-rs snapshot 的 7 个 `case_projection_invalid` 是已知外部阻塞；不得放宽校验或把候选发布写成全绿。

## Review Focus

- `AppState` 的生产配置、测试 override 和默认限制不能在具名装配迁移时互换；Task 1 的装配合同测试必须固定关键字段和 override 身份。
- 最小/full E2E 资源目录、自动启动开关和自定义 engine/agent/analyzer 可以任意组合；Task 2 的 builder 测试必须覆盖组合而非只测默认值。
- HTTP 错误的 status、稳定 code、message 和 details 不得因去重改变；Task 3 必须表驱动覆盖代表性的 400/401/404/409/413/422/429/500/503。
- REST JSON 与三个文件下载入口必须共享 401 通知、结构化错误解析、AbortSignal 和 filename fallback；Task 4 必须覆盖成功、401、非 JSON 错误和缺失 disposition。
- 测试 fixture builder 不得在测试间共享可变数组/对象；Task 5 必须证明每次调用生成独立嵌套值，并让现有测试继续通过。

---

### Task 1: 用具名依赖装配 `AppState`

**Files:**
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/main.rs`
- Modify: `backend/tests/common/mod.rs`
- Modify: `backend/tests/app_bootstrap.rs`

**Interfaces:**
- Produces: `AppStateDependencies { pool, engine, agent_factory, auth, scheduler }`。
- Produces: `AppStateOptions { data_dir, storage_limits, engine_snapshot, upload_single_file_limit_bytes, upload_batch_limit_bytes, upload_staging_ttl_hours, script_repair_max_attempts, agent_model_retry_max, managed_engine_timeout, managed_result_max_file_bytes, script_author_override, failure_analyzer_override, managed_workflow_auto_start, archive_limits }`。
- Produces: `AppState::assemble(dependencies: AppStateDependencies, options: AppStateOptions) -> AppState`。
- Removes: 无仓库内消费者的 `new`、`new_with_agent_factory`、`new_with_data_dir*`、`new_with_storage_limits`、`new_with_resources` 和 `new_with_resources_and_agent_factory` 构造链；Task 2 暂时消费的 E2E wrapper 可保留到 Task 2 再移除。
- Preserves: `AppState` 字段、服务依赖关系、生产默认值、测试限制和运行行为。

- [ ] **Step 1: 写出失败的具名装配合同测试**

  在 `backend/tests/app_bootstrap.rs` 增加编译时接口断言和行为断言：

  ```rust
  fn named_assembly_contract(
      assemble: fn(AppStateDependencies, AppStateOptions) -> AppState,
  ) {
      let _ = assemble;
  }

  #[tokio::test]
  async fn app_state_assembly_uses_named_dependencies_and_options() {
      named_assembly_contract(AppState::assemble);
      let app = TestApp::spawn().await;
      assert_eq!(app.state.upload_single_file_limit_bytes, 2 * 1024 * 1024);
      assert_eq!(app.state.upload_batch_limit_bytes, 8 * 1024 * 1024);
      assert_eq!(app.state.engine_snapshot.contract_version, "tensoreval-parquet-v3");
  }
  ```

- [ ] **Step 2: 运行测试并确认 RED**

  Run: `cd backend && cargo test --features e2e-test --test app_bootstrap app_state_assembly_uses_named_dependencies_and_options`

  Expected: FAIL to compile because `AppStateDependencies`、`AppStateOptions` and `AppState::assemble` do not exist.

- [ ] **Step 3: 定义具名装配接口并迁移生产组合根**

  在 `backend/src/application/mod.rs` 定义上述两个结构和 `assemble`。把现有 `new_with_resources_and_agent_factory` 的服务装配主体移入 `assemble`，删除未使用的 `_catalog` 参数和无调用构造链。`backend/src/main.rs` 仍执行 `SchemaCatalog::from_snapshot(...)` 启动校验，但不把已验证 catalog 作为未使用参数传入状态。

- [ ] **Step 4: 迁移测试组合根并删除位置参数调用**

  把 `backend/tests/common/mod.rs` 的完整测试启动路径改为构造 `AppStateDependencies` 与 `AppStateOptions`。保留测试当前的 engine snapshot、限制、override 和 `managed_workflow_auto_start=false` 语义。

- [ ] **Step 5: 运行聚焦验证**

  Run: `cd backend && cargo test --features e2e-test --test app_bootstrap`

  Expected: PASS；生产和测试装配都只调用 `AppState::assemble`。

- [ ] **Step 6: 运行静态门禁**

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`

  Expected: PASS with no `too_many_arguments` allowance on `AppState` assembly and no unused catalog value.

- [ ] **Step 7: 提交**

  ```bash
  git add backend/src/application/mod.rs backend/src/main.rs backend/tests/common/mod.rs backend/tests/app_bootstrap.rs
  git commit -m "refactor: assemble application state from named dependencies"
  ```

### Task 2: 收敛 E2E 状态和 `TestApp` 测试夹具

**Files:**
- Create: `backend/src/testing/app.rs`
- Create: `backend/src/testing/engine_fixture.rs`
- Modify: `backend/src/testing/mod.rs`
- Modify: `backend/src/testing/control.rs`
- Modify: `backend/src/application/mod.rs`
- Create: `backend/tests/common/app.rs`
- Create: `backend/tests/common/fixtures.rs`
- Create: `backend/tests/common/http.rs`
- Modify: `backend/tests/common/mod.rs`
- Modify: `backend/tests/app_bootstrap.rs`
- Modify: `backend/tests/agent_factory.rs`
- Modify: `backend/tests/agent_scheduler.rs`
- Modify: `backend/tests/failure_atomicity.rs`
- Modify: `backend/tests/scripted_agent.rs`
- Modify: `backend/tests/script_repair.rs`
- Modify: `backend/tests/simulation_runs.rs`
- Modify: `backend/tests/workflow_retries.rs`

**Interfaces:**
- Produces: `testing::E2eCatalogMode::{Minimal, Full}`、`testing::E2eAppOptions` and `testing::build_e2e_app(pool, data_dir, agent_factory, engine, auth, options) -> AppState`；E2E 资源 fixture 只由 `testing::engine_fixture` 创建。
- Produces: `common::TestApp::builder() -> TestAppBuilder`，with methods `auth_settings(ttl, secure)`、`scheduler(...)`、`engine(...)`、`agent_factory(...)`、`failure_analyzer(...)`、`script_author(...)`、`catalog_mode(E2eCatalogMode)` and `spawn().await`。
- Preserves convenience: `TestApp::spawn()` may remain as the single default shorthand delegating to the builder。
- Removes: `AppState::new_for_e2e`、`application::E2eAppOptions` and the six `TestApp::spawn_with_*`/`spawn_minimal_with_*` variants。
- Splits: `common/mod.rs` becomes re-exports; app lifecycle lives in `app.rs`, domain fixture helpers in `fixtures.rs`, HTTP helpers in `http.rs`。

- [ ] **Step 1: 写出失败的组合 builder 测试**

  在 `backend/tests/app_bootstrap.rs` 增加：

  ```rust
  #[tokio::test]
  async fn test_app_builder_combines_overrides_without_constructor_variants() {
      let engine: Arc<dyn SimEngine> = Arc::new(ScenarioEngine::default());
      let factory = Arc::new(ScriptedAgentFactory::default());
      let analyzer = Arc::new(ScriptedAuthor::default());
      let app = TestApp::builder()
          .engine(engine.clone())
          .agent_factory(factory.clone())
          .failure_analyzer(analyzer.clone())
          .catalog_mode(E2eCatalogMode::Minimal)
          .spawn()
          .await;
      assert!(Arc::ptr_eq(&app.state.engine, &engine));
      assert_eq!(app.state.engine_snapshot.contract_version, "e2e-contract");
  }
  ```

  测试实现时若 trait-object `Arc::ptr_eq` 需要先显式强转，保留身份断言，不退化为只检查类型。

- [ ] **Step 2: 运行测试并确认 RED**

  Run: `cd backend && cargo test --features e2e-test --test app_bootstrap test_app_builder_combines_overrides_without_constructor_variants`

  Expected: FAIL because `TestApp::builder` does not exist.

- [ ] **Step 3: 把 E2E-only app 与资源 fixture 移出 application**

  将 `AppState::new_for_e2e` 的资源树创建、snapshot 选择和测试限制迁入 `backend/src/testing/engine_fixture.rs` 与 `backend/src/testing/app.rs`。`control.rs` 改用 `build_e2e_app`。生产 `application` 不再包含 E2E fixture 文件写入或 `E2eAppOptions`。

- [ ] **Step 4: 实现 `TestAppBuilder` 并拆分 common 模块**

  builder 默认值必须与当前 `TestApp::spawn()` 一致；每个 setter 只覆盖自己的字段。完整资源 fixture 与 minimal catalog 选择通过同一测试模块实现，不在 `backend/tests/common` 再复制 manifest/catalog 写入算法。把 `seed_test_users`、`test_user`、`publish_model_bytes` 移入 `fixtures.rs`，把 `session_deletion_token` 移入 `http.rs`。

- [ ] **Step 5: 迁移全部测试调用方并删除旧构造器**

  使用 builder 表达每个现有调用的差异。迁移完成后运行：

  Run: `rg -n 'new_for_e2e|spawn_with_|spawn_minimal_with_' backend/src backend/tests`

  Expected: no old E2E/AppState/TestApp constructors remain；生产必要的其他 `spawn_with_*` 名称不在此匹配范围时需逐项说明。

- [ ] **Step 6: 运行测试夹具回归**

  Run: `cd backend && cargo test --features e2e-test --test app_bootstrap --test agent_factory --test agent_scheduler --test failure_atomicity --test scripted_agent --test script_repair --test simulation_runs --test workflow_retries`

  Expected: PASS，覆盖默认、scheduler、agent、engine、failure analyzer、script author 与 minimal/full catalog 组合。

- [ ] **Step 7: 运行静态门禁并提交**

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`

  ```bash
  git add backend/src/testing backend/src/application/mod.rs backend/tests/common backend/tests/app_bootstrap.rs backend/tests/agent_factory.rs backend/tests/agent_scheduler.rs backend/tests/failure_atomicity.rs backend/tests/scripted_agent.rs backend/tests/script_repair.rs backend/tests/simulation_runs.rs backend/tests/workflow_retries.rs
  git commit -m "test: consolidate backend application fixtures"
  ```

### Task 3: 分离并简化 HTTP 错误映射

**Files:**
- Create: `backend/src/api/error.rs`
- Modify: `backend/src/api/mod.rs`
- Test: `backend/src/api/error.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Produces: crate-private `ApiError::public(status: StatusCode, code: &'static str, message: impl Into<String>) -> ApiError`。
- Produces: crate-private `ApiError::with_details(self, details: serde_json::Value) -> ApiError`。
- Preserves: `internal`、`not_found`、`authentication_required`、`invalid_credentials` and every existing `From<DomainError>` mapping。
- Moves: `ApiError`、`ApiErrorBody`、`IntoResponse` and all domain conversions from router assembly into `api/error.rs`。
- Does not produce: a shared application error trait or a global domain error enum。

- [ ] **Step 1: 添加失败的错误构造与映射测试**

  在新模块测试中表驱动断言代表错误：认证 401、not-found 404、冲突 409、容量 413、验证 422、队列 429、内部错误 500、历史搜索 503。至少单独断言 `RunError::ScriptIncompatible` 保留 `missingAliases` details，`ChartServiceError` 保留 chart details。

- [ ] **Step 2: 运行测试并确认 RED**

  Run: `cd backend && cargo test --lib api::error::tests`

  Expected: FAIL because `api::error` and shared constructors do not exist.

- [ ] **Step 3: 移动错误模块并用统一构造器替换字段拼装**

  `api/mod.rs` 只保留 module registration、router/manifest 和 health handler。每个 `From` 仍显式决定 status/code/message/details；只消除重复的 `details: None` 和响应 body 构造，不把不同领域的 code 集合合并为隐式全局表。

- [ ] **Step 4: 运行错误与 API 合同验证**

  Run: `cd backend && cargo test --lib api::error::tests && cargo test --features e2e-test --test contract_manifest --test auth_scope --test chart_views --test data_resources`

  Expected: PASS with unchanged public status/code/details.

- [ ] **Step 5: 运行静态门禁并提交**

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`

  ```bash
  git add backend/src/api/error.rs backend/src/api/mod.rs
  git commit -m "refactor: centralize HTTP error responses"
  ```

### Task 4: 统一前端受保护请求和文件下载

**Files:**
- Modify: `frontend/src/api/client.ts`
- Modify: `frontend/src/api/client.test.ts`

**Interfaces:**
- Produces: private `checkedFetch(url: string, init?: RequestInit, reportAuthentication?: boolean) -> Promise<Response>`。
- Produces: private `download(url: string, fallbackFilename: string, signal?: AbortSignal) -> Promise<{ blob: Blob; filename: string }>`。
- Preserves: public `request<T>` and all current API functions；三个下载函数新增一致的可选 `AbortSignal` 尾参数。
- Migrates: `downloadDataset`、`downloadDataResource` and `downloadModelDataFrame` to the shared download function。
- Preserves: structured `ApiError`、coalesced authentication-required event、Content-Disposition filename、fallback filename and abort signal。

- [ ] **Step 1: 增加失败的共享传输行为测试**

  在 `frontend/src/api/client.test.ts` 添加：

  - 三个下载函数都使用响应 filename，缺失 header 时分别使用正确 fallback。
  - 下载的 JSON 401 产生 `ApiError` 并触发一次 authentication-required 事件。
  - 下载的非 JSON 500 产生 `请求失败（500）`。
  - `downloadDataset`、`downloadDataResource` 与 `downloadModelDataFrame` 都把可选 `AbortSignal` 交给 fetch；测试以当前尚不存在的 dataset/resource signal 参数形成 RED。

  测试同时 spy `fetch`，断言每次操作只有一个请求，防止错误处理重试。

- [ ] **Step 2: 运行聚焦测试并确认现有重复实现至少有一项失败**

  Run: `cd frontend && npm test -- --run src/api/client.test.ts`

  Expected: FAIL to type-check because dataset/resource download functions do not yet accept `AbortSignal`.

- [ ] **Step 3: 实现共享 transport 边界并迁移 JSON/download 调用**

  `checkedFetch` 唯一负责 `apiUrl`、headers、fetch、结构化错误和认证事件；`request` 只负责成功响应解析；`download` 只负责 blob 与 filename。登录继续以 `reportAuthentication=false` 调用。

- [ ] **Step 4: 运行前端 API 与认证测试**

  Run: `cd frontend && npm test -- --run src/api/client.test.ts src/api/events.test.ts src/state/auth.test.ts`

  Expected: PASS with no duplicate manual `if(!res.ok)` branches in public download functions.

- [ ] **Step 5: 运行生产构建并提交**

  Run: `cd frontend && npm run build`

  ```bash
  git add frontend/src/api/client.ts frontend/src/api/client.test.ts
  git commit -m "refactor: share frontend response handling"
  ```

### Task 5: 建立隔离的前端测试 DTO builders

**Files:**
- Create: `frontend/src/test/fixtures.ts`
- Create: `frontend/src/test/fixtures.test.ts`
- Modify: `frontend/src/components/messages/FailureDiagnosisPanel.test.tsx`
- Modify: `frontend/src/hooks/useRunFailureDiagnosis.test.tsx`
- Modify: `frontend/src/components/runs/runPresentation.test.ts`
- Modify: `frontend/src/components/runs/RunHistory.test.tsx`
- Modify: `frontend/src/components/messages/UploadBatchCard.test.tsx`
- Modify: `frontend/src/components/uploads/UploadBatchDecisionCard.test.tsx`
- Modify: `frontend/src/hooks/useUploadBatches.test.ts`
- Modify: `frontend/src/components/datasets/ChartBuilder.test.tsx`
- Modify: `frontend/src/components/datasets/SavedChartPanel.test.tsx`
- Modify: `frontend/src/types/contracts.test.ts`

**Interfaces:**
- Produces: `simulationRun(overrides?: Partial<SimulationRun>): SimulationRun`。
- Produces: `failureDiagnosis(overrides?: Partial<FailureDiagnosis>): FailureDiagnosis`。
- Produces: `uploadBatch(overrides?: Partial<UploadBatchView>): UploadBatchView`。
- Produces: `chartCapabilities(overrides?: Partial<ChartCapabilities>): ChartCapabilities`。
- Does not produce: builders used by only one test file；study/workflow fixtures remain local until a second stable consumer exists。

- [ ] **Step 1: 写出失败的 fixture isolation 测试**

  `fixtures.test.ts` 必须断言：

  ```ts
  const first = simulationRun()
  const second = simulationRun()
  first.agentSkillRefs.push({ kind: 'study-plan', function: 'changed' })
  expect(second.agentSkillRefs).toEqual([])

  const one = uploadBatch()
  const two = uploadBatch()
  one.candidates.push({ id: 'x' } as never)
  expect(two.candidates).toEqual([])
  ```

  另断言顶层 override 只替换指定字段；需要替换 `error`、`analysis` 或 `limits` 时由调用点提供完整的对应公共 DTO，builder 不实现隐式 deep merge。

- [ ] **Step 2: 运行测试并确认 RED**

  Run: `cd frontend && npm test -- --run src/test/fixtures.test.ts`

  Expected: FAIL because shared builders do not exist.

- [ ] **Step 3: 实现 builders 并迁移重复消费者**

  每次调用都创建新的嵌套数组和对象；不要通过 JSON stringify/parse 克隆。迁移只删除与 builder 默认值相同的本地 fixture；测试关心的差异继续在调用点显式 override，避免隐藏断言前提。

- [ ] **Step 4: 验证没有保留目标重复 builders**

  Run: `rg -n 'const (run|makeRun|batch|diagnosis|capabilities)\s*=.*(SimulationRun|UploadBatchView|FailureDiagnosis|ChartCapabilities)' frontend/src --glob '*.test.ts' --glob '*.test.tsx'`

  Expected: no duplicate builder for the four migrated DTOs；仅单文件特有 helper 或明确不同语义 fixture 可保留并在任务记录中说明。

- [ ] **Step 5: 运行相关测试与生产构建**

  Run: `cd frontend && npm test -- --run src/test/fixtures.test.ts src/components/messages/FailureDiagnosisPanel.test.tsx src/hooks/useRunFailureDiagnosis.test.tsx src/components/runs/runPresentation.test.ts src/components/runs/RunHistory.test.tsx src/components/messages/UploadBatchCard.test.tsx src/components/uploads/UploadBatchDecisionCard.test.tsx src/hooks/useUploadBatches.test.ts src/components/datasets/ChartBuilder.test.tsx src/components/datasets/SavedChartPanel.test.tsx src/types/contracts.test.ts`

  Run: `cd frontend && npm run build`

  Expected: PASS；测试数量增加 fixture isolation tests，产品 bundle 不导入 `src/test/fixtures.ts`。

- [ ] **Step 6: 提交**

  ```bash
  git add frontend/src/test frontend/src/components/messages/FailureDiagnosisPanel.test.tsx frontend/src/hooks/useRunFailureDiagnosis.test.tsx frontend/src/components/runs/runPresentation.test.ts frontend/src/components/runs/RunHistory.test.tsx frontend/src/components/messages/UploadBatchCard.test.tsx frontend/src/components/uploads/UploadBatchDecisionCard.test.tsx frontend/src/hooks/useUploadBatches.test.ts frontend/src/components/datasets/ChartBuilder.test.tsx frontend/src/components/datasets/SavedChartPanel.test.tsx frontend/src/types/contracts.test.ts
  git commit -m "test: share isolated frontend DTO fixtures"
  ```

### Task 6: 固化模块所有权、基线和阶段门禁

**Files:**
- Modify: `docs/architecture.md`
- Modify: `docs/development-status.md`
- Modify: `docs/integration-testing-design.md`

**Interfaces:**
- Produces: `docs/architecture.md` 中 application composition、API error mapping、backend E2E fixtures、frontend transport、test DTO fixture 的权威所有权表。
- Produces: `docs/development-status.md` 中第一阶段完成项、删除的构造器/重复分支、实际测试计数和已知外部门禁状态。
- Preserves: API、数据库、Agent 和用户行为文档；本阶段没有公共产品合同变化时不得制造版本或路由数量变化。

- [ ] **Step 1: 更新架构与测试文档**

  记录以下边界：生产 `AppState` 只接受具名依赖/options；E2E app 与资源 fixture 只在 feature-gated testing 模块；领域错误保留独立，HTTP 统一响应构造；前端所有受保护 REST/download 共用 checked transport；共享测试 builders 只在测试模块且每次深层隔离。

- [ ] **Step 2: 运行全量前端门禁**

  Run: `cd frontend && npm test -- --run && npm run build && npm run test:e2e`

  Expected: all Vitest files, production build and 31 Playwright journeys PASS；从输出记录新的准确 Vitest 数量。

- [ ] **Step 3: 运行全量后端门禁**

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --features e2e-test`

  Expected: fmt/Clippy PASS。若完整测试仍只因 `current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count` 报告相邻 snapshot 的同一 7 个 `case_projection_invalid`，保留完整输出，再运行：

  `cargo test --features e2e-test -- --skip current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count`

  Expected: every remaining backend test PASS。任何新增失败都必须修复，不得归入外部阻塞。

- [ ] **Step 4: 运行功能覆盖和差异门禁**

  Run: `./scripts/check-functional-coverage --require-pr`

  Expected: 100% coverage with no missing/duplicate/unknown contracts.

  Run: `git diff --check`

  Expected: no whitespace errors.

- [ ] **Step 5: 请求独立代码审查**

  审查重点为：是否删除了仍有消费者的 constructor；test builder 是否改变默认语义；API status/code/details 是否漂移；前端下载是否漏掉 401/abort/filename；fixture 是否共享可变值；文档是否把外部 eig-rs 阻塞写成全绿。

- [ ] **Step 6: 提交当前文档和覆盖证据**

  ```bash
  git add docs/architecture.md docs/development-status.md docs/integration-testing-design.md
  git commit -m "docs: record maintainability foundation boundaries"
  ```

  本阶段不改变公共产品合同，因此不修改功能目录；若实施中发现必须改变公共合同，应停止本计划并先修订设计与计划，而不是临时追加清单。

## Self-Review Result

- **Spec coverage:** 本计划只实现路线图的第一个子系统，覆盖基线、应用装配、公共 HTTP 错误、测试 app、前端 transport 和共享测试 DTO；artifact/DB、engine/import、上传/资源、工作流、结果、Agent 与 UI 领域重构明确留给后续独立计划。
- **Step scan:** 每个代码任务都有新接口测试、RED、最小实现、聚焦验证、静态门禁和独立提交；文档任务只记录最终证据。
- **Type consistency:** Task 1 的 `AppStateDependencies/AppStateOptions` 被 Task 2 的 E2E builder 消费；Task 3 不把领域错误类型改成共享 trait；Task 4 的 transport 不改变公共客户端函数；Task 5 的 builders 只依赖公共 DTO。
- **Review Focus:** 五项风险分别由 Task 1、2、3、4、5 的明确测试覆盖。
- **Proportion:** 计划详细指定跨文件接口和验证命令，但不抄写装配函数、错误映射或 fixture builder 的实现体。
