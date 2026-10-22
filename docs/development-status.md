# 开发状态与设计速览

> 面向新开的 agent：读这份可快速了解「已完成 / 待做 / 关键决策」，避免重复调研。
> 详细的机制与契约请看配套文档：`docs/architecture.md`（分层与核心机制）、
> `docs/api-design.md`（前后端契约，前端开发 agent 的单点契约）、
> `docs/db-design.md`（数据库设计）、
> `docs/frontend-design.md`（前端功能设计，后端 agent 对接必读）、
> `docs/model-workspace-design.md`（上传、版本、自然语言仿真、历史与对比的目标设计）、
> `docs/agent-prompt-skills-design.md`（系统 prompt、领域技能、引擎知识来源与版本管理）、
> `docs/superpowers/plans/2026-10-22-v2-simulation-workspace.md`（v2 可执行实施方案）。

---

## 1. 项目定位

配电网仿真智能体：Rust 后端（rig 框架实现 LLM agent，工具封装仿真内核能力）+ React/TS
前端（对话式仿真工作台，左侧历史栏参考 DeepSeek 一类产品，ECharts 绘曲线）。前端通过 Vite 把 `/api` 代理到后端
`http://127.0.0.1:8080`。

---

## 2. 当前状态总览（截至 2026-10-22）

| 模块 | 状态 | 说明 |
|---|---|---|
| 前端静态骨架 | ✅ v2 协议已接入 | 8 类消息、按 operation/callId 归并、可恢复 SSE；工作区和通用结果组件待后续任务 |
| 前端接入真实后端 | ✅ 阶段 1–5 完成 | `api/client.ts`、`api/events.ts`、`App.tsx` 已接线，含重命名/删除/附件/曲线/报告（见 §5） |
| 后端分层架构 | ✅ 第 1–4 步完成 | adapters/application/tools/agent/api 已落地，SSE 流式可跑通 |
| 后端数据库 schema | 🚧 v2 实施中 | 已增加 storage scope 和全链路归属过滤；artifact/workspace/dataset 等表按后续任务增加 |
| 后端真实引擎接入 | ✅ 第 4 步完成 | `adapters::ProcessEngine` 起 `tensoreval` 子进程（`TENSOREVAL_BIN` 选择，未设置回退 stub）+ 任务状态机 + 限流信号量 |
| 后端曲线/报告端点 | ✅ 第 5 步完成 | `GET /api/tasks/{id}/curves`、`/report` 已实现；metrics 真实值（尽力解析） |
| 会话状态/重命名/删除/附件 | ✅ 完成 | `PATCH/DELETE /api/sessions/{id}`、`POST .../attachments`，状态按最新任务推导 |
| 前后端字段命名对齐 | ✅ 已对齐 | 后端出网 `tool-call`/`toolName`/`taskId` 与前端类型一致（§3） |
| 历史聊天浏览 | ✅ 基础功能完成 | 全量会话列表、今天/昨天/更早分组、打开回放、重命名和删除 |
| 历史聊天检索 | 🎯 已完成设计 | 侧栏关键词/筛选/分页 + agent 自然语言跨会话检索；尚未实现 |
| 模型工作区与不可变版本 | 🎯 已完成设计 | 支持 RSS、ZIP/`.rssbundle` 目录、分批 DataFrame；尚未实现 |
| 表头推断与多工作表 Excel | 🎯 已完成设计 | 按内核 schema catalog 的列名推断，歧义由用户确认；尚未实现 |
| 参数修改、任务历史与仿真对比 | 🎯 已完成设计 | 结构化补丁、版本绑定、稳定键对比；尚未实现 |
| 通用结果 DataFrame 分析 | 🎯 已完成设计 | v2 用 `--result-dir --output-format parquet` 保存原生 Parquet；自然语言生成 RSS 查询脚本并持久化派生表、schema 与血缘；尚未实现 |
| Agent 调度与容量保护 | ✅ 完成 | 全局 8、owner 2、同 session 1、等待队列 32；满载返回 `429 agent_queue_full` 且不写消息 |
| 历史文件配额 | 🎯 已完成设计 | 默认每实例/owner 20,000 个唯一 artifact、20 GiB，80% 告警，不按时间自动删除；尚未实现 |
| 多用户隔离 | ✅ 基础边界完成 | instance/trusted_proxy、`/api/me`、storage/access scope 和 Session/Message/Task/SSE 404 隔离已落地 |
| Agent prompt 与领域技能 | 🎯 已完成设计 | 当前只有一句 system prompt；v2 启动时预索引 TENSOREVAL_CWD 文档，再以版本化 system、按需技能、机器 catalog 和工具合同分层使用，尚未实现 |
| 内置案例库 | 🎯 已完成设计 | 当前目录已确认 16 个根 RSS、37 个已发布 rssbundle；v2 支持自然语言搜索、导入、派生修改和仿真，尚未实现 |
| v2 详细实施方案 | 🚧 执行中 | Tasks 1–4 已完成；M0 平台基础完成，继续实施 artifact、工作区和摄取 |

