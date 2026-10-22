# 配电网仿真智能体 v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 交付支持安全多用户隔离、不可变模型工作区、多文件摄取、原生 Parquet 结果、自然语言结果分析、历史检索与仿真对比的第二版系统，并以干净 v2 schema 一次性替换未投入使用的 v1 原型。

**Architecture:** 先建立身份 scope、运行调度和可恢复事件流，再按 `Artifact → Ingestion → ModelVersion → SimulationTask → ResultDataset → ResultAnalysisRun/Comparison` 的顺序实现后端。前端从单体 `App.tsx` 拆成按 session 键控的领域 store；所有领域写操作继续经自然语言和 agent 工具完成，固定接口只承担上传、历史浏览和结果读取。

**Tech Stack:** Rust 2024、Axum 0.8、Tokio、Diesel/SQLite、rig 0.45、tensoreval、Arrow/Parquet、React 19、TypeScript 5.9、Vite 8、ECharts 6、Vitest、Testing Library。

**Spec:** `docs/model-workspace-design.md`，prompt/skill 资源见 `docs/agent-prompt-skills-design.md`，配套契约见 `docs/architecture.md`、`docs/api-design.md`、`docs/db-design.md`、`docs/frontend-design.md`。

## Global Constraints

- 仿真内核和 `tensoreval` 的命令构造、执行、超时与错误翻译只能位于 `backend/src/adapters/`。
- 前端出网类型以 `frontend/src/types/index.ts` 为权威；所有 ID 出网为字符串，时间戳为 ISO 8601。
- v2 仿真和结果分析必须显式传 `--result-dir ... --output-format parquet`；删除 v1 CSV 结果读取、固定 metrics 和旧曲线/报告端点。
- v1 未投入使用且没有需要保留的生产数据；v2 允许重置本地开发数据库和 DATA_DIR，不写数据回填或双读逻辑。
- 数据库只保存结果文件的 schema、行数、血缘和相对 artifact 路径，不预设任何领域统计指标。
- 模型版本、artifact、result dataset 和分析脚本一经发布不可修改；改变输入必须产生新版本或新资源。
- LLM 和前端不得提交服务器路径、命令行、任意 SQL 或任意 ECharts option，只能提交稳定资源 ID 和受控 spec。
- 公开多人部署必须使用 `AUTH_MODE=trusted_proxy`；`AUTH_MODE=instance` 只允许可信单用户/单租户部署。
- 可信代理必须移除外部同名请求头并注入 `X-Zjudss-Subject`、可选 `X-Zjudss-Tenant` 和 `X-Zjudss-Proxy-Token`；后端以常量时间比较 `AUTH_PROXY_TOKEN`。
- 每次资源读取都先检查当前 access scope；跨 scope 的已知 ID 与不存在 ID 均返回 404。
- 默认配额为 20,000 个唯一 artifact 和 21,474,836,480 字节，80% 告警，不按时间自动删除。
- 同一 session 的 agent run 和 workspace 写操作串行；实例活动 run 默认 8、owner 活动 run 默认 2、等待队列默认 32。
- 用户上传、模型修改、运行、查询和比较必须能在刷新后从数据库与 artifact 存储恢复，不能依赖进程内事件或浏览器缓存。
- 启动时必须从 `${TENSOREVAL_CWD}/sparrowzz/dss` 建立通过 hash 校验的只读 docs/catalog/examples 快照；运行期不直接读取变化中的源文件。
- v2 非测试启动必须显式配置可读的 `TENSOREVAL_CWD`；测试只能显式注入最小 engine-resource fixture，不能静默跳过知识/案例 preflight。
- 每个阶段合入前都必须通过 `cargo test`、`npm test -- --run`、`npm run build` 和 `git diff --check`。

## Review Focus

- 两个 owner 猜测同一个 session、task、dataset 或 SSE URL 时，非归属方始终得到 404；由 Task 2、12、15 的集成测试覆盖。
- 同一批次重试相同字节必须幂等，重用 batch/file ID 却改变字节必须得到 `409 idempotency_conflict`；由 Task 8 覆盖。
- ZIP traversal、符号链接、压缩炸弹、Excel 空表和同类型重复表不得污染 active 版本；由 Task 8、9 覆盖。
- 仿真成功但 Parquet 损坏、结果过大或配额并发超限时，不得发布半成品 dataset/result 消息；由 Task 11–13 覆盖。
- 刷新、SSE 断线、历史定位和旧异步响应不得造成消息丢失、跨 session 状态串用或图表静默截断；由 Task 4、16、17 覆盖。

---

## Milestones

| 里程碑 | Tasks | 可发布结果 |
|---|---:|---|
| M0 平台基础 | 1–4 | 可测试应用、身份隔离、有限队列和无缺口 SSE |
| M1 模型工作区 | 5–10 | 引擎文档/案例快照、artifact/配额、active/draft、表头推断、上传与案例导入、版本修改 |
| M2 通用仿真结果 | 11–13 | Parquet 内核合同、v2 仿真、数据集表格/多视图、自然语言结果分析 |
| M3 历史与对比 | 14–15 | 任务比较、聊天全文检索、自然语言历史工具 |
| M4 前端工作台 | 16–17 | 多文件状态、模型/任务/结果三层工作台、六类 ECharts 图表、历史定位 |
| M5 发布 | 18 | 四条端到端路径、schema 收口、容量与安全验收 |

## Target File Map

```text
backend/src/
├── lib.rs / config.rs
├── auth/                    # Principal 提取与 instance/trusted_proxy 两种模式
├── application/
│   ├── scheduler.rs         # agent admission、全局/owner/session 并发约束
│   ├── artifacts.rs         # artifact 提交、配额与 GC 编排
│   ├── workspace.rs         # active/draft 与不可变版本事务
│   ├── ingestion.rs         # 摄取状态机与版本发布
│   ├── simulations.rs       # v2 任务编排
│   ├── datasets.rs          # 数据集、分页与视图
│   ├── analysis.rs          # 结果分析运行
│   ├── comparisons.rs       # 两任务兼容性和差值分析
│   └── history.rs           # 分页、FTS 和上下文窗口
├── adapters/
│   ├── process/             # 从现有 855 行 process.rs 拆出 command/output
│   └── contract.rs          # 启动能力探测和版本合同
├── catalog/                 # 版本化 schema catalog 与表头推断
├── engine_resources/        # TENSOREVAL_CWD 文档索引、案例目录和只读 snapshot
├── ingestion/               # CSV/Excel/Parquet reader 与安全归档处理
├── storage/                 # staging、CAS blob、Parquet 读取
├── api/                     # auth/workspaces/ingestions/datasets/history 等 handler
├── db/repositories/         # 按领域拆分 repository，所有查询显式接收 scope
└── tools/                   # 工作区、仿真、结果分析、比较、历史工具

frontend/src/
├── types/                   # session/message/workspace/task/dataset/view 分文件导出
├── api/                     # 对应 v2 REST 与可恢复 SSE
├── hooks/                   # useSessionFeed/useWorkspace/useTaskDatasets 等
├── state/                   # 纯 reducer 与归并逻辑
└── components/
    ├── workspace/ uploads/ history/
    └── datasets/            # DatasetTable、ChartRenderer 和六类 builder
```

