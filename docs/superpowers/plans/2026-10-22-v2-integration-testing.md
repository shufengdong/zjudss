# v2 Full-Function Integration Testing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立可重复的浏览器到后端全链路测试、真实 DeepSeek/tensoreval 发布验收和机器可计算的 100% 功能覆盖门禁。

**Architecture:** PR 测试由 Playwright 驱动真实 React/Vite 和 Axum/SQLite/CAS，Rig 使用可编排 completion model，仿真边界使用实现 `SimEngine` 的 ScenarioEngine。候选发布使用同一组语义断言连接真实 DeepSeek 和当前合同的 tensoreval；功能目录把需求、测试文件、测试名称、断言和执行层级连接起来。

**Tech Stack:** Rust 2024、Axum、Rig 0.45 test-utils、SQLite、Arrow/Parquet、React 19、TypeScript、Vite、Vitest、Playwright、ECharts、YAML。

**Spec:** `docs/integration-testing-design.md`

## Global Constraints

- “100%”指适用功能目录项全部有自动化业务断言并在指定门禁通过，不指 100% 源码行覆盖或全部数据组合。
- 所有仿真内核调用继续只经过 `backend/src/adapters/`；ScenarioEngine 也实现该层的 `SimEngine`。
- L1/L2 使用生产路由、service、repository、工具、SQLite、CAS 和 SSE，只替换 LLM completion model 与 engine adapter 实现。
- e2e 控制接口只在 Cargo feature `e2e-test` 下编译，只监听 loopback，并校验启动时注入的 token。
- 生产 binary 不得在 DeepSeek、tensoreval 或 engine snapshot 缺失时回退到测试实现。
- PR 门禁禁止网络依赖；L3 缺少 `DEEPSEEK_API_KEY`、`TENSOREVAL_BIN` 或 `TENSOREVAL_CWD` 时明确失败。
- 异步测试等待 DOM、HTTP 状态或 SSE 事件；禁止用固定 sleep 判定成功。
- 测试 fixture 不得包含生产用户数据；上传和结果文件必须记录 schema、行数、单位和 SHA-256。
- 浏览器测试优先使用 role、label 和可见文本；仅对无稳定语义节点使用 `data-testid`。
- 每项任务完成后更新 `integration-tests/catalog/features.yaml` 中已落地的功能 ID，最终任务启用全量双向校验。

## Review Focus

- Excel 中存在空 sheet、重名列和混合 dtype 时，摄取应进入可解释的 `collecting`，页面不崩溃；Task 9 的 `dataframe-workspace.spec.ts` 固化该行为。
- SSE 在数据库提交后、浏览器收帧前断开时，重连应补发或要求 resync，不能产生重复消息；Task 8 的 `session-recovery.spec.ts` 固化该行为。
- 同一 SHA-256 在接近配额时被并发上传，artifact 计数与逻辑字节必须原子且不重复；Task 11 的 `quota-concurrency.rs` 固化该行为。
- ZIP 含路径穿越、绝对路径、重复路径、符号链接或高压缩比条目时必须整批拒绝且不留下 draft；Task 9 的 `malicious-bundle.spec.ts` 固化该行为。
- LLM 产生未知工具、畸形参数或未加载 skill 的工具调用时，run 必须收敛为可读错误并释放 permit；Task 5 的 `scripted_agent.rs` 固化该行为。

---

### Task 1: 功能目录格式和校验器骨架

**Files:**
- Create: `integration-tests/catalog/features.yaml`
- Create: `integration-tests/catalog/schema.json`
- Create: `integration-tests/catalog/fixtures/{valid,duplicate-id,missing-test}.yaml`
- Create: `frontend/scripts/check-functional-coverage.mjs`
- Create: `frontend/scripts/check-functional-coverage.test.ts`
- Modify: `frontend/package.json`
- Modify: `frontend/package-lock.json`
- Create: `scripts/check-functional-coverage`

**Interfaces:**
- Produces: `loadCatalog(path) -> FeatureEntry[]`、`validateCatalog(entries, repoRoot, manifest?) -> CoverageReport`。
- Produces: `CoverageReport { catalogEntries, validEntries, missing, duplicateIds, contractTotal, contractCovered, coveragePercent }`；合同 manifest 尚未接入时后三项为 `null`，不得显示 100%。

- [ ] **Step 1: 写校验器失败测试**

在 `check-functional-coverage.test.ts` 添加 `accepts_valid_catalog`、`rejects_duplicate_ids`、
`rejects_missing_test_symbol`、`requires_assertions_and_lanes`。断言有效 fixture 无错误且覆盖率为 null，
另外两个 fixture 分别报告重复 ID 和不存在的 `file::testName`；此时不计算合同覆盖率。

- [ ] **Step 2: 运行测试确认失败**

Run: `cd frontend && npm test -- --run scripts/check-functional-coverage.test.ts`

Expected: FAIL，模块或 fixture 尚不存在。

- [ ] **Step 3: 实现目录 schema 和校验器**