> v2 实施以 `docs/superpowers/plans/2026-10-22-v2-simulation-workspace.md` 的 Task 编号为准；当前
> 已完成 Task 1–4。后文保留的 v1 细节是迁移背景，若与本节冲突，以本节和 v2 契约为准。
> 上表 `🎯` 表示已评审目标，不能理解为代码已具备该能力；目标实施顺序见
> `docs/model-workspace-design.md` §11。

### 2.1 v2 身份与恢复语义

当前代码已实现 `AUTH_MODE=instance|trusted_proxy`。公开部署必须使用可信代理；所有现有
Session/Message/Task/SSE 读写均由服务端 principal 注入 scope，跨 scope 和不存在资源返回相同 404。
聊天仍持久化在 SQLite；前端已使用“先打开 SSE、再读快照”的顺序恢复消息，并按稳定键去重。
`GET /api/me` 已提供 `scopeKey`，按 scope 保存上次 session 的本地导航偏好将在前端工作台任务完成。

以下条目仅记录 v1 被替换前的风险背景：

- 当前后端**没有用户身份**。没有登录态、认证 token、用户 cookie、`user_id` 或 owner/tenant
  过滤；连接同一个后端实例的所有浏览器共享同一套 SQLite 会话和消息。
- `session_id` 只表示一段对话，不是用户身份或权限边界。两个使用者可以分别操作不同 session，
  但双方都能通过会话列表看到、打开、重命名和删除全部 session；如果同时进入同一个 session，消息和
  SSE 事件也会混在同一条对话中。因此当前版本不能直接供互不信任的多个用户共同使用。
- 聊天记录不是只存在浏览器内存。消息已写入 SQLite；刷新页面后，前端重新请求
  `GET /api/sessions` 和 `GET /api/sessions/{id}/messages`，只要后端仍使用同一个持久化
  `DATABASE_URL`，历史聊天就能找回。后端进程重启本身不会清空已提交的聊天记录。
- 当前前端没有把 `selectedId` 写入 `localStorage` 或其他持久存储。刷新时它会打开按
  `updatedAt` 降序返回的第一条会话，而不保证回到刷新前正在查看的会话；用户仍可从左侧历史栏切回。
  其他使用者若刚更新了会话，刷新后的默认会话也可能随之变化。
- 删除或更换数据库文件、改变相对 `DATABASE_URL` 的启动目录，或把数据库放在容器临时盘上，都会让
  原历史不可见。生产部署必须把 SQLite 文件和 `DATA_DIR` 放在持久卷中；数据库记录是聊天历史的
  真相来源，`DATA_DIR` 只保存上传文件和结果文件。
- 浏览器自行生成并保存在 `localStorage` 的随机 ID 最多只能用于恢复界面偏好，不能防止伪造，不能
  作为多用户隔离方案。在正式身份系统落地前，如果确实需要多个独立使用者，只能为每个使用者或可信
  租户部署独立的后端实例、数据库和 `DATA_DIR`。
