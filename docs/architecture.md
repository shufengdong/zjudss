# 后端架构设计

> 版本：v1 草案 · 状态：待评审
> 配套：数据库设计见 `docs/db-design.md`；前端契约见 `frontend/src/types/index.ts`
> 说明：本文只描述架构与职责划分，不包含具体实现。引擎结论以 `../eig-rs/sparrowzz/dss/docs/` 为准（见 db-design §0）。

---

## 0. 目标

把「配电网仿真智能体」后端从当前的单文件 demo（`main.rs` 硬编码工具）演进为**分层、可扩展、
可观测**的服务，核心能力：

1. 把一个 rig agent 的「黑盒对话循环」变成**结构化事件流**，前端能实时渲染工具调用、进度、结果；
2. 落实 AGENTS.md 的硬约束：**所有内核调用收敛到 `adapters/` 适配器层**；
3. 把「同步阻塞的引擎 CLI」包装成「有状态、可追溯的异步仿真任务」。

---

## 0.5 多用户：先匿名、预留扩展（已确认决策）

**v1 不做账号体系，但把多用户扩展点预留到位。** 依据：DB 无 `users` 表、无 `user_id` 外键；
前端无登录态；AGENTS.md 只要求内核 API 鉴权（收敛在适配器），均未提多用户。

- **多人同时用**：架构天然支持——`session` 独立、事件流按 session 隔离，多浏览器/多人
  各自建会话、各自对话、互不干扰，无需任何用户管理。
- **不做的（v1）**：账号注册/登录、认证中间件、多租户数据隔离、权限/角色。
- **会话归属语义（v1 定死）**：所有 `sessions` 属于当前部署实例，`GET /api/sessions` 返回全部。
- **预留 = 路径预留，不建空结构**（以下三点落地在设计与实现习惯里，而非空表/空列）：
  1. **迁移天然支持**：Diesel 版本化迁移（已有 `00000000000000_initial`）意味着未来加
     `user_id` 是新增一条迁移文件，不破坏既有数据。
  2. **repository 查询预留过滤维度**：`SessionRepo`/`MessageRepo` 的查询方法预留
     `user_id: Option<i32>` 形参（v1 传 `None`），未来传 `Some(id)` 即可按用户过滤，不改调用方。
  3. **鉴权插入点明确在 api 层**：未来加 `axum` 中间件（token → `user_id` → 应用层过滤），
     是 router 上的纯加法，业务/应用层不感知。

---

## 1. 现状与差距

| 关注点 | 现状 | 目标 |
|---|---|---|
| 工具定义 | `#[rig_tool]` 硬编码在 `main.rs`，返回 `"model ok"` / `"task-0001"` | 独立 `tools/`，有状态实例，走适配器 |
| 适配器层 | **不存在**（AGENTS.md 要求） | `adapters/` 作为内核唯一边界 |
| agent 生命周期 | 每个请求重建，`agent.prompt()` 一次性返回文本 | 每请求构建 + 流式驱动 |
| 输出形态 | `{ reply }` 单次 JSON | SSE 结构化事件流（对齐前端 6 类消息） |
| 会话持久化 | 无（DB 已建但未接入） | 全链路落库，DB 为持久化真相 |
| 仿真执行 | 无（硬编码 task_id） | 子进程跑引擎 + 限流 + 状态机 |

---

## 2. 前端契约（决定后端输出形态）

前端 `types/index.ts` 把消息定义为 6 类 discriminated union，后端的事件流必须能映射到这 6 类：

| 前端类型 | 关键字段 | 后端事件来源 |
|---|---|---|
| `user` | content | 入站消息（直接落库、回放） |
| `assistant` | content(Markdown) | agent 最终回复（`FinalResponse`） |
| `tool-call` | toolName / args / status(running\|success\|failed) / summary | 工具调用的开始与结果 |
| `progress` | taskId / stages[](pending\|running\|done\|failed) | 仿真任务阶段推进 |
| `result` | taskId / title / metrics(maxVoltage·minVoltage·violationNodes·violationDuration) | `task_results` 聚合 |
| `attachment` | file 信息 | 上传/产出的文件（模型、报告） |