`FeatureEntry` 固定字段为 `id/source/title/layers/lanes/tests/assertions/fixtures`；`tests` 项固定为
`{ runner, file, name }`。加入 dev dependency `yaml`，校验来源文档/anchor、测试文件和测试名称实际存在，并让根脚本转发
`node frontend/scripts/check-functional-coverage.mjs`。

- [ ] **Step 4: 建立仅引用现有测试的初始目录**

先录入 BOOT、AUTH、INPUT、WORKSPACE、TASK、DATASET、VIEW、COMPARE、HISTORY 的现有 Rust/Vitest
测试条目。此阶段不启用枚举完整性，目录中的每项必须真实可定位。

- [ ] **Step 5: 运行测试和目录校验**

Run: `cd frontend && npm test -- --run scripts/check-functional-coverage.test.ts && cd .. && ./scripts/check-functional-coverage`

Expected: PASS，没有缺失文件/测试名称，`coveragePercent: null`；Task 14 启用完整合同后才允许报告 100%。

- [ ] **Step 6: 提交**

```bash
git add integration-tests/catalog frontend/package.json frontend/package-lock.json frontend/scripts scripts/check-functional-coverage
git commit -m "测试：建立功能覆盖目录和校验器"
```

### Task 2: 修复并锁定 Agent 工具合同

**Files:**
- Create: `backend/src/tools/get_storage_usage.rs`
- Modify: `backend/src/tools/mod.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/application/chat.rs`
- Modify: `backend/src/application/artifacts.rs`
- Modify: `backend/src/api/storage.rs`
- Modify: `backend/resources/agent/v2/manifest.json`
- Modify: `backend/resources/agent/v2/system.md`
- Modify: `backend/resources/agent/v2/skill-index.json` only if regenerated hash changes are required
- Create: `backend/tests/tool_manifest.rs`
- Modify: `backend/tests/common/mod.rs`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Produces: `pub fn registered_tool_names() -> &'static [&'static str]` generated from the same macro entries as `build_tools`.
- Produces: `ArtifactService::usage_for(&StorageScopeRef) -> Result<StorageUsageSummary, ArtifactError>`.
- Produces: Agent tools `list_result_datasets` and `get_storage_usage` in both manifest and runtime registry.

- [ ] **Step 1: 写失败的双向合同测试**

添加 `manifest_allowed_tools_equal_runtime_registry`，对 manifest `allowedTools` 与
`registered_tool_names()` 排序后逐项相等；另加 `get_storage_usage_returns_current_scope_only`，以两个 owner
创建 artifact，断言工具只返回当前 storage scope 的计数、字节、限制和告警。

- [ ] **Step 2: 运行测试确认当前差异**

Run: `cd backend && cargo test --test tool_manifest`

Expected: FAIL，并同时指出 manifest 缺 `list_result_datasets`、运行时缺 `get_storage_usage`。

- [ ] **Step 3: 用单一宏清单生成名称和注册代码**

在 `tools/mod.rs` 定义私有 `for_each_agent_tool!` 清单，由它生成 `registered_tool_names()` 和
`build_tools(Arc<ToolServices>)`；清单包含设计中的 22 个工具。给 `ToolServices` 和 `ChatService` 增加
`artifact_service: ArtifactService`。

- [ ] **Step 4: 实现存储使用工具并复用 API DTO 语义**

新增 `StorageUsageSummary` 和 `ArtifactService::usage_for`，让 HTTP handler 与工具共同序列化 camelCase
字段 `scope/artifactLimit/byteLimit/warningRatio/artifactCount/logicalBytes/isWarning/isExceeded`。

- [ ] **Step 5: 更新 agent bundle 并重算 hash**

把两个工具都写入 allowedTools；若 system/skill-index 内容修改，使用现有 hash 生成方式更新
`systemSha256`、`skillIndexSha256` 和对应 skill hash，不手工填占位值。

- [ ] **Step 6: 验证**

Run: `cd backend && cargo test --test tool_manifest --test agent_knowledge && cargo test`

Expected: PASS，双向清单完全相等，共 22 项。

- [ ] **Step 7: 提交**

```bash
git add backend/src backend/resources/agent/v2 backend/tests integration-tests/catalog/features.yaml
git commit -m "修复：统一 Agent 工具注册与 manifest"
```

### Task 3: 建立可枚举的生产合同 manifest

**Files:**
- Create: `backend/src/contract_manifest.rs`
- Create: `backend/src/bin/contract-manifest.rs`
- Modify: `backend/src/lib.rs`
- Modify: `backend/src/api/mod.rs`
- Modify: `backend/src/application/events.rs`
- Modify: `backend/src/application/visualization.rs`
- Modify: `backend/src/ingestion/mod.rs`
- Modify: `frontend/src/types/message.ts`
- Modify: `frontend/src/types/dataset.ts`
- Create: `backend/tests/contract_manifest.rs`
- Modify: `frontend/scripts/check-functional-coverage.mjs`
- Modify: `frontend/scripts/check-functional-coverage.test.ts`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Produces: `pub struct RouteContract { pub method: &'static str, pub path: &'static str }` and `api::route_manifest()`.
- Produces: `contract_manifest() -> ContractManifest` with routes, message types, tools, skills, input formats, view kinds and stable error codes.
- Produces: `cargo run --quiet --bin contract-manifest` JSON for the Node checker.