### Task 1: 建立可集成测试的应用入口和集中配置

**Files:**
- Create: `backend/src/lib.rs`
- Create: `backend/src/config.rs`
- Create: `backend/tests/common/mod.rs`
- Create: `backend/tests/app_bootstrap.rs`
- Create: `frontend/vitest.config.ts`
- Create: `frontend/src/test/setup.ts`
- Create: `frontend/src/test/smoke.test.tsx`
- Modify: `backend/src/main.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/Cargo.toml`
- Modify: `frontend/package.json`

**Interfaces:**
- Produces: `pub fn build_router(state: AppState) -> Router`、`RuntimeConfig::from_env()`、测试用 `TestApp::spawn()`。
- Preserves: `cargo run` 仍监听 `127.0.0.1:8080`；本任务只重构装配，接口替换由后续任务完成。

- [x] **Step 1: 写失败测试**：新增 `config_defaults_are_safe`，断言默认 DB、DATA_DIR、agent 上限、上传限制和 `AUTH_MODE=instance` 的精确值；非测试模式缺少/不可读 `TENSOREVAL_CWD` 必须失败；新增 `test_app_runs_migrations_in_temp_dir`。
- [x] **Step 2: 验证失败**：分别运行 `cd backend && cargo test config_defaults_are_safe` 和 `cd backend && cargo test --test app_bootstrap`，预期因模块/接口不存在而失败。
- [x] **Step 3: 实现最小入口**：把模块导出和应用装配移入 `lib.rs`，`main.rs` 只读取配置、建 listener 并调用库入口；集中解析配置且拒绝零值/负值/不可写 DATA_DIR。
- [x] **Step 4: 建测试夹具**：测试数据库和 DATA_DIR 使用独立临时目录，`StubEngine` 注入 `AppState`，禁止测试读取仓库 `.env`。
- [x] **Step 5: 建前端测试入口**：加入 Vitest、jsdom 和 Testing Library，`smoke.test.tsx` 只验证测试环境能渲染 React；增加 `npm test` 脚本。
- [x] **Step 6: 验证**：运行 `cd backend && cargo test`、`cd frontend && npm test -- --run` 和 `npm run build`。
- [x] **Step 7: Commit**：`git commit -m "重构：建立 v2 配置与测试入口"`。

### Task 2: 身份主体、存储 scope 与全链路访问隔离

**Files:**
- Create: `backend/migrations/00000000000002_identity_scope/up.sql`
- Create: `backend/migrations/00000000000002_identity_scope/down.sql`
- Create: `backend/src/auth/mod.rs`
- Create: `backend/src/auth/trusted_proxy.rs`
- Create: `backend/src/api/auth.rs`
- Test: `backend/tests/auth_scope.rs`
- Create: `backend/src/db/repositories/{mod.rs,session.rs,message.rs,task.rs}`
- Remove: `backend/src/db/repo.rs`
- Modify: `backend/src/db/{mod.rs,schema.rs,models.rs}`
- Modify: `backend/src/api/{mod.rs,sessions.rs,events.rs,tasks.rs}`
- Modify: `backend/src/application/{mod.rs,chat.rs}`
- Modify: `docs/{architecture.md,api-design.md,db-design.md}`

**Interfaces:**
- Produces: `Principal { access: AccessScope, storage: StorageScopeRef, subject: String }`；Axum extractor `CurrentPrincipal`；`GET /api/me`。
- Repository rule: `SessionRepo::{create,list,find,rename,delete}(&AccessScope, ...)`，不保留无 scope 的公开重载。

- [x] **Step 1: 写 schema 测试**：migration 创建 storage_scopes 和带 `access_scope_kind/access_scope_key/storage_scope_id` 的 session；全新数据库不存在无 scope 的插入路径。
- [x] **Step 2: 写隔离测试**：以 alice/bob 两套可信代理头创建会话，断言列表互不可见，bob 对 alice 的消息、附件、任务和 SSE 均为 404；错误体不泄露资源是否存在。
- [x] **Step 3: 实现认证模式**：`instance` 产生固定主体；`trusted_proxy` 校验代理 token 后读取 subject/tenant，缺失或无效返回 401。增加 `PUBLIC_DEPLOYMENT=true` 时禁止以 instance 模式启动的配置测试。
- [x] **Step 4: 强制 repository scope**：会话直接过滤 access scope；子资源通过 join 到 sessions 校验，不接受客户端提供 owner/scope ID。EventBus 仍可按 session ID 分发，但订阅前必须完成归属检查。
- [x] **Step 5: 前端身份恢复契约**：`GET /api/me` 返回不可变 `scopeKey` 和展示主体；文档规定 `localStorage` 的上次 session key 必须包含 scopeKey。
- [x] **Step 6: 验证**：运行 `cd backend && cargo test --test auth_scope && cargo test`。
- [x] **Step 7: Commit**：`git commit -m "功能：增加身份 scope 与会话隔离"`。

### Task 3: Agent admission、同会话串行与稳定错误体

**Files:**
- Create: `backend/src/application/scheduler.rs`
- Create: `backend/tests/agent_scheduler.rs`
- Modify: `backend/src/api/mod.rs`
- Modify: `backend/src/api/sessions.rs`
- Modify: `backend/src/application/{mod.rs,chat.rs}`
- Modify: `backend/src/config.rs`

**Interfaces:**
- Produces: `RunScheduler::try_admit(scope: &AccessScope, session_id: i32) -> Result<RunTicket, AppError>`；`RunTicket::run(future)`。
- Error: `ApiErrorBody { error: String, code: String, details: Option<Value> }`，队列满为 `429 agent_queue_full`。