**推论**：后端必须把 agent 循环内部发生的「调用工具 → 工具执行 → 出结果 → 最终回复」逐条
**发布成事件**，而不是等 agent 跑完一次性返回。这是本架构的轴心。

---

## 3. 分层架构

自顶向下六层，依赖方向单向向下（上层不跨层调用，工具只依赖其下两层）：

```
┌──────────────────────────────────────────────────────────┐
│ api（HTTP 层）    axum router / handler / DTO / SSE        │
│   POST /api/sessions/{id}/messages   触发 agent，返回 ack  │
│   GET  /api/sessions/{id}/events     SSE 订阅事件流         │
│   GET  /api/sessions/{id}/messages   回放历史               │
│   GET  /api/tasks/{id}/curves        曲线                   │
├──────────────────────────────────────────────────────────┤
│ application（编排层） 会话编排 / 任务状态机 / 事件总线 / 限流 │
│   ChatService、SimulationOrchestrator、EventBus            │
├──────────────────────────────────────────────────────────┤
│ agent（agent 层）   provider 选择 / preamble / 工具装配 / 流 │
│   llm::build_agent 扩展：run_channel() 驱动                │
├──────────────────────────────────────────────────────────┤
│ tools（工具层）   有状态工具实例，只调 adapters + repository │
│   InspectModel、RunSimulation（+ 未来更多）                 │
├──────────────────────────────────────────────────────────┤
│ adapters（适配器层） 内核唯一边界（AGENTS.md 硬约束）         │
│   SimEngine：inspect / run（子进程 + 错误码翻译）            │
├──────────────────────────────────────────────────────────┤
│ db（持久层）   Diesel pool + repository（6 表）             │
│   SessionRepo / MessageRepo / TaskRepo / ResultRepo         │
└──────────────────────────────────────────────────────────┘
```

职责与关键约束：

- **api**：只做 HTTP 协议与 DTO 转换，不含业务逻辑；SSE 端点把内部事件序列化为前端消息。
- **application**：持有 `AppState`（连接池、事件总线、限流信号量、引擎适配器、LLM 配置），
  编排一次对话的生命周期；是唯一知道「如何把一次 chat 变成 agent run + 落库 + 推事件」的地方。
- **agent**：现有 `llm.rs` 的扩展——provider 解耦保留，新增用 `run_channel()` 驱动 run 的能力。
- **tools**：**有状态实例**（见 §5），只依赖 `adapters` 与 `db`，绝不直接碰内核 API。
- **adapters**：内核地址、鉴权、错误码翻译、子进程生命周期都收敛在此。
- **db**：已就绪；新增 repository 封装 CRUD，上层不直接拼 SQL。

---

## 3.5 Prompt 与领域知识的来源与管理

rig 不从文件加载 prompt/skill，三者都来自代码：

- **系统提示词**：`AgentBuilder::preamble()`（字符串）。当前是 `llm.rs` 里一句简短的
  `SYSTEM_PROMPT`，缺领域知识。
- **工具（"技能"的实质）**：Rust `Tool` 实现（`#[rig_tool]` 或手动 impl），`description()`
  自动进模型 schema，不进文件。
- **上下文知识**：`AgentBuilder::context()`（静态）/ `dynamic_context()`（动态检索）。
- rig 里的 `Skill` 是 Anthropic 容器扩展专属（code-execution container 预加载技能），与
  DeepSeek / 本项目无关。

**本项目"配电网仿真知识"的真实来源是引擎 docs**，尤其 `agent_quickstart.md`
（需求→分析类型→DSA 函数→RSS 脚本的步骤）、`dsa_function_reference.md`（函数签名）、
`dataframe_schema_reference.md`（结果表与单位）、`simulation_recipes.md`（配方）。