- [ ] **Step 1: 写失败测试**

`production_router_and_manifest_share_one_declaration` 断言 27 个 method/path 组合无重复；
`contract_manifest_contains_all_public_enums` 断言八类消息、22 个工具、六个 skill、六种上传来源和六类视图。
前端测试断言消息/视图 `const` 数组生成现有 union 类型。

- [ ] **Step 2: 运行测试确认失败**

Run: `cd backend && cargo test --test contract_manifest`

Expected: FAIL，manifest API 尚不存在。

- [ ] **Step 3: 让路由注册和 route manifest 共用声明**

在 `api/mod.rs` 用局部 `add_route!` 宏同时扩展 Router 并收集 `RouteContract`；组合路由按 method 分成独立
manifest 项，上传路由保留 `DefaultBodyLimit::disable()`。`router()` 和 `route_manifest()` 都从同一构建
函数取得结果。

- [ ] **Step 4: 汇总其他生产枚举**

从事件、工具、skill index、摄取和 visualization 的单一常量构造 `ContractManifest`。前端把
`MESSAGE_TYPES`、`VISUALIZATION_KINDS` 声明为 `as const`，相应类型从数组元素推导。

- [ ] **Step 5: 扩展覆盖校验器**

校验器执行 `cargo run --quiet --manifest-path backend/Cargo.toml --bin contract-manifest`，要求每个枚举值在
功能目录中出现且声明所需 layer；此任务先提供 `--check-manifest` 的测试 fixture，最终强制开关在 Task 14
启用。

- [ ] **Step 6: 验证并提交**

Run: `cd backend && cargo test --test contract_manifest && cargo test && cd ../frontend && npm test -- --run && npm run build`

Expected: PASS。

```bash
git add backend frontend/src frontend/scripts integration-tests/catalog/features.yaml
git commit -m "测试：发布可枚举的前后端功能合同"
```

### Task 4: 抽象 AgentFactory 并保持生产行为不变

**Files:**
- Create: `backend/src/agent/factory.rs`
- Modify: `backend/src/agent/mod.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/application/chat.rs`
- Modify: `backend/src/main.rs`
- Modify: `backend/tests/common/mod.rs`
- Create: `backend/tests/agent_factory.rs`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Produces: `AgentBuildRequest { tools: Vec<RegisteredTool>, system_prompt: String }`.
- Produces: `pub trait AgentFactory: Send + Sync { fn build(&self, request: AgentBuildRequest) -> Result<rig::Agent, LlmError>; }`.
- Produces: `DeepSeekAgentFactory::new(LlmConfig) -> Self`.
- Changes: `AppState.agent_factory: Arc<dyn AgentFactory>`; convenience constructors still accept `LlmConfig` and wrap it for existing tests.

- [ ] **Step 1: 写失败测试**

添加 `chat_builds_one_agent_per_user_message`，注入 recording factory，POST 两条消息后断言 build 两次、每次
收到 22 个工具和包含 engine snapshot/skill index 的 prompt；`deepseek_factory_rejects_unknown_provider`
保持现有稳定错误。

- [ ] **Step 2: 运行测试确认失败**

Run: `cd backend && cargo test --test agent_factory`

Expected: FAIL，AppState 不能注入 factory。

- [ ] **Step 3: 提取 factory 并改造 ChatService**

把 provider 初始化和 AgentBuilder 公共配置移到 `factory.rs`。`ChatService::run` 构造
`AgentBuildRequest` 后调用 factory；事件消费、历史加载、MAX_TURNS 和工具服务保持原路径。

- [ ] **Step 4: 保持生产和测试构造器兼容**

`main.rs` 显式注入 `Arc::new(DeepSeekAgentFactory::new(llm))`；已有 `AppState::new*` 继续接受 LlmConfig，
新增 `new_with_agent_factory(...)` 供 e2e 使用。

- [ ] **Step 5: 验证并提交**

Run: `cd backend && cargo test --test agent_factory --test e2e_v2 && cargo test`

Expected: PASS，现有 chat 行为无变化。

```bash
git add backend/src backend/tests integration-tests/catalog/features.yaml
git commit -m "重构：支持注入 AgentFactory"
```

### Task 5: 实现 Scripted LLM 和 Agent 故障场景

**Files:**
- Modify: `backend/Cargo.toml`
- Create: `backend/src/testing/mod.rs`
- Create: `backend/src/testing/scripted_agent.rs`
- Modify: `backend/src/lib.rs`
- Create: `backend/tests/scripted_agent.rs`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Produces: Cargo feature `e2e-test`。
- Produces: `ScriptScenario { id, turns: Vec<ScriptTurn>, expected_prompt: Option<String> }`.
- Produces: `ScriptTurn::{Text, ToolCall { call_id, name, args }, Error, DelayUntilReleased}`.
- Produces: `ScriptedAgentFactory::enqueue(scenario)`、`release(barrier_id)`、`trace() -> Vec<AgentTrace>`。

- [ ] **Step 1: 写 Rig 全路径失败测试**

