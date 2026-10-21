# 开发状态与设计速览

> 面向新开的 agent：读这份可快速了解「已完成 / 待做 / 关键决策」，避免重复调研。
> 详细的机制与契约请看配套文档：`docs/architecture.md`（分层与核心机制）、
> `docs/api-design.md`（前后端契约，前端开发 agent 的单点契约）、
> `docs/db-design.md`（数据库设计）、
> `docs/frontend-design.md`（前端功能设计，后端 agent 对接必读）。

---

## 1. 项目定位

配电网仿真智能体：Rust 后端（rig 框架实现 LLM agent，工具封装仿真内核能力）+ React/TS
前端（DeepSeek 风格仿真工作台，ECharts 绘曲线）。前端通过 Vite 把 `/api` 代理到后端
`http://127.0.0.1:8080`。

---

## 2. 当前状态总览（截至 2026-10-21）

| 模块 | 状态 | 说明 |
|---|---|---|
| 前端静态骨架 | ✅ 完成 | 6 类消息组件、三栏布局、ECharts 曲线、侧栏分组/重命名/删除、结果面板 |
| 前端接入真实后端 | ✅ 阶段 1–3 完成 | `api/client.ts`、`api/events.ts`、`App.tsx` 已接线（见 §5） |
| 后端分层架构 | ✅ 第 1–4 步完成 | adapters/application/tools/agent/api 已落地，SSE 流式可跑通 |
| 后端数据库 | ✅ 完成 | 6 张表 + models + 仓储 `find/rename/delete/latest_status` 已齐 |
| 后端真实引擎接入 | ✅ 第 4 步完成 | `adapters::ProcessEngine` 起 `tensoreval` 子进程（`TENSOREVAL_BIN` 选择，未设置回退 stub）+ 任务状态机 + 限流信号量 |
| 后端曲线/报告端点 | ✅ 第 5 步完成 | `GET /api/tasks/{id}/curves`、`/report` 已实现；metrics 真实值（尽力解析） |
| 会话状态/重命名/删除/附件 | ✅ 完成 | `PATCH/DELETE /api/sessions/{id}`、`POST .../attachments`，状态按最新任务推导 |
| 前后端字段命名对齐 | ✅ 已对齐 | 后端出网 `tool-call`/`toolName`/`taskId` 与前端类型一致（§3） |

> 后端「第 1–5 步」指 `docs/architecture.md` §12 的演进路线。第 5 步已于 2026-10-21 落地。

---

## 3. 前后端契约（务必先读 api-design.md）

- **前端类型是权威**：`frontend/src/types/index.ts` 定义 6 类 discriminated union
  （`user`/`assistant`/`tool-call`/`progress`/`result`/`attachment`）与 `Session`、`CurveData`。
  后端事件必须能映射到这 6 类，字段命名以前端为准。
- **通信模型**：`POST /messages` 立即回 202 → 后台 agent 跑 → 事件经 SSE 逐条推送 → 断线由
  `GET /messages` 回放补齐。
- **已实现端点**：`GET/POST /api/sessions`、`PATCH/DELETE /api/sessions/{id}`、
  `GET/POST /api/sessions/{id}/messages`、`POST /api/sessions/{id}/attachments`、
  `GET /api/sessions/{id}/events`（SSE）、`GET /api/tasks/{id}/curves`、`/report`、`GET /health`。
- **错误约定**：未知会话/任务/无曲线报告 → `404 {"error":…}`；仓储/适配器错误 → `500`。

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
  映射为前端消息；`ToolCall`/`ToolResult` 是批量提交后成对出现，因此「running 状态 + progress
  阶段推进」这类实时中间态由工具实例在体内主动发事件（见 `application/chat.rs`）。
- **有状态工具**：每请求构建一套（携带 session_id + `Arc<ToolServices>`），绕开 ToolContext 注入。
- **引擎选择**：`adapters/process.rs` 的 `ProcessEngine` 起 `tensoreval` 子进程（`TENSOREVAL_BIN`
  选择，未设置回退 `adapters/stub.rs` 的 `StubEngine`）；错误码翻译到 `ErrorCategory`。

### 后端目录（实际）

```
backend/src/
├── main.rs            # 装配 AppState、迁移、路由、引擎选择（env）
├── adapters/          # mod.rs（SimEngine + AdapterError + engine_from_env）、process.rs、stub.rs
├── agent/mod.rs       # provider 选择 + build_agent
├── api/               # mod.rs（router）、sessions.rs、events.rs（SSE）
├── application/       # mod.rs（AppState）、chat.rs、events.rs（EventBus/AppEvent）
├── db/                # mod.rs（pool/PRAGMA/迁移）、schema.rs、models.rs、repo.rs（含 TaskRepo）
└── tools/             # mod.rs（build_tools）、inspect_model.rs、run_simulation.rs
```