**落地方式**：把这些提炼固化到 `backend/prompts/`，用 `include_str!` 编译期嵌入（与
`embed_migrations!` 同理，可移植、无运行时文件依赖），分层注入：

| 层 | 来源 | 内容 |
|---|---|---|
| `preamble` | `prompts/system.md` | 角色定位 + 行为准则 + 精简版 agent_quickstart |
| `context` | `prompts/engine_knowledge.md` | 结果表 schema 摘要 + 单位约定 + 常见配方 |
| 工具说明 | `Tool::description()` | 代码内，不进文件 |

**约束**：引擎 docs 目前不在本仓库（AGENTS.md：内核源码与文档未纳入），不能 `include_str!`
引用 `../eig-rs/...` 外部路径；须**拷一份精简版进 `backend/prompts/`**，随本项目版本管理，
内核更新时手动同步。

---

## 4. 核心机制一：结构化事件流（rig `run_channel` → 前端消息）

rig 0.45 提供了把 agent loop 拆成「驱动 future + 事件 feed」的能力：

- `AgentRunner::run_channel()` 返回 `(Future<PromptResponse>, RunEvents)`，
  `RunEvents: Stream<Item = MultiTurnStreamItem>`；
- `MultiTurnStreamItem` 含：`ToolCall`（工具调用已提交）、`ToolExecutionCommitted`、
  `ToolResult`、`StreamAssistantItem`（模型文本/工具参数流）、`CompletionCall`、
  `FinalResponse`（最终回复）。

**映射**（在 application 层做）：

| MultiTurnStreamItem | 前端消息 | 说明 |
|---|---|---|
| `StreamAssistantItem`（最终文本段） | `assistant` | 可做流式 Markdown |
| `ToolCall` / `ToolExecutionCommitted` | `tool-call`(running) | 工具开始 |
| `ToolResult`（成功） | `tool-call`(success, summary) | 工具结束 |
| `ToolResult`（失败） | `tool-call`(failed) | 工具失败 |
| 工具体内发布的阶段事件 | `progress`(stages) | 引擎运行期间实时推进 |
| 工具体内发布的 result 事件 | `result` | 仿真指标 |

**关键细节**：`ToolCall`/`ToolResult` 是**批量提交后**才成对出现，不是实时 start 通知（rig 文档
明确说明实时观察要靠 tool hook 或工具体内副作用）。因此「`running` 状态 + `progress` 阶段推进」
这类**实时中间态**由工具实例在体内主动发布（见 §5/§6），而不是等 rig 的 ToolResult。

**传输**：`GET /api/sessions/{id}/events` 用 SSE（`text/event-stream`）单向推送，天然契合
「后端产生、前端消费」的模型；事件同时落库，DB 是持久化真相，SSE 只负责实时性（断线后由
`GET /messages` 回放补齐）。

**事件总线**：每 session 一条 `tokio::sync::broadcast`（有界、可丢最旧），挂在 `AppState`
的 session 注册表里；工具与编排层往对应 session 的 channel 发事件，SSE 端点订阅并转成前端消息。

---

## 5. 核心机制二：有状态工具 + 适配器边界

当前 `#[rig_tool]` 生成的是**无状态 unit struct**，无法感知 session/DB/事件总线。rig 0.45 的
`Tool::call(&self, context, args)` 是**异步实例方法**，且官方文档明确「`Arc<Mutex<_>>`、channel
等共享状态属于工具实例，不属于 `ToolContext`」。因此工具写成**有状态实例**：