分别添加 `scripted_tool_call_runs_registered_tool_and_persists_events`、
`unknown_tool_and_malformed_args_end_with_readable_error`、`missing_skill_is_a_failed_tool_result`、
`provider_error_releases_scheduler_permit`。断言 tool-call running/success/failed 事件和最终 assistant 消息落库。

- [ ] **Step 2: 运行确认失败**

Run: `cd backend && cargo test --features e2e-test --test scripted_agent`

Expected: FAIL，testing 模块尚不存在。

- [ ] **Step 3: 用 Rig completion model 实现脚本化 factory**

实现 Rig `CompletionModel`/`StreamingCompletionModel`，从线程安全 FIFO 取 `ScriptTurn`；用
`AgentBuilder` 和生产 `RegisteredTool` 构建真正的 Rig Agent。trace 记录 system prompt、用户/历史消息、
工具 schema 和每轮结果，但序列化时过滤上传正文和 secret header。

- [ ] **Step 4: 实现错误和 barrier**

Error 产生 provider 错误；DelayUntilReleased 由 Notify/barrier 驱动，以观察状态释放，测试不得 sleep。

- [ ] **Step 5: 验证并提交**

Run: `cd backend && cargo test --features e2e-test --test scripted_agent && cargo test`

Expected: PASS，非 feature 构建不包含 `backend::testing` 公共入口。

```bash
git add backend/Cargo.toml backend/src backend/tests integration-tests/catalog/features.yaml
git commit -m "测试：加入可编排 Rig Agent"
```

### Task 6: 实现 ScenarioEngine 和真实 Parquet 故障注入

**Files:**
- Create: `backend/src/testing/scenario_engine.rs`
- Modify: `backend/src/testing/mod.rs`
- Create: `backend/tests/scenario_engine.rs`
- Create: `integration-tests/fixtures/results/{single,multi}/manifest.json`
- Create: `integration-tests/fixtures/results/README.md`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Produces: `EngineScenario::{Success(Vec<DatasetFixture>), InspectFailure, RunFailure, Timeout, Empty, CorruptParquet, Oversize, EscapingPath, Symlink}`.
- Produces: `ScenarioEngine::enqueue(EngineScenario)`、`release(barrier_id)`、`calls() -> Vec<RecordedEngineCall>`.
- Implements: production `SimEngine::inspect` and `SimEngine::run` only through adapter boundary.

- [ ] **Step 1: 写失败测试**

覆盖合法 Snappy Parquet/多数据集，以及非零失败、超时、空目录、坏 Parquet、超限、逃逸路径、symlink。
断言成功 metadata 的 schema/行数/codec，失败时 task/analysis 为 failed、无 dataset/artifact 半发布且 staging
被清理。

- [ ] **Step 2: 运行确认失败**

Run: `cd backend && cargo test --features e2e-test --test scenario_engine`

Expected: FAIL，ScenarioEngine 尚不存在。

- [ ] **Step 3: 实现 scenario queue 和 fixture writer**

使用 Arrow/Parquet 60 写 Snappy 文件；所有输出先写到 `RunSpec.result_dir`。恶意场景返回对应路径或创建
symlink，让生产 `validate_outputs` 和发布事务处理，测试实现不得自行伪造应用层错误。

- [ ] **Step 4: 记录 fixture 元数据并校验 SHA-256**

README/manifest 记录生成器版本、列、dtype、单位、行数和 hash；测试重新计算并逐项相等。

- [ ] **Step 5: 验证并提交**

Run: `cd backend && cargo test --features e2e-test --test scenario_engine --test process_engine_v2`

Expected: PASS。

```bash
git add backend/src/testing backend/tests/scenario_engine.rs integration-tests/fixtures integration-tests/catalog/features.yaml
git commit -m "测试：加入仿真场景引擎和 Parquet 故障注入"
```

### Task 7: 启动隔离的 e2e server 并提供安全控制面

**Files:**
- Modify: `backend/Cargo.toml`
- Create: `backend/src/bin/e2e-server.rs`
- Create: `backend/src/testing/control.rs`
- Modify: `backend/src/testing/mod.rs`
- Modify: `backend/src/application/mod.rs`
- Create: `backend/tests/e2e_server.rs`
- Modify: `frontend/vite.config.ts`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Produces: feature-gated binary `cargo run --features e2e-test --bin e2e-server`.
- Produces: control endpoints `POST /__e2e/llm-scenarios`、`POST /__e2e/engine-scenarios`、
  `POST /__e2e/barriers/{id}/release`、`GET /__e2e/trace`、`GET /__e2e/ready`.
- Produces: `VITE_API_TARGET` override for Vite proxy; default remains `http://127.0.0.1:8080`.

- [ ] **Step 1: 写失败测试**

`e2e_server_uses_temp_db_and_data_dir`、`control_routes_require_token`、
`control_routes_are_absent_from_production_router`、`server_rejects_non_loopback_bind`。使用随机端口，断言退出后
临时根目录可删除。

- [ ] **Step 2: 运行确认失败**

Run: `cd backend && cargo test --features e2e-test --test e2e_server`

Expected: FAIL，binary/control router 尚不存在。