- 目标实现是在 API 层认证并解析不可变 owner/tenant，随后对 session、message、task、artifact、
  SSE 和历史检索的每一次读写强制带归属范围；访问其他主体的资源统一返回 404。身份系统落地后，
  前端可以按当前主体保存“上次选中的 session”作为导航偏好，但历史恢复仍以服务端数据库为准。

---

## 3. 前后端契约（务必先读 api-design.md）

- **前端类型是权威**：`frontend/src/types/index.ts` 定义 8 类 v2 discriminated union，新增
  `model-status/comparison`；progress 使用 operation，tool-call 使用稳定 callId，result 引用 outputs。
- **通信模型**：`POST /messages` admission 成功后立即回 202 → 后台 agent 串行运行 → 持久事件经
  带 ID 的 SSE 推送 → 断线按 `Last-Event-ID` 补发，必要时触发快照重同步。
- **已实现端点**：`GET/POST /api/sessions`、`PATCH/DELETE /api/sessions/{id}`、
  `GET/POST /api/sessions/{id}/messages`、`POST /api/sessions/{id}/attachments`、
  `GET /api/sessions/{id}/events`（SSE）、`GET /api/tasks/{id}/curves`、`/report`、`GET /api/me`、`GET /health`。
- **错误约定**：固定 `{error,code,details?}`；跨 scope 资源统一 404；队列满为
  `429 agent_queue_full`。

### 字段命名已对齐（api-design §8）

后端 `application/events.rs` 已按前端权威 `types/index.ts` 对齐：`tool-call`（`ToolCall`
变体 `rename = "tool-call"`）、`toolName`/`taskId`（变体级 `rename_all = "camelCase"`），
并由 `wire_format_matches_frontend_types` 单测锁定。前端零转换直接消费，`tool-call`/`progress`/
`result` 三类消息联调不再渲染不匹配。

---

## 4. 后端分层（架构速览）

```
api（axum handler/DTO/SSE）
  → application（AppState / EventBus / ChatService 编排）
    → agent（provider 选择 + run_channel 流式驱动）
    → tools（有状态工具，只调 adapters + repo）
      → adapters（SimEngine trait，内核唯一边界）
      → db（Diesel pool + repository）
```

- **硬约束（AGENTS.md）**：内核调用必须走 `adapters/`，工具层不得直接调内核 API。
- **事件流轴心**：用 rig `run_channel()` 拆出「驱动 future + 事件 feed」，`MultiTurnStreamItem`
  映射为前端消息；`StreamAssistantItem::End` 写完整 assistant 文本，工具体内主动发布仿真
  progress/result，`ToolResult` 再产生 tool-call 终态（见 `application/chat.rs`）。
- **有状态工具**：每请求构建一套（携带 session_id + `Arc<ToolServices>`），绕开 ToolContext 注入。
- **Agent 生命周期**：不是每用户常驻一个 agent；每条用户消息由 `tokio::spawn` 启动一次短生命周期
  run，run 内重建 agent/工具并从 DB 加载 session 历史，完成后释放。rig 本身没有设置 agent 数量上限。
- **当前容量边界**：单次 run 最多 8 个模型轮次；`RunScheduler` 强制同 session 1、owner 2、实例 8
  个活动 run，等待队列默认 32。仿真内核另有独立 `Semaphore(4)`；两层限制职责不同。
- **引擎选择**：`adapters/process.rs` 的 `ProcessEngine` 起 `tensoreval` 子进程（`TENSOREVAL_BIN`
  选择，未设置回退 `adapters/stub.rs` 的 `StubEngine`）；错误码翻译到 `ErrorCategory`。

### 后端目录（实际）

```
backend/src/
├── main.rs            # 装配 AppState、迁移、路由、引擎选择（env）
├── adapters/          # mod.rs（SimEngine + AdapterError + engine_from_env）、process.rs、stub.rs
├── agent/mod.rs       # provider 选择 + build_agent
├── api/               # mod.rs（router）、sessions.rs、events.rs（SSE）
├── application/       # AppState、chat、events（EventBus/AppEvent）、scheduler
├── db/                # pool/迁移/schema/models/repositories（全部显式接收 scope）
└── tools/             # mod.rs（build_tools）、inspect_model.rs、run_simulation.rs
```