```rust
// 概念示意，非最终代码
struct RunSimulation {
    session_id: i32,              // 每请求构建时注入，落库/发事件都带它
    services: Arc<AppServices>,   // pool / events / semaphore / engine 适配器
}

impl Tool for RunSimulation {
    type Args = RunSimulationArgs;     // model_path, duration, step, ...
    type Output = String;              // 返回 task_id 给 agent
    type Error = AdapterError;         // 错误码已由适配器翻译

    async fn call(&self, ctx: &mut ToolContext, args: Self::Args) -> Result<String, AdapterError> {
        // 1. 发 tool-call(running) 事件 + 落 messages 表
        // 2. 调 self.services.engine.run(...) —— 只走适配器
        // 3. 期间按引擎阶段发 progress 事件（见 §6）
        // 4. 落 simulation_tasks / task_results / curve_data
        // 5. 发 tool-call(success, summary="task_id: ...") 事件
        Ok(task_id)
    }
}
```

- **每次请求构建一套工具实例**（携带 `session_id` + `Arc<AppServices>`），随 agent 一起创建，
  绕开「agent 如何注入 ToolContext」的不确定点，最稳妥。
- **适配器是唯一内核边界**：工具只调用 `services.engine`（`adapters::SimEngine`）暴露的
  `inspect()` / `run()`；内核地址、鉴权、错误码翻译、子进程参数拼接全部在 `adapters/` 内。
  AGENTS.md 硬约束由此落实。

`inspect_model` 同理：不再是硬编码 `"model ok"`，而是调用 `SimEngine::inspect()`（对应引擎
`import_dss` 的解析步骤，fail-closed + 诊断，见 db-design §0.4）。

---

## 6. 核心机制三：同步引擎 → 异步任务

引擎是同步阻塞 CLI（db-design §0.3「进程级单线程」、§0.5「并发限流」），后端要把它包成
异步、可限流、可追踪的任务：

- **子进程 + 异步外壳**：`adapters::SimEngine` 用 `tokio::process::Command` 起 `tensoreval`，
  外层 `tokio::task::spawn_blocking`（或 `Command::wait`）避免阻塞 async worker。
- **限流**：`AppState` 持 `tokio::sync::Semaphore`（初定 4 并发引擎子进程），工具执行前先
  `acquire`；超过并发排队，对应 `progress` 停在 `pending`。
- **任务状态机**：`model_check → script_gen → running → collecting → (succeeded | failed)`
  （db-design §0.3），映射为前端 `progress.stages`（inspect / sim / analyze / report）。
  阶段由 `SimulationOrchestrator` 在工具体内推进，每推进一档就发一条 `progress` 事件并落
  `simulation_tasks.stage`。
- **结果聚合**：引擎产出 `summary` / `bus_voltage` / `branch_flow` 等命名表，适配器落
  `task_results`（metrics：max/min 电压、越限数、越限时长）+ 曲线（CSV 文件 + `curve_data`
  或 `task_files`，见 db-design §5）。工具据此发 `result` 事件。

---

## 7. 一次请求的完整时序

```
前端                    api                 application          tools            adapters          引擎
 │ POST /messages        │                      │                   │                 │              │
 │──────────────────────>│ 落 user 消息          │                   │                 │              │
 │   <── 202 {session}   │─────────────────────>│ spawn agent run    │                 │              │
 │                       │                      │ build agent+工具   │                 │              │
 │ GET /events (SSE)     │                      │ run_channel() ────>│                 │              │
 │ <── assistant(流式)   │<── FinalResponse 回填 │                   │                 │              │
 │ <── tool-call:inspect │                      │ ToolCall ────────>│                 │              │
 │ <── tool-call(ok)     │                      │ ToolResult ──────>│ 调 engine.inspect│── 子进程 ──>│
 │                       │                      │                   │                 │<── 诊断 ────│
 │ <── tool-call:run(run)│                      │ ToolCall ────────>│ acquire 信号量    │              │
 │ <── progress(inspect) │                      │                   │ 发阶段事件 ──────>│── 子进程 ──>│
 │ <── progress(sim..)   │                      │                   │ (running 期间逐档)│              │
 │ <── progress(done)    │                      │                   │ 落 task/result/curve│<── CSV ────│
 │ <── result(metrics)   │                      │                   │ 发 result 事件     │              │
 │ <── tool-call(ok)     │                      │ ToolResult ──────>│ 返回 task_id       │              │
 │ <── assistant(final)  │                      │ FinalResponse     │                   │              │
 │                       │                      │ 落 assistant 消息  │                   │              │
```