- [ ] **Step 3: 实现 e2e bootstrap**

从 `E2E_ROOT` 创建 SQLite/CAS，注入 ScriptedAgentFactory、ScenarioEngine、trusted-proxy auth 和小配额；
bind 仅接受 `127.0.0.1`/`::1`，启动后输出单行 `E2E_READY` JSON。token 必须来自非空
`E2E_CONTROL_TOKEN`。

- [ ] **Step 4: 实现控制路由并编译隔离**

控制 handler 只操作 scenario queue/barrier/trace，不直接写领域数据库；生产 `build_router` 不 merge 它。

- [ ] **Step 5: 验证并提交**

Run: `cd backend && cargo test --features e2e-test --test e2e_server && cargo build && cargo build --features e2e-test --bin e2e-server`

Expected: PASS。

```bash
git add backend frontend/vite.config.ts integration-tests/catalog/features.yaml
git commit -m "测试：增加隔离的端到端服务入口"
```

### Task 8: 建立 Playwright harness 和会话恢复旅程

**Files:**
- Modify: `frontend/package.json`
- Modify: `frontend/package-lock.json`
- Create: `frontend/playwright.config.ts`
- Create: `integration-tests/harness/serve.mjs`
- Create: `integration-tests/e2e/fixtures.ts`
- Create: `integration-tests/e2e/helpers/{control,events,server,uploads}.ts`
- Create: `integration-tests/e2e/session-recovery.spec.ts`
- Create: `integration-tests/e2e/auth-isolation.spec.ts`
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/components/history/HistorySidebar.tsx`
- Modify: `frontend/src/components/Composer.tsx`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Produces: `npm run test:e2e`，由 loopback harness 启动/停止 e2e-server 和 Vite
  （127.0.0.1:18080/15173），并可用同一 E2E_ROOT 重启后端。
- Produces: Playwright fixture `appPage({ subject, tenant })` and `control.enqueueLlm/engine/release/trace`.
- Produces: helpers that wait on API/SSE/DOM state without fixed delays.

- [ ] **Step 1: 写失败的浏览器 smoke 和恢复测试**

覆盖新建/切换/重命名/删除、乐观 user 消息替换、user/assistant/tool-call 消息渲染、超过 50 条的 older、
历史搜索、刷新恢复、Last-Event-ID 断线补发、旧游标 resync、失效 localStorage 回退。断线测试在事务提交
后用 barrier 关闭页面连接，再重连并断言同一 message ID 只出现一次；其余五类消息由 Task 9/10 的真实
业务旅程产生，Task 14 对八类消息做完整性检查。

- [ ] **Step 2: 写失败的两 owner 隔离测试**

两个 browser context 带不同 subject/tenant；验证 session、message、SSE、task、dataset/download、history
搜索的猜 ID 都返回不可区分的 404。另验证缺失/错误 proxy token 为 401。

- [ ] **Step 3: 运行确认失败**

Run: `cd frontend && npm run test:e2e -- --project=chromium session-recovery.spec.ts auth-isolation.spec.ts`

Expected: FAIL，Playwright/harness 或稳定 accessible name 尚不存在。

- [ ] **Step 4: 配置 harness 并补齐语义选择器**

加入 `@playwright/test`；Playwright webServer 启动 `serve.mjs`，由它管理 Cargo e2e binary 与 Vite，解析
ready 状态并向测试 helper 提供带 token 的重启入口。给无文本图标按钮增加 `aria-label`，不改变自然语言
主交互。

- [ ] **Step 5: 验证并提交**

Run: `cd frontend && npm run test:e2e -- --project=chromium session-recovery.spec.ts auth-isolation.spec.ts`

Expected: PASS，trace 仅在失败时保留。

```bash
git add frontend integration-tests/e2e integration-tests/catalog/features.yaml
git commit -m "测试：覆盖浏览器会话恢复和身份隔离"
```

### Task 9: 覆盖上传、推断、重传、版本和案例

**Files:**
- Create: `integration-tests/e2e/dataframe-workspace.spec.ts`
- Create: `integration-tests/e2e/rss-replacement.spec.ts`
- Create: `integration-tests/e2e/bundle-examples.spec.ts`
- Create: `integration-tests/e2e/malicious-bundle.spec.ts`
- Create: `integration-tests/fixtures/uploads/**`
- Modify: `frontend/src/components/uploads/UploadQueue.tsx`
- Modify: `frontend/src/components/workspace/WorkspacePanel.tsx`
- Modify: `frontend/src/components/examples/{ExampleLibrary,ExampleDetail}.tsx`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Consumes: Task 8 `appPage`、upload helper 和 Scripted LLM control。
- Produces: 可复用 fixtures：单 RSS、目录/ZIP bundle、CSV、Parquet、Excel 多 sheet 及五种恶意 ZIP。

- [ ] **Step 1: 写 DataFrame 旅程失败测试**

上传 CSV/Excel/Parquet 并分批补传；断言按表头而非文件名推断，高置信/未知/歧义/缺列/dtype 冲突/同类
重复均在工作区展示。空 sheet、重名列和混合 dtype 必须 collecting；通过自然语言
`confirm_dataframe_mapping` 后生成 ready draft 并激活。

- [ ] **Step 2: 写 RSS 重传和不可变版本失败测试**

错误 RSS → 新 batch/file → 对话调用 replace → parent/child 可追溯；remove、modify、activate、rollback
都生成或切换不可变版本，旧任务继续引用旧版本。

- [ ] **Step 3: 写 bundle、案例和恶意归档失败测试**

目录与 ZIP 得到等价 manifest；搜索/详情/导入代表案例后修改副本，共享 snapshot hash 不变。恶意 ZIP
五类整批失败且无 draft/artifact 泄漏；当前 engine snapshot 的全部 73 个案例由 Task 12 的真实依赖门禁
逐一发现和导入。

- [ ] **Step 4: 运行确认失败并补齐必要 UI 状态**

Run: `cd frontend && npm run test:e2e -- --project=chromium dataframe-workspace.spec.ts rss-replacement.spec.ts bundle-examples.spec.ts malicious-bundle.spec.ts`

Expected before implementation: FAIL；补齐 accessible status/diagnostic 文本后 PASS。

- [ ] **Step 5: 回归后端摄取测试并提交**

Run: `cd backend && cargo test --test dataframe_ingestion --test bundle_ingestion --test example_import --test workspace_versions && cd ../frontend && npm test -- --run && npm run build`

Expected: PASS。

```bash
git add frontend/src integration-tests/e2e integration-tests/fixtures integration-tests/catalog/features.yaml
git commit -m "测试：覆盖全部模型摄取和版本旅程"
```

### Task 10: 覆盖仿真、分析、表格、六类图表和比较

**Files:**
- Create: `integration-tests/e2e/simulation-results.spec.ts`
- Create: `integration-tests/e2e/analysis-views.spec.ts`
- Create: `integration-tests/e2e/comparison.spec.ts`
- Modify: `frontend/src/hooks/useDataset.ts`
- Modify: `frontend/src/hooks/useDatasetView.ts`
- Modify: `frontend/src/components/datasets/{DatasetWorkbench,DatasetTable,ChartRenderer}.tsx`
- Modify: `frontend/src/components/tasks/{TaskHistory,ComparisonView}.tsx`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Consumes: Task 6 ScenarioEngine success/failure datasets and Task 8 browser fixtures.
- Produces: browser assertions for all study types and visualization kinds, including reload recovery.

- [ ] **Step 1: 写 study/task/result 失败测试**

通过自然语言分别触发 power-flow、fault、QSTS、harmonic、open study；断言 queued/running/succeeded/failed、
绑定 version/RSS/skills/engine contract、多结果发现、schema/单位/行数、Parquet 下载。

- [ ] **Step 2: 写 DataFrame 页和 hook 竞态测试**

快速从 dataset A 切到 B 时，A 的迟到响应不得覆盖 B；`loadMore` 失败后保留已有页并允许重试。浏览器验证
cursor 分页、列序、null“—”、合法重复行和列选择。

- [ ] **Step 3: 写分析和六类视图失败测试**

Agent 先 list schema、加载 result-analysis skill、运行受控 RSS，产生有血缘的派生 Parquet；line/bar/
scatter/heatmap/boxplot/histogram 均渲染，单位和 histogram 边界正确。非法字段与规模 422 只影响结果层，
刷新后视图恢复。

- [ ] **Step 4: 写比较失败测试**

覆盖结果名、稳定键、dtype、单位、QSTS 时间键兼容和各类不可比原因；可比任务生成差值 DataFrame，刷新
后通过 comparison ID 恢复。

- [ ] **Step 5: 修复发现的 hook/UI 竞态并验证**

Run: `cd frontend && npm test -- --run && npm run test:e2e -- --project=chromium simulation-results.spec.ts analysis-views.spec.ts comparison.spec.ts`

Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add frontend/src integration-tests/e2e integration-tests/catalog/features.yaml
git commit -m "测试：覆盖仿真结果分析可视化和比较"
```

### Task 11: 覆盖调度、配额、故障原子性和服务重启

**Files:**
- Create: `backend/tests/quota_concurrency.rs`
- Create: `backend/tests/failure_atomicity.rs`
- Create: `backend/src/testing/faults.rs`
- Modify: `backend/src/testing/control.rs`
- Modify: `backend/src/testing/mod.rs`
- Modify: `backend/src/application/events.rs`
- Create: `integration-tests/e2e/resilience.spec.ts`
- Create: `integration-tests/e2e/storage-quota.spec.ts`
- Create: `integration-tests/e2e/responsive-accessibility.spec.ts`
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/components/ResultPanel.tsx`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Consumes: ScriptedAgent/ScenarioEngine barriers and e2e server lifecycle helpers.
- Produces: `FaultInjector::fail_next(FaultPoint::EventCommit)`，仅在 `e2e-test` feature 注入下一次 SQLite
  event transaction failure。
- Produces: deterministic tests for limits 8/2/1/32/4、20,000/20 GiB、80% warning and restart recovery.

- [ ] **Step 1: 写并发与配额失败测试**

barrier 同时占满 session/owner/instance/engine permits，L0 断言生产默认值 8/2/1/32/4；L2 用降低后的同构
边界快速验证 queue-full 行为，包含“下一个等待项 429 且不写 user message”。两个并发上传使用同一
SHA-256 并逼近降低后的测试配额，断言唯一 artifact 计数、逻辑字节和引用原子一致；生产默认
20,000/20 GiB 和 80% 继续由配置合同测试锁定。

- [ ] **Step 2: 写失败原子性测试**

依次注入 LLM error/timeout、中断、引擎失败/timeout、空/坏/超限 Parquet、逃逸/symlink 和 SQLite 写失败；
断言 permit 释放、错误脱敏、无半发布 dataset/artifact、已发布历史仍可读。

- [ ] **Step 3: 写重启和 FTS 降级测试**

通过 Task 8 harness 停止并以同一 E2E_ROOT 重启 server；已提交 session/message/version/task/view 恢复，
运行中 task 按合同收敛。
损坏 FTS 后搜索返回 `history_search_unavailable`，普通消息页仍成功。

- [ ] **Step 4: 写桌面/窄屏和基础无障碍测试**

桌面三栏与窄屏抽屉都完成上传、对话、工作区、任务和结果查看；用 `@axe-core/playwright` 检查严重违规，
并验证焦点顺序和所有图标按钮名称。

- [ ] **Step 5: 验证并提交**

Run: `cd backend && cargo test --features e2e-test --test quota_concurrency --test failure_atomicity --test agent_scheduler && cd ../frontend && npm run test:e2e -- --project=chromium resilience.spec.ts storage-quota.spec.ts responsive-accessibility.spec.ts`

Expected: PASS，无固定 sleep。

```bash
git add backend/tests frontend integration-tests/e2e integration-tests/catalog/features.yaml
git commit -m "测试：覆盖并发配额恢复和安全故障"
```

### Task 12: 建立真实 tensoreval 和全部案例发布验收

**Files:**
- Modify: `backend/tests/process_engine_v2.rs`
- Create: `backend/tests/release_engine_cases.rs`
- Create: `scripts/test-release-engine`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Produces: non-ignored release command requiring `TENSOREVAL_BIN` and `TENSOREVAL_CWD`.
- Produces: machine-readable `target/release-integration/engine-report.json`.

- [ ] **Step 1: 写环境缺失失败测试**

脚本在任一必需变量缺失、binary capability 不符、snapshot hash 不符时非零退出并输出稳定原因；不得把测试
标记 ignored 后返回成功。

- [ ] **Step 2: 扩展真实引擎合同**

测试命名 Parquet 输入、`--result-dir`、`--output-format parquet`、expr_df、Snappy、多结果和失败诊断；
逐一验证当前 snapshot 的 73 个案例可发现、依赖 hash 正确且可导入临时 scope。

- [ ] **Step 3: 增加能力代表运行**

选择 catalog 声明支持的最小案例分别执行 DataFrame 潮流、RSS 短路、QSTS、谐波和开放 study。报告记录
example ID、study type、binary hash、engine contract、输出 schema/行数/codec，不记录绝对用户路径。

- [ ] **Step 4: 在真实环境运行**

Run: `TENSOREVAL_BIN=/absolute/.../tensoreval TENSOREVAL_CWD=/absolute/.../eig-rs ./scripts/test-release-engine`

Expected: PASS，报告 73/73 可导入且所有能力代表运行成功。

- [ ] **Step 5: 提交**

```bash
git add backend/tests scripts/test-release-engine integration-tests/catalog/features.yaml
git commit -m "测试：自动验收真实 tensoreval 和案例合同"
```

### Task 13: 建立真实 DeepSeek + tensoreval 候选发布旅程

**Files:**
- Create: `integration-tests/release/real-dependencies.spec.ts`
- Create: `integration-tests/release/semantic-assertions.ts`
- Create: `frontend/playwright.release.config.ts`
- Create: `scripts/test-release-integration`
- Modify: `integration-tests/catalog/features.yaml`

**Interfaces:**
- Produces: semantic assertions `expectToolTrace(trace, expectedDependencies)` and `expectDomainState(api, expected)`.
- Produces: release script requiring DeepSeek secret and engine variables, writing `target/release-integration/llm-report.json`.

- [ ] **Step 1: 写发布环境 preflight**

缺 `DEEPSEEK_API_KEY`、`TENSOREVAL_BIN`、`TENSOREVAL_CWD` 或合同 hash 不匹配时脚本失败；日志对 key、用户
上传正文和绝对路径去敏。

- [ ] **Step 2: 写真实依赖用户旅程**

浏览器执行 DataFrame 潮流、RSS 短路、结果提问产生派生 DataFrame、修改参数重跑、比较两任务。断言
允许工具集合、skill 先于 gated 工具、稳定 ID 参数和最终 version/task/dataset/comparison 状态，不匹配
回答逐字文本。

- [ ] **Step 3: 实现受控重试**

仅对明确的 provider 暂时性错误最多重试 2 次；业务断言失败、非法工具、内核合同失败不重试。重试耗尽
保持发布失败并归档 trace。

- [ ] **Step 4: 在候选发布环境运行**

Run: `DEEPSEEK_API_KEY=... TENSOREVAL_BIN=... TENSOREVAL_CWD=... ./scripts/test-release-integration`

Expected: PASS，engine 和 LLM 两份报告完整，无 skip。

- [ ] **Step 5: 提交**

```bash
git add integration-tests/release frontend/playwright.release.config.ts scripts/test-release-integration integration-tests/catalog/features.yaml
git commit -m "测试：加入真实智能体候选发布验收"
```

### Task 14: 强制 100% 功能门禁并接入 CI

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `frontend/scripts/check-functional-coverage.mjs`
- Create: `.github/workflows/ci.yml`
- Create: `.github/workflows/release-integration.yml`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/development-status.md`
- Modify: `docs/v2-deployment.md`

**Interfaces:**
- Consumes: Task 3 contract manifest and Tasks 4–13 test IDs.
- Produces: strict `./scripts/check-functional-coverage --require-pr` and `--require-release` gates.

- [ ] **Step 1: 完成功能目录**

逐项覆盖设计 §5 的 BOOT/AUTH/SESSION/MESSAGE/SSE/SCHED/UPLOAD/INPUT/INFERENCE/WORKSPACE/
EXAMPLE/AGENT/STUDY/TASK/DATASET/ANALYSIS/VIEW/COMPARE/HISTORY/STORAGE/UI/LEGACY；每个 route method、
消息、22 个工具、六个 skill、六种上传来源、视图和稳定错误码都有独立 ID、断言和真实测试符号。

- [ ] **Step 2: 写严格模式失败测试**

从 fixture manifest 各删除一个 route/tool/message/view/format/error code，断言 checker 报出精确缺口；添加
无理由 skip、`manual-only`、不存在的测试名称和 release 项只在 PR lane 的 fixture，均必须失败。

- [ ] **Step 3: 启用 PR CI**

`ci.yml` 顺序执行 Rust fmt/test、Vitest/build、Playwright L1/L2、严格 PR 功能校验；缓存 Cargo/npm，上传
JUnit、coverage JSON/HTML、Playwright trace 和去敏日志。任何测试 skip 或覆盖率低于 100% 失败。

- [ ] **Step 4: 启用候选发布 workflow**

受保护环境提供 secret 和 engine artifact；先执行 PR 门禁，再执行 Task 12/13 脚本和
`--require-release`。缺依赖、73 个案例未全部验证或真实旅程失败均阻止发布。

- [ ] **Step 5: 全量验证**

Run: `cd backend && cargo fmt --check && cargo test --features e2e-test && cd ../frontend && npm test -- --run && npm run build && npm run test:e2e && cd .. && ./scripts/check-functional-coverage --require-pr && git diff --check`

Expected: 全部 PASS，PR 功能报告为 `covered == total` 和 `coveragePercent == 100`，无非预期 skip。

- [ ] **Step 6: 更新状态文档**

只有 Step 5 通过后，才把 Playwright/L1/L2 标记为已实现。只有 release workflow 在目标环境通过后，才在
发布记录中标记 L3 已验收；保留执行日期、engine contract 和报告 artifact ID。

- [ ] **Step 7: 提交**

```bash
git add integration-tests frontend/scripts .github docs
git commit -m "测试：启用百分百功能覆盖发布门禁"
```

### Task 15: 最终审查与可复现性验证

**Files:**
- Modify only if verification finds a defect; do not add scope during this task.

**Interfaces:**
- Consumes: all commands and reports from Tasks 1–14.
- Produces: clean branch whose documented commands reproduce PR acceptance; release evidence remains environment-specific.

- [ ] **Step 1: 从干净 worktree 重装并运行 PR 门禁**

使用 `superpowers:using-git-worktrees` 创建临时干净 worktree，并在其中运行：

Run: `npm ci --prefix frontend && cd backend && cargo fmt --check && cargo test --features e2e-test && cd ../frontend && npm test -- --run && npm run build && npm run test:e2e && cd .. && ./scripts/check-functional-coverage --require-pr`

Expected: PASS，不依赖开发机残留数据库、DATA_DIR、node_modules 外文件或真实网络。

- [ ] **Step 2: 审查测试隔离和敏感数据**

搜索生产 router 的 `__e2e`、测试回退、固定 sleep、绝对 fixture 路径、key/header/prompt 原文日志；任何命中
必须解释为测试 feature 内合法使用或修复。

- [ ] **Step 3: 运行发布门禁并保存证据**

Run: `./scripts/test-release-integration`

Expected: 在配置完整的候选发布环境 PASS；报告含 engine/agent bundle hash、测试 ID 和语义断言结果。

- [ ] **Step 4: 请求代码审查并修复阻断项**

审查重点为生产/测试隔离、adapter 硬边界、测试是否真正经过浏览器、功能目录是否存在虚假覆盖、真实 LLM
断言是否允许合理措辞变化。修复后只重跑受影响测试及完整 PR 门禁各一次。

- [ ] **Step 5: 最终提交（仅有审查修复时）**

```bash
git add <review-fix-files>
git commit -m "测试：修正全链路验收审查问题"
```