### 后端已落地（第 4–5 步，详见 `docs/architecture.md` §12）

第 4 步（真实引擎子进程 + 任务状态机 + 限流）与第 5 步（补齐 6 项缺口）均已落地：

- **真实引擎子进程**：`adapters/process.rs` 的 `ProcessEngine` 起 `tensoreval`（`--df --result-dir`），
  `TENSOREVAL_BIN` 未设置时回退 `StubEngine`；错误码翻译到 `ErrorCategory`。
- **真实内核 QSTS 契约（2026-10-22 实测对齐）**：
  - `inspect` → `tensoreval --dss-to-rss --check --study snapshot <model.dss>`（静态潮流校验）。
  - 时序仿真由 `.rss` 脚本驱动：`import_dss` + `solution`(DAILY) + `load_shape` + `ds_radial_qsts`
    请求 `bus_phase,load_state,summary`（示例 `eig-rs/sparrowzz/dss/ieee13_daily_qsts.rss`）。
  - 结果 CSV 名 = RSS 变量名（`qsts_summary.csv`/`qsts_voltage.csv`/`qsts_load_state.csv`），非固定
    `summary.csv`；适配器按后缀/列名容错识别。
  - `summary` 为**每时步一行**，适配器按最坏情形聚合：`converged` AND、损耗/越限取最大值、
    `violation_duration = 越限步数 × 步长`。
  - 电压：`min_voltage` 取 `load_state.v_min_pu`（标幺，精确）；`max_voltage` 由 `voltage.v_mag`（单位
    V）按源母线归一（单电压等级精确、多电压等级近似）。
  - 环境变量新增 `TENSOREVAL_CWD`：`.rss` 内 `import_dss` 相对路径按此目录解析（本地指内核仓库根）。
- **任务状态机**：`TaskRepo` 落 `simulation_tasks`（`pending→running→succeeded/failed`、阶段
  `model_check→script_gen→running→collecting`），`task_id` = `simulation_tasks.id`。
- **限流接入**：`AppState.semaphore`（=4）沿 `ChatService → ToolServices → RunSimulationTool`
  传下，执行前 acquire。
- **会话状态动态化**：`GET /api/sessions` 由 `TaskRepo::latest_statuses()` 一次批量查询推导
  `status`（`succeeded→completed`、`pending/running→running`、`failed/cancelled→failed`）。
- **v1 固定 metrics（遗留）**：`RunMetrics` 当前仍从 QSTS CSV 尽力聚合
  `max_voltage/min_voltage/violation_duration_s` 供现有前端使用。这不是 v2 结果模型；v2 保存完整
  DataFrame 文件，用户查询时由 RSS 脚本生成派生表。
- **曲线/报告**：`read_curves`/`read_report` 从 `data_dir/results/{task_id}` 与 `reports/{task_id}.md`
  按需读取（曲线解析为启发式、失败软回退），`/curves`/`/report` 端点已实现。
- **重命名/删除/附件**：`PATCH/DELETE /api/sessions/{id}`（删除时 `EventBus::unregister`）、
  `POST /api/sessions/{id}/attachments`（multipart 上传 → 回发 `attachment` 消息）。
- **隐患修复**：`messages.task_id` 随 progress/result 落库；`EventBus` 注销防泄漏；404 由
  `RepoError::NotFound` 映射；`tool-call.args` 收口归一为对象（永不为 null）。