- [x] **Step 1: 写失败测试**：全局最多 8、owner 最多 2、同 session 最多 1 个 active；第 33 个等待项返回 429；拒绝时不写 user 消息。
- [x] **Step 2: 验证失败**：运行 `cd backend && cargo test --test agent_scheduler`。
- [x] **Step 3: 实现调度器**：使用公平 semaphore 控制全局/owner 活动数，session keyed mutex 保证顺序，admission counter 限制等待数；permit drop 必须释放所有计数并清理空 keyed state。
- [x] **Step 4: 接入消息端点**：先校验会话和 admission，再落 user 消息并 spawn；排队状态使用持久 progress/tool-call 表达，不能靠内存 toast。
- [x] **Step 5: 统一错误映射**：错误体固定为必需 `error/code` 和可选 `details`，任何 details 均经过 allowlist 序列化。
- [x] **Step 6: 验证**：运行 `cd backend && cargo test --test agent_scheduler && cargo test`。
- [x] **Step 7: Commit**：`git commit -m "功能：限制 agent 并发并串行化会话"`。

### Task 4: v2 消息协议与无缺口 SSE 恢复

**Files:**
- Create: `backend/tests/sse_recovery.rs`
- Create: `frontend/src/state/messageReducer.ts`
- Create: `frontend/src/state/messageReducer.test.ts`
- Modify: `backend/src/application/events.rs`
- Modify: `backend/src/api/events.rs`
- Modify: `backend/src/db/repositories/message.rs`
- Modify: `frontend/src/types/index.ts`
- Modify: `frontend/src/api/events.ts`

**Interfaces:**
- Produces: `OperationRef { kind, id }`、稳定 `callId`、`model-status/comparison`、`MessageRepo::list_after(scope, session_id, after_id)`。
- SSE: 持久消息帧必须含 `id: <messageId>`；支持 `Last-Event-ID`，无法补发时发 `resync-required`。

- [x] **Step 1: 写线格式测试**：8 类 v2 消息能序列化为前端字段；progress 按 `operation.kind + id` 合并，tool-call 按 callId 合并；旧 DTO 不再出现在公开类型中。
- [x] **Step 2: 写恢复竞态测试**：在“订阅 broadcast”和“查询补发”之间插入消息，断言客户端只收到一次且顺序按 ID；跨 session/owner 游标不可读取。
- [x] **Step 3: 实现后端恢复**：handler 先订阅并缓存实时消息，再查 DB 补发，按 ID 去重释放缓存；广播前必须先提交消息。
- [x] **Step 4: 实现前端连接状态机**：`subscribeEvents()` 暴露 `opened` Promise、message/control 回调和 close；打开后并行读快照，reducer 丢弃旧 session 的迟到事件。
- [x] **Step 5: 验证**：运行 `cd backend && cargo test --test sse_recovery` 和 `cd frontend && npm test -- --run messageReducer`。
- [x] **Step 6: Commit**：`git commit -m "功能：升级消息协议并补齐 SSE 恢复"`。

### Task 5: 内容寻址 artifact、原子配额与垃圾回收

**Files:**
- Create: `backend/migrations/00000000000003_artifacts/up.sql`
- Create: `backend/migrations/00000000000003_artifacts/down.sql`
- Create: `backend/src/storage/{mod.rs,blob.rs,staging.rs}`
- Create: `backend/src/application/artifacts.rs`
- Create: `backend/src/db/repositories/{artifact.rs,storage_scope.rs}`
- Create: `backend/src/api/storage.rs`
- Test: `backend/tests/artifact_store.rs`
- Modify: `backend/Cargo.toml`

**Interfaces:**
- Produces: `ArtifactStore::stage(reader, limits) -> StagedArtifact`、`ArtifactService::commit(scope_id, staged, metadata) -> ArtifactRef`、`get_usage(scope_id) -> StorageUsage`、`collect_unreferenced() -> GcReport`。
- Blob path: `objects/<sha256[0..2]>/<sha256>`；DB 只存相对路径；staging 必须位于 DATA_DIR 内以支持原子 rename。

- [x] **Step 1: 写失败测试**：相同 scope/内容只计一次；跨 scope 分别计费；20,000 个或 20 GiB 任一越界均回滚；两个并发提交只能有一个越过剩余额度；路径始终位于 DATA_DIR。
- [x] **Step 2: 验证失败**：运行 `cd backend && cargo test --test artifact_store`。
- [x] **Step 3: 实现存储与事务**：流式计算 SHA-256 和大小，fsync 后原子发布；事务内锁定/更新 scope 计数、插入 scope-artifact；失败清理 staging，禁止先发布 DB 后移动文件。
- [x] **Step 4: 实现只读用量 API 和 GC**：`GET /api/storage-usage` 从 principal 解析 scope；GC 只删除无任何引用的 artifact，先事务删除元数据/扣额度，再幂等删除 blob 并记录失败重试。
- [x] **Step 5: 验证**：运行 `cd backend && cargo test --test artifact_store && cargo test`。
- [x] **Step 6: Commit**：`git commit -m "功能：实现 artifact 存储与历史配额"`。

### Task 6: Workspace、不可变 ModelVersion 与 active/draft 事务

**Files:**
- Create: `backend/migrations/00000000000004_model_workspace/up.sql`
- Create: `backend/migrations/00000000000004_model_workspace/down.sql`
- Create: `backend/src/db/repositories/workspace.rs`
- Create: `backend/src/application/workspace.rs`
- Create: `backend/src/api/workspaces.rs`
- Test: `backend/tests/workspace_versions.rs`
- Modify: `backend/src/db/{schema.rs,models.rs}`
- Modify: `backend/src/api/mod.rs`

**Interfaces:**
- Produces: `WorkspaceService::{get_or_create,publish_version,activate,rollback}`；`publish_version(workspace_id, expected_base, VersionDraft) -> ModelVersion`。
- Invariant: published version rows and manifests are immutable; active 只能指向同 workspace 的 ready；draft 与 active 不得相同。