### 后端已落地（第 4–5 步，详见 `docs/architecture.md` §12）

第 4 步（真实引擎子进程 + 任务状态机 + 限流）与第 5 步（补齐 6 项缺口）均已落地：

- **真实引擎子进程**：`adapters/process.rs` 的 `ProcessEngine` 起 `tensoreval`（`--df --result-dir`），
  `TENSOREVAL_BIN` 未设置时回退 `StubEngine`；错误码翻译到 `ErrorCategory`。
- **任务状态机**：`TaskRepo` 落 `simulation_tasks`（`pending→running→succeeded/failed`、阶段
  `model_check→script_gen→running→collecting`），`task_id` = `simulation_tasks.id`。
- **限流接入**：`AppState.semaphore`（=4）沿 `ChatService → ToolServices → RunSimulationTool`
  传下，执行前 acquire。
- **会话状态动态化**：`GET /api/sessions` 由 `TaskRepo::latest_statuses()` 一次批量查询推导
  `status`（`succeeded→completed`、`pending/running→running`、`failed/cancelled→failed`）。
- **结果 metrics 真实值**：`RunMetrics` 增补 `max_voltage/min_voltage/violation_duration_s`（可选），
  `ProcessEngine` 尽力解析 summary/曲线 CSV、`StubEngine` 返回非零样本；工具层 `format_duration` 转展示文本。
- **曲线/报告**：`read_curves`/`read_report` 从 `data_dir/results/{task_id}` 与 `reports/{task_id}.md`
  按需读取（曲线解析为启发式、失败软回退），`/curves`/`/report` 端点已实现。
- **重命名/删除/附件**：`PATCH/DELETE /api/sessions/{id}`（删除时 `EventBus::unregister`）、
  `POST /api/sessions/{id}/attachments`（multipart 上传 → 回发 `attachment` 消息）。
- **隐患修复**：`messages.task_id` 随 progress/result 落库；`EventBus` 注销防泄漏；404 由
  `RepoError::NotFound` 映射；`tool-call.args` 收口归一为对象（永不为 null）。

---

## 5. 前端实现要点

### 目录（实际）

```
frontend/src/
├── main.tsx            # ReactDOM + StrictMode
├── App.tsx             # 全部状态与接线（见下）
├── types/index.ts      # ★ 权威类型定义（Session / Message 6 类 / CurveData / 回调）
├── api/                # 新增：后端接入层
│   ├── client.ts       #   listSessions/createSession/listMessages/postMessage + ApiError
│   └── events.ts       #   subscribeEvents(sessionId, onMessage) → SSE，返回关闭函数
├── hooks/useMediaQuery.ts
├── mocks/              # sessions.ts/messages.ts 已无引用（保留作 6 类消息样例）；curves.ts 仍在用
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

### 前端阶段 4 缺口（本地兜底，未碰后端）

- **重命名/删除**：后端无 `PATCH/DELETE /api/sessions/{id}`，现为**本地乐观**（刷新失效），
  `App.tsx` 内已注释标注。
- **附件上传**：无上传端点，`Composer` 附件芯片仍 mock 本地展示，`send()` 仅文本触发 POST。
- **曲线/报告**：`ResultPanel` 曲线暂用 `mocks/curves.ts`；`ResultCard` 的真实 `taskId` 留作
  后续 fetch 接入点。

---

## 6. 关键决策记录

| # | 决策 | 理由 |
|---|---|---|
| 1 | 前端类型 `types/index.ts` 是前后端契约的权威 | 后端按 api-design §8 对齐，前端零转换 |
| 2 | 乐观 user 消息「本地临时 id + SSE 回传后替换」（方案 A） | 即时反馈；按 `local-user-` 前缀 FIFO 匹配，与内容无关，两条相同消息不误删；失败可干净回滚 |
| 3 | `progress` 按 `taskId` 归并替换 | 后端逐档推进会发多条 progress，前端应替换而非追加 |
| 4 | 回放先于 SSE（`selectSession` 先 GET 再开 EventSource） | SSE 不回放历史、无 `Last-Event-ID`，避免 §6 竞态 |
| 5 | 内核调用只走 `adapters/` | AGENTS.md 硬约束 |
| 6 | 曲线存文件、DB 只存元数据 | db-design §5.2 容量估算结论 |
| 7 | 同步引擎用子进程 + `spawn_blocking` + 信号量限流包成异步任务 | 引擎是同步阻塞 CLI |

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