- **端到端已验证（2026-10-22）**：真实 `tensoreval` 跑 `ieee13_daily_qsts.rss` 全链路通——`inspect`
  校验 194 条语句、`run_simulation` 产出 `qsts_*` CSV、metrics/curves/report 均正确（单测 13 个全过）。
  **环境已核对（2026-10-22）**：`.env` 的 `TENSOREVAL_BIN` 与 `TENSOREVAL_CWD` 均已指向更名后的
  `eig-rs`。**已知待办（后端）**：① agent 系统提示词不含模型/脚本路径，当前需消息显式给绝对路径，
  v2 将改为只传版本 ID；② v1 电压曲线按 16 母线×相位展开成 41 条序列且图例拥挤，v2 改用受控
  多视图、数据规模预算和自然语言筛选，不在后端硬编码代表母线；③ `maxVoltage` 走源母线归一近似（结果目录无 `*_bus.csv` 的 `voltage_base_v`），只用于当前原型
  显示；④ 输出文件尚未登记为 `artifacts/result_datasets`，自然语言结果分析运行与血缘也未实现；⑤ agent run 尚无
  实例级上限或有界队列，同一会话也未串行化；⑥ 任务取消尚未形成完整契约；⑦ 当前 QSTS 任务的
  `study_type` 仍写 NULL；v2 新 schema 必须改为规范的非空值。

- **v2 Parquet 合同（源码已确认、运行时待接入）**：eig-rs 的 tensoreval 已增加
  `--output-format <csv|parquet>`，默认 CSV，`parquet` 输出 Snappy 压缩的 `NAME.parquet`。当前
  `.env` 指向的已编译二进制帮助信息尚未出现该参数，启用 v2 前必须重编译/替换并在适配器启动探测；
  v2 同步删除原型 CSV 任务读取和 `/curves` 端点。tensoreval 本身已经能通过 `--input NAME=FILE.parquet` 读取 Parquet；
  当前缺少的是 **zjudss 结果服务层** 的 Parquet reader：`backend/src/adapters/process.rs` 仍只枚举、
  `read_to_string` 并解析 CSV，`Cargo.toml` 也没有直接声明结果读取依赖。v2 必须补齐结果登记所需的
  schema/行数校验，以及 REST 表格分页、列投影和图表 JSON 读取；不能依赖 rig 间接带入的
  Arrow/DataFusion 包。

---

## 5. 前端实现要点

### 目录（实际）

```
frontend/src/
├── main.tsx            # ReactDOM + StrictMode
├── App.tsx             # 全部状态与接线（见下）
├── types/index.ts      # ★ 权威类型定义（Session / Message 6 类 / CurveData / 回调）
├── api/                # 新增：后端接入层
│   ├── client.ts       #   listSessions/createSession/renameSession/deleteSession/listMessages/postMessage/uploadAttachment/fetchCurves/fetchReport + ApiError
│   └── events.ts       #   subscribeEvents(sessionId, onMessage) → SSE，返回关闭函数
├── hooks/useMediaQuery.ts
├── mocks/              # sessions.ts/messages.ts/curves.ts 已无引用（保留作样例）
├── components/
│   ├── Sidebar.tsx / Composer.tsx / ResultPanel.tsx / Markdown.tsx / icons.tsx
│   ├── charts/LineChart.tsx   # ECharts 直连，props 传 CurveData，支持 dataZoom/legend/resize
│   └── messages/              # MessageItem + 6 类消息卡片
```

### App.tsx 接线逻辑（本会话阶段 1–3 的产物）

- **会话**：挂载 `listSessions()` 选第一条；`newSession` 走 `createSession('新的仿真')`；
  `selectSession` 先 `listMessages(id)` 回放。
- **发送 + SSE**：`send()` 乐观插入 `local-user-` 临时消息 → `postMessage()`（202）→ SSE 收到
  真实 `user` 事件后按 id 前缀 FIFO 替换（**方案 A**，见 §6 决策）。
- **事件合并** `appendEvent()`（`App.tsx` 模块级函数）：
  - `user` 事件 → 替换第一条 `local-user-`；
  - `progress` → 按 `taskId` 归并（替换而非追加，api-design §5.4）；
  - 其它 → 按 `id` 去重后追加。
- **SSE 生命周期**：`openEvents(id)` 在回放完成后调用；切换会话 / 卸载时 `closeEvents()` 关闭。
  用 `selectedIdRef` 避免异步回放竞态（过期结果丢弃）。

### 前端阶段 4–5 接入（已完成后端第 5 步的接线）