- [x] **Step 1: 写失败测试**：首次 ready 自动 active；已有 active 后上传/案例导入只推进 draft；origin kind/ref 进入不可变 manifest；错误 expected base 返回 `409 version_conflict`；激活事务同时更新 active 并清空 draft；回退不改历史任务。
- [x] **Step 2: 验证失败**：运行 `cd backend && cargo test --test workspace_versions`。
- [x] **Step 3: 实现迁移与 repository**：建立 workspaces/model_versions/version_artifacts/dataframe_bindings 及约束；model version 区分 source kind 与 `upload/example/generated` origin；manifest 使用 canonical JSON 后计算 content hash。
- [x] **Step 4: 实现读取 API**：`GET /sessions/{id}/workspace`、`GET /sessions/{id}/versions`、`GET /versions/{id}` 全部执行 scope 和 session 归属校验。
- [x] **Step 5: 发布 model-status**：事务提交后分别发布 active/draft 完整快照；清空 draft 必须发布 `snapshot:null`。
- [x] **Step 6: 验证**：运行 `cd backend && cargo test --test workspace_versions && cargo test`。
- [x] **Step 7: Commit**：`git commit -m "功能：建立不可变模型工作区"`。

### Task 7: 版本化引擎合同、Agent skill bundle 与 DataFrame 类型推断

**Files:**
- Create: `backend/resources/agent/v2/{manifest.json,system.md,skill-index.json}`
- Create: `backend/resources/agent/v2/skills/{model-intake.md,study-planning.md,rss-generation.md,result-analysis.md,comparison.md,troubleshooting.md}`
- Create: `backend/src/engine_resources/{mod.rs,knowledge.rs,examples.rs,snapshot.rs}`
- Create: `backend/src/catalog/{mod.rs,schema.rs,inference.rs}`
- Create: `backend/src/agent/knowledge.rs`
- Create: `backend/src/ingestion/{mod.rs,tabular.rs}`
- Create: `backend/tests/catalog_inference.rs`
- Create: `backend/tests/agent_knowledge.rs`
- Create: `backend/tests/engine_resources.rs`
- Create: `backend/tests/fixtures/engine_resources/`
- Create: `backend/tests/fixtures/inference/`
- Modify: `backend/Cargo.toml`
- Modify: `backend/src/config.rs`

**Interfaces:**
- Produces: `AgentKnowledge::load_embedded(bundle_version)`、`EngineResourceRegistry::load(tensoreval_cwd) -> EngineResourceSnapshot`、`SchemaCatalog::from_snapshot(contract_version)`、`TabularInspector::inspect(path, format) -> Vec<TableSchema>`、`infer_table(schema, catalog) -> InferenceEvidence`。
- Scoring: required coverage 0.75、optional coverage 0.15、stable-key coverage 0.10；自动推断要求第一名 ≥0.90、无缺失 required 且领先第二名 ≥0.10，否则 pending。

- [x] **Step 1: 固化资源来源**：agent manifest 记录允许的 eig-rs/sparrowzz commit、两个 catalog SHA-256、schema version、skill 文件 hash、允许工具和兼容关系；工作树有未提交修改时记录 dirty 状态且不能把未验证内容混入发布合同。
- [x] **Step 2: 写知识资源失败测试**：缺 docs/catalog、hash 不符、路径逃逸、重复 skill ID、未知 required tool、bundle/contract 不兼容均使脚本生成 preflight 失败；system prompt 不含路径、密钥或函数/schema 全集。
- [x] **Step 3: 写推断参数化测试**：catalog 每种输入 schema 至少一个正例；另覆盖 BOM/大小写/空白、未知表、相似 schema、dtype 冲突、重复表、多工作表与空工作表。
- [x] **Step 4: 写文档/案例目录测试**：按标题切分 11 份 Markdown，全文搜索返回稳定 section/hash；发现 16 个根 RSS 和 37 个 bundle；无 manifest、依赖闭包不全、文件 hash 变化和缺 required feature 的案例不可导入。
- [x] **Step 5: 验证失败**：运行 `cd backend && cargo test --test agent_knowledge --test engine_resources --test catalog_inference`。
- [x] **Step 6: 实现 snapshot 和 reader**：从 `TENSOREVAL_CWD/sparrowzz/dss` 读取允许子树，原子发布 docs/catalog/examples 快照；CSV 只读 header 和有限采样；Excel 每 sheet 独立；Parquet 从 footer 读 schema；文件名和 sheet 名不得进入评分，dtype 只写诊断。
- [x] **Step 7: 实现检索和推断**：提供有字符上限的文档 search/read；生成包含候选分数、命中、缺失和类型冲突的 inference evidence；引擎合同不匹配时返回 `engine_contract_mismatch`。
- [x] **Step 8: 验证**：运行 `cd backend && cargo test --test agent_knowledge --test engine_resources --test catalog_inference && cargo test`。
- [x] **Step 9: Commit**：`git commit -m "功能：预载 Agent 知识与引擎案例"`。

### Task 8: DataFrame 多文件上传、幂等摄取和局部失败

**Files:**
- Create: `backend/migrations/00000000000005_ingestions/up.sql`
- Create: `backend/migrations/00000000000005_ingestions/down.sql`
- Create: `backend/src/db/repositories/ingestion.rs`
- Create: `backend/src/application/ingestion.rs`
- Create: `backend/src/api/ingestions.rs`
- Test: `backend/tests/dataframe_ingestion.rs`
- Modify: `backend/src/api/sessions.rs`

**Interfaces:**
- Produces: `IngestionService::accept_batch(principal, session_id, UploadBatch) -> AcceptedBatch` 和后台 `process(ingestion_id)`。
- Multipart: `clientBatchId`、`file.<clientFileId>`、可选 `relativePaths`；粗粒度 attachment 状态为 accepted/ingesting/ready/failed。
- DB: ingestion 保存服务端确定的 `origin_kind=upload|example` 和可空 `source_ref_json`；客户端不能提交 example 源路径。

- [x] **Step 1: 写幂等与乱序测试**：相同 batch/file/bytes 返回同 ingestion；改变 bytes 返回 409；同名文件不同 file ID 可并存；局部失败产生 `completed_with_errors` 和 collecting draft。
- [x] **Step 2: 写配额/恢复测试**：刷新后 `GET /ingestions/{id}` 返回完整 item 状态和 stages；配额失败不推进 draft；旧 base version 完成时返回 version_conflict。
- [x] **Step 3: 验证失败**：运行 `cd backend && cargo test --test dataframe_ingestion`。
- [x] **Step 4: 实现接收与状态机**：请求只负责流式落 staging、建 ingestion/items 并返回 202；后台按 workspace 锁依次执行 inspect/infer/validate/publish。
- [x] **Step 5: 规范化 Excel**：每个非空 sheet 生成独立 Parquet artifact，保留 source artifact/sheet name；CSV/Parquet 原文件保留，绑定记录读取到的 schema。
- [x] **Step 6: 发布事件**：每次粗粒度状态变化落 attachment 快照；每个阶段落完整 progress；最终发布 model-status。
- [x] **Step 7: 验证**：运行 `cd backend && cargo test --test dataframe_ingestion && cargo test`。
- [x] **Step 8: Commit**：`git commit -m "功能：实现多 DataFrame 幂等摄取"`。