要点：`POST /messages` **尽快返回 ack**（触发后台 agent run），后续一切经 SSE 推送；任何时刻
前端可 `GET /messages` 重放历史补齐断线。

---

## 8. 目录结构（目标形态）

```
backend/src/
├── main.rs            # 入口：装配 AppState、路由、启动
├── api/               # axum handler + DTO + SSE（§3 api 层）
│   ├── mod.rs
│   ├── dto.rs         # 请求/响应/事件 DTO（对齐前端 types）
│   ├── sessions.rs    # 会话/消息 CRUD
│   └── events.rs      # SSE 事件流端点
├── application/       # 编排层（§3 application 层）
│   ├── mod.rs         # AppState / AppServices 定义
│   ├── chat.rs        # ChatService：一次对话的编排
│   ├── events.rs      # EventBus（per-session broadcast）
│   └── simulation.rs  # SimulationOrchestrator：任务状态机
├── agent/             # agent 层（现 llm.rs 的扩展）
│   └── mod.rs         # provider 选择 + preamble + run_channel 驱动
├── tools/             # 工具层（§5）
│   ├── mod.rs         # 工具注册（每请求装配）
│   ├── inspect_model.rs
│   └── run_simulation.rs
├── adapters/          # 适配器层（§5/§6，AGENTS.md 硬约束）
│   ├── mod.rs         # SimEngine trait + 实现 + AdapterError
│   └── engine.rs      # tensoreval 子进程封装 + 错误码翻译
├── db/                # 持久层（已就绪）
│   ├── mod.rs         # pool + 迁移（现有）
│   ├── schema.rs      # 表（现有）
│   ├── models.rs      # 实体（现有）
│   └── repo/          # 新增 repository：SessionRepo/MessageRepo/TaskRepo/ResultRepo
└── llm.rs → agent/mod.rs  # 迁移并扩展（provider 解耦逻辑保留）
```

> `main.rs` 里的两个硬编码工具与 `chat` handler 将被拆分进 `tools/`、`api/`、`application/`。

---

## 9. HTTP API 形态（v1）

> 完整契约（逐端点请求/响应 JSON、6 类消息 schema、SSE 线格式、与前端类型的对齐清单）见
> `docs/api-design.md`，是给前端开发 agent 的单点契约。下表仅作索引。

| 方法 | 路径 | 作用 |
|---|---|---|
| GET | `/api/sessions` | 会话列表 |
| POST | `/api/sessions` | 新建会话 |
| GET | `/api/sessions/{id}/messages` | 回放历史消息（DB 重组为前端 Message 流） |
| POST | `/api/sessions/{id}/messages` | 发用户消息 → 触发 agent run → 返回 202 |
| GET | `/api/sessions/{id}/events` | SSE 订阅事件流 |
| GET | `/api/tasks/{id}/curves` | 曲线数据（前端 CurveData） |
| GET | `/api/tasks/{id}/report` | 报告下载（后续） |

（保留 `/health`。消息/事件 DTO 对齐 `frontend/src/types/index.ts`。）

---

## 10. 关键决策与权衡