- **重命名/删除**：`PATCH/DELETE /api/sessions/{id}` 真持久化；`App.tsx` 乐观更新、失败回滚
  （重命名回滚到旧列表，删除失败重拉会话列表）。
- **附件上传**：`Composer` 纸夹 → 真实文件选择 → `POST .../attachments`（multipart 字段 `file`），
  201 返回 attachment 消息经 `appendEvent` 落地；SSE 广播同一条按 id 去重。
- **曲线/报告**：`ResultCard` 的「查看曲线」/「下载报告」按 `taskId` 调 `GET /api/tasks/{id}/curves`、
  `/report`；`ResultPanel` 曲线/报告标签页按需拉取（报告 Markdown 内联渲染 + 下载）。日志标签页
  暂无后端端点，保留静态样例。当前图表仍固定为共享时间轴的 `LineChart`，没有通用分页表格或
  多视图渲染器；目标契约见 §8 和 `frontend-design.md` §7.8。
- **指标显示四舍五入**：`ResultCard` 对电压等标幺指标按 3 位小数展示（`fmtPu`），真实内核返回的
  长浮点（如 `0.881936848419229`）不再直接上屏。

---

## 6. 关键决策记录

| # | 决策 | 理由 |
|---|---|---|
| 1 | 前端类型 `types/index.ts` 是前后端契约的权威 | 后端按 api-design §8 对齐，前端零转换 |
| 2 | 乐观 user 消息「本地临时 id + SSE 回传后替换」（方案 A） | 即时反馈；按 `local-user-` 前缀 FIFO 匹配，与内容无关，两条相同消息不误删；失败可干净回滚 |
| 3 | v1 `progress` 按 `taskId` 归并；v2 改为 `operation.kind + operation.id` | 摄取与仿真 ID 可能相同，v2 必须分命名空间 |
| 4 | v1 先 GET 再开 SSE 是已知丢事件窗口；v2 改为 SSE open 后读快照，并按 `Last-Event-ID` 补发 | 同时覆盖初次进入与断线恢复 |
| 5 | 内核调用只走 `adapters/` | AGENTS.md 硬约束 |
| 6 | 结果 DataFrame 存文件、DB 只存目录/schema/血缘元数据 | db-design §5.2 容量估算结论 |
| 7 | 同步引擎用 `tokio::process::Command` 异步等待子进程，并用信号量限流 | 引擎计算在子进程内，不阻塞 Tokio worker |
| 8 | Agent 按每条消息短生命周期构建；v2 同会话串行、实例级活动 run 默认上限 8，并设置有界队列 | 会话连续性由 DB 保持；防止突发请求耗尽 LLM 配额和应用资源 |
| 9 | DataFrame 始终有分页表格入口，并可拥有多个受控语义图表视图 | Agent 生成 `VisualizationSpec`，前端注册表编译为 ECharts；API 不接收任意 ECharts option |
| 10 | v2 直接删除原型 CSV 结果链路，`result_datasets` 统一使用原生 Snappy Parquet | v1 未投入使用；适配器显式传 `--output-format parquet`，不保留双读或转换分支 |
| 11 | 历史文件默认每个归属主体 20,000 个唯一 artifact、20 GiB，80% 告警且不自动过期 | v2 instance 模式按实例、trusted_proxy 模式按 owner/tenant；删除会话并完成 GC 后释放额度 |
| 12 | v2 不兼容未投入使用的 v1 原型数据和接口 | 发布前重建干净 v2 schema，删除旧消息、固定 metrics、`task_results/curve_data`、`/curves` 和 `/report` 分支 |

---

## 7. 运行与验证

```bash
# 后端（另由后端 agent 维护）
cd backend && cargo run          # http://127.0.0.1:8080

# 前端
cd frontend && npm install && npm run dev   # http://localhost:5173，/api 代理到 8080
npm run build                              # 校验 TS 无错
```

端到端验证路径：新建会话 → 发一条消息 → 观察 `tool-call → progress → result → assistant`
流式渲染；刷新后经 `GET /messages` 回放复原，progress 不重复叠加。**前提**：后端已按 §3 完成
字段命名对齐。

---