### Task 9: ZIP、RSS 和 `.rssbundle` 安全摄取

**Files:**
- Create: `backend/src/ingestion/{archive.rs,rss.rs,bundle.rs}`
- Create: `backend/src/application/example_import.rs`
- Create: `backend/tests/bundle_ingestion.rs`
- Create: `backend/tests/example_import.rs`
- Create: `backend/tests/fixtures/bundles/`
- Modify: `backend/Cargo.toml`
- Modify: `backend/src/application/ingestion.rs`

**Interfaces:**
- Produces: `ArchiveExtractor::extract(staged_zip, limits) -> ExtractedTree`、`BundleValidator::validate(tree) -> BundleManifest`、`RssInspector::inspect(entry, tree) -> DependencyReport`、`ExampleImportService::import(scope, workspace, example_id, snapshot, expected_base: Option<VersionId>) -> ModelVersion`。
- Defaults: 单文件 50 MiB、批次 500 MiB、最多 1,000 个条目、展开 2 GiB、压缩比 100；全部可由配置下调或上调。

- [x] **Step 1: 写安全测试**：拒绝绝对路径、`..`、NUL、符号链接、重复规范化路径、超文件数/大小/压缩比；失败不得在 CAS 或版本中留下引用。
- [x] **Step 2: 写功能测试**：单 RSS 缺依赖为 collecting；合法 ZIP 和目录增强 `.rssbundle` 得到相同规范 manifest/content hash；入口/manifest/hash 错误为结构化诊断。
- [x] **Step 3: 写案例导入测试**：按 example ID 导入根 RSS 的完整依赖闭包或 bundle 全树；覆盖 dss/models、psa/tests/bmopf、sparrowzz/rspower/lib 根内依赖和根外逃逸拒绝；空工作区自动 active、已有 active 只推进 draft，导入本身不创建仿真任务；重复导入幂等；snapshot/hash 变化、能力缺失、跨 scope 和配额超限均不发布半成品版本。
- [x] **Step 4: 验证失败**：运行 `cd backend && cargo test --test bundle_ingestion --test example_import`。
- [x] **Step 5: 实现隔离解包**：只写 ingestion staging 子目录，逐 entry 检查限制，禁止 follow symlink；完成后每个文件分别提交 artifact。
- [x] **Step 6: 实现 bundle/RSS 与案例校验**：解析 manifest、入口和显式依赖闭包，虚拟路径统一相对 `TENSOREVAL_CWD` 且使用 `/` 语义；案例只从当前 EngineResourceSnapshot 按 ID 解析，复制后再次校验 hash，永不修改共享源目录。
- [x] **Step 7: 验证**：运行 `cd backend && cargo test --test bundle_ingestion --test example_import && cargo test`。
- [x] **Step 8: Commit**：`git commit -m "功能：支持 RSS、rssbundle 与内置案例摄取"`。

### Task 10: 工作区自然语言工具、替换/移除/激活与结构化修改

**Files:**
- Create: `backend/src/tools/{inspect_workspace.rs,confirm_dataframe_mapping.rs,replace_model_input.rs,remove_model_input.rs,activate_model_version.rs,modify_model.rs}`
- Create: `backend/src/tools/load_agent_skill.rs`
- Create: `backend/src/tools/{search_engine_docs.rs,read_engine_doc_section.rs,search_examples.rs,inspect_example.rs,import_example.rs}`
- Create: `backend/src/agent/prompt.rs`
- Create: `backend/src/application/model_patch.rs`
- Create: `backend/tests/workspace_tools.rs`
- Modify: `backend/src/tools/mod.rs`
- Modify: `backend/src/agent/mod.rs`
- Modify: `backend/src/application/workspace.rs`

**Interfaces:**
- Produces: `PromptAssembler`、prompt/文档/案例只读工具、`import_example` 和 spec §7 的六个工作区工具；所有变更工具都要求 `expected_draft_version_id`，仅在 active/draft 均不存在时允许为 null，目标使用 version input ID；`import_example` 另要求 engine resource snapshot。
- Patch: `ModelPatch { table_kind, row_key, changes }`；每个 change 必须含 from/to/unit，应用前验证旧值和唯一行。

- [x] **Step 1: 写 prompt/skill/知识工具测试**：每轮只固定注入 system、skill index 和资源摘要；合法 skill 返回 id/version/hash/content；文档工具只接受 doc/section ID 且限制字符数；案例工具只接受 example ID；未知资源被拒绝；脚本生成工具缺必需 skill 时返回 `skill_required`。
- [x] **Step 2: 写工作区工具测试**：参数中不得出现 path/filename 作为目标标识；缺 expected version、重复行、旧值不符、单位不兼容均失败且不产生版本；替换/移除创建子版本，确认 mapping 保留证据，激活/回退保持历史任务不变；案例导入版本修改负荷/故障/时步时只生成用户子版本，无法唯一定位的执行计划参数 fail closed。
- [x] **Step 3: 验证失败**：运行 `cd backend && cargo test --test workspace_tools`。
- [x] **Step 4: 实现服务和工具**：工具只调 application service；DataFrame 补丁写新 Parquet，RSS 补丁写新脚本并重新检查；所有产物先经 ArtifactService。
- [x] **Step 5: 实现 prompt 与技能装配**：注册 `load_agent_skill`，在当前 run 记录 `LoadedSkillSet`；加入稳定 ID、自然语言确认和禁止路径规则；技能/工具输出设置大小上限。
- [x] **Step 6: 验证**：运行 `cd backend && cargo test --test workspace_tools && cargo test`。
- [x] **Step 7: Commit**：`git commit -m "功能：增加模型版本自然语言工具"`。

### Task 11: tensoreval v2 合同、Parquet 输出验证与适配器拆分

**Files:**
- Move: `backend/src/adapters/process.rs` → `backend/src/adapters/process/{mod.rs,command.rs,output.rs}`
- Create: `backend/src/adapters/contract.rs`
- Create: `backend/src/storage/parquet.rs`
- Create: `backend/tests/process_engine_v2.rs`
- Modify: `backend/src/adapters/mod.rs`
- Modify: `backend/Cargo.toml`