| # | 决策 | 理由 | 备选/代价 |
|---|---|---|---|
| 1 | **事件流是轴心**：用 rig `run_channel()` 而非 `agent.prompt()` 一次性 | 前端要 6 类消息的实时渲染 | 复杂度上升，需处理流生命周期 |
| 2 | **有状态工具实例**（`&self` 持 session_id + Arc\<Services\>），每请求构建 | 工具要落库/发事件/带会话上下文；rig 官方定位共享状态归工具实例 | 每请求构建有轻微开销，可接受 |
| 3 | **适配器 = 内核唯一边界**（AGENTS.md 硬约束） | 工具只调 `adapters::SimEngine`，内核地址/鉴权/错误码收敛一处 | — |
| 4 | **SSE 推送 + DB 落库**双通道 | SSE 实时、HTTP 简单单向；DB 是真相，断线可回放 | 需维护两套状态一致性（以 DB 为准） |
| 5 | **Semaphore 限流**（初定 4 并发引擎子进程） | 引擎同步阻塞、CPU 密集（db-design §0.5），多开抢核变慢 | 并发数后续可调/可配置 |
| 6 | **任务状态机后端强加**（model_check→…→succeeded/failed） | 引擎无 task_id/进度概念（db-design §0.3） | 需与引擎真实阶段对齐（实现时验证） |
| 7 | **POST 返回 ack + 后台 agent run** | 仿真可能几十秒，不能让 HTTP 长挂 | 需处理「会话被并发写」与取消 |

---

## 11. 待验证点（实现前必须落地）

1. **`run_channel()` 的具体用法**：确认 `AgentRunner` 如何获得（`Agent` → `AgentRunner` 的转换），
   以及 `RunEvents` 与 axum SSE 的桥接（`Stream` → `EventStream`）。
2. **`#[rig_tool]` vs 手动 `impl Tool`**：确认能否给工具带状态字段；不能则用手动 `impl Tool`
   或 `DynamicTool::new_with_context`（二者 rig 0.45 都支持）。
3. **`ToolCall`/`ToolResult` 的批量提交语义**：确认「实时 running/progress」必须靠工具体内
   主动发事件（而非 rig 的 ToolResult）这条判断，实测验证时序。
4. **SSE 断线重连**：`broadcast` 会丢旧事件，重连后需用 `Last-Event-ID` 或回放端点补齐；
   具体补丁方案实现时定。
5. **引擎阶段对齐**：`model_check/script_gen/running/collecting` 与真实 `tensoreval` 执行阶段
   的映射，需在接入真实引擎后校准（现在引擎只有「一次跑完」这一个外部可见状态）。
6. **并发写一致性**：同一 session 的 agent run 与落库是否要串行化（避免消息乱序）。

---

## 12. 演进路线（建议实现顺序）

1. `adapters/` + `AdapterError`：先把内核边界立起来（即使暂用假实现）—— ✅ 已落地；
2. `application/`（AppState / EventBus）+ `api/`（SSE + 消息回放）—— ✅ 已落地；
3. `tools/` 有状态化 + `agent/` 的 `run_channel` 驱动 —— ✅ 已落地；
4. 接入真实引擎子进程 + 状态机 + 限流 —— ✅ 已落地（`adapters::ProcessEngine`，见下）；
5. 曲线/报告端点的数据聚合 —— ✅ 已落地（`read_curves`/`read_report`，见 development-status §4）。

> 第 4 步落地说明：`ProcessEngine` 由 `TENSOREVAL_BIN` 环境变量选择，未设置时回退 `StubEngine`；
> `TENSOREVAL_CWD` 指定引擎工作目录（`.rss` 内 `import_dss` 的相对路径按此解析，本地指内核仓库根）。
> 任务状态机落 `simulation_tasks`（pending → running → succeeded/failed），限流用 `AppState.semaphore`（4 并发）。
>
> 真实内核 QSTS 契约（实测）：`inspect` 走 `--dss-to-rss --check --study snapshot`；时序仿真由 `.rss`
> 脚本（`import_dss` + `solution` + `load_shape` + `ds_radial_qsts` 请求 `bus_phase,load_state,summary`）
> 驱动，结果 CSV 名 = RSS 变量名（`qsts_summary.csv` 等），`summary` 每时步一行、适配器按最坏情形聚合。
> 详见 `docs/development-status.md` §4。