## 8. 下一阶段：模型工作区与历史检索闭环

已确认的产品语义：

- 一个会话对应一个模型工作区；
- 工作区区分 active 与 draft：默认仿真使用 active，错传或未确认的 draft 不阻塞已有模型；
- 单 `.rss`、ZIP、目录形式的 `.rssbundle` 和多次追加的 CSV/Excel/Parquet 都是合法输入；
- 服务启动时从 `TENSOREVAL_CWD/sparrowzz/dss/docs` 预检、切分并索引文档；agent 按需读取相关 section，
  不把全部文档常驻每轮 prompt；
- `TENSOREVAL_CWD/sparrowzz/dss` 下经过验证的根 RSS 和 rssbundle 形成只读案例库；用户可用自然语言
  搜索并导入，导入后复制到自己的 artifact/版本链，再继续修改参数和仿真；
- DataFrame 类型依据表头（列名）推断，文件名与工作表名不决定类型；dtype 只做校验；
- 模糊、未知或重复类型必须由智能体向用户说明并确认；
- 每次上传、确认或修改产生不可变版本，任务绑定具体版本和完整 `study_spec`；
- 文件传错时可重新上传并明确替换、移除或回退；不覆盖历史版本及其任务；
- 每个上传批次和文件分别使用 `clientBatchId/clientFileId`，支持乱序完成、局部失败和幂等重试；
- 用户通过自然语言发起潮流、短路、QSTS、结果查询、模型修改和前后对比；
- 历史聊天支持侧栏搜索/筛选/分页和自然语言跨会话查询，并能定位到命中的具体消息；
- “上次”“修改前”先在当前会话解析，跨会话有多个候选时必须让用户确认；
- 固定控件仅保留会话、上传/重试和只读浏览等高频低歧义操作，领域快捷项仍发送可见自然语言消息；
- 仿真类型保持开放；v2 适配器固定使用 `--result-dir ... --output-format parquet`，每张命名
  DataFrame 以原生 Parquet 保存为结果数据集，不预设统计指标；
- 用户查询结果时，智能体读取数据集 schema；需要筛选、聚合、透视或统计计算时生成 RSS 脚本，
  通过受控 `--input` 和独立 `--result-dir` 产出新的 DataFrame，再以分页表格或多个图表视图展示；
- 每张 DataFrame 始终有分页表格入口，并可关联多个 `dataset_views`；纯换图只追加受控视图，无需运行
  tensoreval，已有视图用固定标签快速切换；
- 首批图表支持折线、柱状、散点、热力、箱线和直方图。智能体只生成版本化 `VisualizationSpec`，
  前端渲染器注册表负责编译 ECharts option，禁止模型生成任意 option 或可执行 formatter；
- 每次结果分析保存用户意图、脚本、输入 dataset/alias、输出 dataset 和初始视图；模型不接触服务器路径；
- v1 未投入使用，不迁移旧 SQLite、CSV 或 DATA_DIR；v2 发布前重建干净 schema，并只保存原生 Parquet；
- 比较绑定两个历史任务并生成 RSS 比较脚本，按稳定业务键和单位输出差值 DataFrame，禁止按行号比较；
- 聊天、版本、任务、结果和比较均可在重新进入会话后恢复。
- 历史文件配额同时限制唯一 artifact 数和逻辑字节；达到硬限制只阻止新增文件，历史读取继续可用，
  首版不按保存天数或“最近 N 个任务”静默删除。

详细实施按六个里程碑推进：平台基础 → 模型工作区 → 通用仿真结果 → 历史与对比 → 前端工作台 →
迁移发布。身份 scope、agent admission 和 SSE 恢复先于工作区实现，避免后续仓储和事件协议返工。
详细对象、API、错误码和验收路径见 `docs/model-workspace-design.md`；数据库迁移见
`docs/db-design.md` §7；目标 HTTP/SSE 契约见 `docs/api-design.md` §10；前端待办见
`docs/frontend-design.md` §7；逐文件、逐测试和逐提交步骤见
`docs/superpowers/plans/2026-10-22-v2-simulation-workspace.md`。