**Interfaces:**
- Produces: `EngineCapabilities::probe(binary) -> Capabilities`；`SimEngine::run_dataframe(RunSpec) -> EngineRunOutput`；`ParquetInspector::{metadata,page,projection}`。
- `RunSpec` 包含受控 task-local input tree/cwd、script path、`Vec<NamedInput>`、staging result dir、timeout；只能由 application 解析 artifact ID 并物化版本后创建。

- [x] **Step 1: 写命令测试**：仿真、bundle、DataFrame 和结果分析四种命令都以任务私有 input tree 为 cwd，并精确包含 `--result-dir` 与 `--output-format parquet`；input alias 合法化且不经过 shell；已导入案例运行时删除/修改共享源目录不影响本次物化输入。
- [x] **Step 2: 写输出测试**：拒绝目录外 symlink、非法变量名、非 parquet、损坏 footer、重复 result name、超文件数/单文件/总字节；读取有序 schema 和 row count。
- [x] **Step 3: 验证失败**：运行 `cd backend && cargo test --test process_engine_v2`。
- [x] **Step 4: 替换适配器合同**：删除 CSV 聚合、固定 `RunMetrics`、旧 `read_curves/read_report`；把命令与输出校验拆开，StubEngine 返回最小合法 Parquet fixture。
- [x] **Step 5: 启动探测**：缺 `--output-format`、Parquet input 或合同版本不符时启动失败，不提供旧二进制降级路径。
- [x] **Step 6: 验证真实二进制**：运行 capability probe 和一份最小 RSS smoke test，输出必须是 Snappy Parquet；CI 无真实 binary 时以显式 ignored integration test 保留命令。
- [x] **Step 7: Commit**：`git commit -m "功能：接入 tensoreval Parquet v2 合同"`。

### Task 12: v2 仿真任务、结果数据集登记与只读 API

**Files:**
- Create: `backend/migrations/00000000000006_result_datasets/up.sql`
- Create: `backend/migrations/00000000000006_result_datasets/down.sql`
- Create: `backend/src/db/repositories/{simulation.rs,dataset.rs}`
- Create: `backend/src/application/{simulations.rs,datasets.rs}`
- Create: `backend/src/tools/{run_study.rs,list_result_datasets.rs}`
- Create: `backend/src/api/datasets.rs`
- Test: `backend/tests/v2_simulation.rs`
- Modify: `backend/src/api/{mod.rs,tasks.rs}`

**Interfaces:**
- Produces: `SimulationService::run(RunStudyRequest) -> SimulationOutcome`；`DatasetService::{list,page,download,metadata}`。
- DB: simulation_tasks 新增 model_version_id/study_spec_json/script_artifact_id/agent_bundle_version/agent_skill_refs_json/engine_contract_version；本任务一次性创建 result_analysis_runs/result_analysis_inputs/result_datasets/dataset_views 完整关系，result_datasets 的两个 producer FK 恰一非空。分析行为到 Task 13 再启用。

- [x] **Step 1: 写成功测试**：ready version + script 产生 task、script artifact、实际 agent bundle/skill refs/engine contract 和多个 named datasets；schema/rows 正确；result 消息只含 outputs，不含固定 metrics。
- [x] **Step 2: 写原子失败测试**：engine 非零、坏 Parquet、配额超限或 DB 提交失败均不发布成功 result，staging 被清理，task 保存稳定错误。
- [x] **Step 3: 写访问测试**：另一个 owner 对 task/dataset/rows/download 全为 404；下载 MIME 和清理后的文件名正确。
- [x] **Step 4: 实现任务编排**：prepare→inspect→sim→collect，每阶段发完整 progress；`run_study` 要求本轮已加载 `study-planning` 与 `rss-generation`，任务始终绑定版本、study spec、实际脚本 artifact 和 agent/engine 版本。
- [x] **Step 5: 实现数据集读取**：rows 默认 100、最大 500；cursor 编码 dataset/schema hash/row-group/offset 并严格校验；NaN/Inf/缺失值出网为 null 并记诊断。
- [x] **Step 6: 删除原型结果路径**：删除 `/curves`、`/report`、固定 metrics、task_results/curve_data 写入和相关 DTO；所有任务必须有 model_version_id 并只产生 result_datasets。
- [x] **Step 7: 验证**：运行 `cd backend && cargo test --test v2_simulation && cargo test`。
- [x] **Step 8: Commit**：`git commit -m "功能：持久化通用 Parquet 仿真结果"`。

### Task 13: 结果分析、VisualizationSpec 和六类图表数据 API

**Files:**
- Create: `backend/src/db/repositories/analysis.rs`
- Create: `backend/src/application/{analysis.rs,visualization.rs}`
- Create: `backend/src/tools/{run_result_analysis.rs,create_dataset_view.rs}`
- Create: `backend/src/api/analysis.rs`
- Create: `backend/tests/result_analysis.rs`
- Modify: `backend/src/api/mod.rs`

**Interfaces:**
- Produces: `AnalysisService::run(AnalysisRequest) -> AnalysisOutcome`、`VisualizationValidator::validate(dataset_schema, spec, budgets) -> ProjectionPlan`。
- View kinds: line/bar/scatter/heatmap/boxplot/histogram；默认预算为 50,000 点、100 series、5,000 categories、100,000 heatmap cells。

- [x] **Step 1: 写脚本安全测试**：拒绝远程 include/import、绝对路径、未绑定 alias、未声明输出和超输出数；LLM 参数中不存在磁盘路径。
- [x] **Step 2: 写血缘测试**：保存 intent/request message/script artifact/agent bundle/skill refs/engine contract/有序 inputs/outputs；派生 dataset 可再次作为输入但不能形成当前/未来引用。
- [x] **Step 3: 写 view 参数化测试**：六种合法 spec 通过；缺列、dtype 不符、kind 不符、函数 formatter、远程资源、规模超限分别返回稳定 422 code；超限不返回前 N 行。
- [x] **Step 4: 实现分析状态机**：`run_result_analysis` 要求本轮已加载 `result-analysis`；script→run→collect→present；输出提交、初始 views、状态和 result 消息在同一事务完成。
- [x] **Step 5: 实现 API**：analysis detail、dataset metadata/rows/views/download、view data；纯换图只追加 view 并重新发布该 producer 的 result 快照，不运行 tensoreval。
- [x] **Step 6: 验证**：运行 `cd backend && cargo test --test result_analysis && cargo test`。
- [x] **Step 7: Commit**：`git commit -m "功能：增加自然语言结果分析与语义视图"`。

### Task 14: 两个历史任务的可追溯比较

**Files:**
- Create: `backend/src/db/repositories/comparison.rs`
- Create: `backend/src/application/comparisons.rs`
- Create: `backend/src/tools/compare_simulations.rs`
- Create: `backend/src/api/comparisons.rs`
- Create: `backend/tests/task_comparison.rs`
- Create: `backend/migrations/00000000000007_task_comparisons/up.sql`
- Create: `backend/migrations/00000000000007_task_comparisons/down.sql`

**Interfaces:**
- Produces: `ComparisonService::compare(scope, session_id, baseline_task_id, candidate_task_id, request_message_id) -> TaskComparison`。
- Compatibility output 固化左右 dataset、列、dtype、单位、稳定键和维度；数值差异只来自关联 analysis run 的 Parquet。

- [x] **Step 1: 写失败测试**：同一任务、自不同 session/owner、无稳定键、单位不兼容和所需结果缺失均返回 `comparison_incompatible` 或 404，且不创建半成品 comparison。
- [x] **Step 2: 写成功测试**：普通表按 catalog 稳定键 join，QSTS 额外按时间 join；输出含 key/baseline/candidate/absolute/relative，baseline=0 时 relative 为 null，缺失对象进诊断表。
- [x] **Step 3: 验证失败**：运行 `cd backend && cargo test --test task_comparison`。
- [x] **Step 4: 实现服务与工具**：工具只接两个 task ID，并要求本轮已加载 `comparison`；服务选择数据集、生成受控 RSS、调用 AnalysisService 并绑定 comparison。
- [x] **Step 5: 发布/读取**：落 comparison 消息并实现 `GET /api/task-comparisons/{id}`；刷新后可沿 comparison→analysis→dataset 恢复。
- [x] **Step 6: 验证**：运行 `cd backend && cargo test --test task_comparison && cargo test`。
- [x] **Step 7: Commit**：`git commit -m "功能：增加仿真任务结果对比"`。

### Task 15: 历史分页、中文全文检索、自动标题与 Agent 历史工具

**Files:**
- Create: `backend/migrations/00000000000008_chat_history_fts/up.sql`
- Create: `backend/migrations/00000000000008_chat_history_fts/down.sql`
- Create: `backend/src/db/repositories/history.rs`
- Create: `backend/src/application/history.rs`
- Create: `backend/src/tools/{search_chat_history.rs,read_chat_context.rs,list_simulation_history.rs}`
- Create: `backend/tests/history_search.rs`
- Modify: `backend/src/api/sessions.rs`

**Interfaces:**
- Produces: `HistoryService::{search_sessions,list_messages,read_context,rebuild_index}`；绑定查询条件的不透明 cursor。
- FTS: external-content `chat_search_fts` 使用 trigram；1–2 字查询只在 owner scope 内执行带 limit 的转义 LIKE。

- [x] **Step 1: 写启动能力测试**：FTS5/trigram 不可用时历史搜索返回 `history_search_unavailable`，普通会话读取仍可用，禁止退化为无界扫描。
- [x] **Step 2: 写检索测试**：中文子串、附件名、标题、日期、study type/status AND 过滤、bm25+时间稳定排序、删除后索引清理、跨 owner 零命中。
- [x] **Step 3: 写分页测试**：最近 50、beforeId、aroundId 窗口；非法组合、篡改 cursor、查询变化复用 cursor 分别返回稳定 400。
- [x] **Step 4: 实现事务索引和自动标题**：只索引允许字段；默认/auto/user 标题规则；重命名后永不被自动标题覆盖；提供离线 `rebuild-search-index` 子命令。
- [x] **Step 5: 实现三个工具**：默认最多 5 个搜索候选、上下文窗口有硬上限；“上次/修改前”先查当前 session，多个相近候选必须返回确认需求。
- [x] **Step 6: 验证**：运行 `cd backend && cargo test --test history_search && cargo test`。
- [x] **Step 7: Commit**：`git commit -m "功能：增加历史检索与消息分页"`。

### Task 16: 前端状态架构、身份恢复、会话历史与模型上传

**Files:**
- Create: `frontend/src/types/{session.ts,message.ts,workspace.ts,task.ts,dataset.ts,index.ts}`
- Create: `frontend/src/hooks/{useSessionFeed.ts,useWorkspace.ts,useIngestion.ts,useExamples.ts}`
- Create: `frontend/src/state/{sessionFeed.ts,workspace.ts}`
- Create: `frontend/src/components/{history/HistorySidebar.tsx,workspace/WorkspacePanel.tsx,uploads/UploadQueue.tsx,examples/ExampleLibrary.tsx,examples/ExampleDetail.tsx}`
- Create: `frontend/src/**/*.test.ts{x}`
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/api/{client.ts,events.ts}`
- Modify: `frontend/src/components/Composer.tsx`
- Modify: `frontend/package.json`

**Interfaces:**
- Consumes: Tasks 2、4、6–10、15 的 REST/SSE 契约。
- Produces: 按 session 键控的 feed/workspace store；`localStorage[zjudss:last-session:<scopeKey>]` 仅作导航偏好。

- [x] **Step 1: 写 v2 前端合同测试**：为消息 reducer、session 恢复和 workspace 状态编写测试，不保留旧消息渲染契约。
- [x] **Step 2: 写 reducer 测试**：operation、callId、batch/file、active/draft、producer+dataset 归并；旧 session 的迟到响应被丢弃；snapshot null 清空 draft。
- [x] **Step 3: 拆分 App 状态**：App 只保留布局/选中 ID；hooks 负责订阅就绪→并行快照→缓存事件→归并→实时消费。
- [x] **Step 4: 实现身份与历史恢复**：先取 `/api/me`，仅恢复当前 scope 可访问的 last session，否则打开最近会话；搜索、筛选、无限滚动和 around 消息定位使用服务端 cursor。
- [x] **Step 5: 实现多文件/目录上传**：生成稳定 batch/file ID；目录上传携带规范相对路径；本地 queued/uploading 与服务端状态按 batch+file 合并；修正后的文件生成新 ID。
- [x] **Step 6: 实现案例库**：只读搜索/筛选/详情显示验证状态、规模和 required features；“使用此案例”发送带 example ID/snapshot context 的可见自然语言消息，不直接调用导入服务。
- [x] **Step 7: 实现 workspace 面板**：同时显示 active/draft、输入 ID、来源案例、推断证据、缺失项和活动摄取；替换/激活等建议点击后发送可见自然语言消息，不直接调用写 API。
- [x] **Step 8: 验证**：运行 `cd frontend && npm test -- --run && npm run build`。
- [x] **Step 9: Commit**：`git commit -m "前端：接入 v2 历史、案例、上传与模型工作区"`。

### Task 17: 前端任务历史、DataFrame 表格、多视图 ECharts 与比较

**Files:**
- Create: `frontend/src/hooks/{useSimulationHistory.ts,useTaskDatasets.ts,useAnalysisRun.ts,useDataset.ts,useDatasetView.ts,useComparison.ts,useStorageUsage.ts}`
- Create: `frontend/src/components/datasets/{DatasetTable.tsx,ChartRenderer.tsx,DatasetWorkbench.tsx}`
- Create: `frontend/src/components/datasets/charts/{line.ts,bar.ts,scatter.ts,heatmap.ts,boxplot.ts,histogram.ts}`
- Create: `frontend/src/components/tasks/{TaskHistory.tsx,ComparisonView.tsx}`
- Test: `frontend/src/components/datasets/*.test.tsx`
- Modify: `frontend/src/components/{ResultPanel.tsx,messages/ResultCard.tsx,messages/MessageItem.tsx}`

**Interfaces:**
- Consumes: Tasks 12–14 的 dataset/view/comparison API。
- Produces: DataFrame 必有分页表格；保存的 view 由本地受控 builder 编译为 ECharts option。

- [x] **Step 1: 写表格测试**：列顺序遵守 schema、cursor 分页去重、null 正确显示、大表不进入全局 message state、下载保持 Parquet。
- [x] **Step 2: 写六类 builder 参数化测试**：每种 spec 只生成允许的 ECharts 配置；不执行 formatter 字符串；单位进入 axis/tooltip；histogram 保留 bin 边界。
- [x] **Step 3: 写规模/错误测试**：422 超限展示服务端规模与“可在对话中要求筛选/聚合”，不画部分数据；单个数据区域失败不清空聊天/workspace。
- [x] **Step 4: 实现工作台**：固定 380px 面板升级为可调整宽度/全宽详情层，窄屏仍为抽屉；结果区为“数据表 | 已保存视图…”，现有 view 切换是只读固定控件。
- [x] **Step 5: 实现任务与比较**：展示 task version/study spec/datasets，比较卡沿 ID 恢复；选择两任务只产生自然语言建议，点击后发送可见 user 消息。
- [x] **Step 6: 接入配额**：80% 告警不阻塞；硬限制错误保留待上传/待发送内容；历史读取和下载继续可用。
- [x] **Step 7: 验证**：运行 `cd frontend && npm test -- --run && npm run build`。
- [x] **Step 8: Commit**：`git commit -m "前端：增加 DataFrame 多视图与仿真对比工作台"`。

### Task 18: v2 schema 收口、端到端验收、容量验证与发布

**Files:**
- Create: `backend/tests/e2e_v2.rs`
- Create: `frontend/src/e2e/`（若采用 Playwright，则在本任务加入并锁定版本）
- Create: `docs/v2-deployment.md`
- Modify: `docs/development-status.md`
- Modify: `.env.example`（若仓库存在；不存在则创建 `backend/.env.example`，不得复制真实密钥）

**Interfaces:**
- Consumes: Tasks 1–17 的全部能力。
- Produces: 单一 v2 schema、可重复验收脚本、开发数据重置与可信代理部署说明。

- [x] **Step 1: 四类输入与共享 E2E 链路**：单 RSS、ZIP/目录 rssbundle、分批 DataFrame、内置案例导入分别验证到不可变 ModelVersion；下游以 ModelVersion 为统一边界，用共享 E2E 完成修改→运行→自然语言分析→表格/图表→重跑→比较→刷新恢复，避免按来源复制同一套仿真测试；案例覆盖根 RSS 和已发布 rssbundle。
- [x] **Step 2: 历史与身份 E2E**：两个 owner 并发操作，验证聊天/文件/task/dataset/SSE/搜索完全隔离；中文关键词、日期、分析类型、模糊“上次”和深层消息定位可用。
- [x] **Step 3: 故障 E2E**：进程执行失败、断开 SSE、刷新中摄取、重复请求、坏 Parquet、满队列、满配额和服务重启后没有成功假象或孤立引用。
- [x] **Step 4: 容量验证**：记录 8 个 agent run、4 个 engine process、owner 2 个 run、32 队列以及默认图表预算下的内存/延迟；结果不达标时只调配置或修复瓶颈，不扩大未测上限。
- [x] **Step 5: schema 收口**：删除 `task_results/curve_data`、旧 CSV reader、固定 metrics、旧 `/curves`、`/report` 和旧消息 DTO；把开发期增量 migration 压成一份全新 v2 initial migration，只验证空库升级。明确要求删除旧本地 DB/DATA_DIR 后重新初始化。
- [x] **Step 6: 发布门禁**：capability、docs/catalog/examples snapshot、agent bundle hash 与兼容关系、FTS/auth/storage preflight 全部通过才接流量；公开部署拒绝 instance auth，不提供 v1 feature flag 或运行时回退。
- [x] **Step 7: 全量验证**：运行 `cd backend && cargo fmt --check && cargo test`、`cd frontend && npm test -- --run && npm run build`、`git diff --check`。
- [x] **Step 8: 文档状态更新**：只把真实完成的表项改为 ✅，记录迁移版本、运行时 tensoreval 合同和剩余限制。
- [x] **Step 9: Commit**：`git commit -m "发布：完成配电网仿真智能体 v2"`。

## Execution Rules

1. 严格按 Task 顺序执行；一个 Task 未通过自身测试和 review，不开始依赖它的 Task。
2. M0–M4 每个里程碑结束都部署到独立测试环境，用真实 SQLite 文件和持久 DATA_DIR 做一次重启恢复。
3. Tasks 2–17 的 migration 在开发期间只追加；Task 18 在发布前将它们与未发布的 v1 migration 一次性压成干净 v2 initial schema，此后 migration 才进入永久只追加规则。
4. 每个 Task 一个提交；若 review 要求修改，修复提交紧跟原提交，里程碑结束再决定是否 squash。
5. 任何实现发现契约缺口时，先修改对应设计文档和本计划，再写代码；不得只在代码中形成隐含规则。
6. v2 前后端必须同步切换；不实现 v1 消息、旧 CSV、固定 metrics、旧结果卡或旧数据库的兼容层。
