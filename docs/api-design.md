# HTTP API 设计（前端开发契约）

> 版本：v1 · 状态：已实现（第 4–5 步），字段命名已与前端对齐
> 配套：架构见 `docs/architecture.md`（尤其 §9）、数据库见 `docs/db-design.md`、
> 前端类型权威定义见 `frontend/src/types/index.ts`
> 本文是**给前端开发 agent 的单点契约**：按本文实现，无需阅读 Rust 代码即可对接。
> 反向视角（前端如何消费、后端还需补齐什么）见 `docs/frontend-design.md`。
> 已实现端点均与当前代码一致（含一次真实 DeepSeek 冒烟测试的 SSE 实况）；§8 列出命名对齐结论与剩余待办。

---

## 0. 阅读前提

- 后端是 **Rust + axum**，对外 JSON over HTTP；前端是 React + TS，通过 Vite 把 `/api` 代理到
  后端（`http://127.0.0.1:8080`）。
- 通信模型是「**快速 ack + 后台执行 + SSE 实时推送 + 历史回放**」：一次对话不是同步返回，
  而是 `POST` 立即回 202，之后 agent 在后端跑，事件经 SSE 逐条推给前端（见 §2）。
- 前端消息的**权威类型定义**在 `frontend/src/types/index.ts`（6 类 discriminated union）。
  后端事件已能映射到这 6 类，字段命名已对齐（§8）。

---

## 1. 全局约定

| 项 | 约定 |
|---|---|
| Base URL | 前端经代理访问 `/api/...`（`/health` 除外），后端实际监听 `http://127.0.0.1:8080` |
| 请求/响应体 | 一律 `application/json`，UTF-8 |
| 字段命名 | 契约 **camelCase**（`toolName`、`taskId`、`maxVoltage`），后端已按此出网（§8） |
| 时间戳 | ISO 8601 字符串（含时区偏移），如 `"2026-10-21T13:25:36.546+00:00"`。落库是毫秒整数，出网转 ISO |
| 标识符 `id` | **字符串**（DB 里是整数主键，出网统一 `.to_string()`） |
| 枚举取值 | 见各字段说明；均为小写字符串 |
| 错误响应 | 统一 `{"error": "<可读信息>"}`，见 §7 |

---

## 2. 通信模型（轴心）

```
前端                             后端
 │ POST /api/sessions/{id}/messages   {message:"..."}
 │───────────────────────────────────────────────>
 │ <──── 202 {ok:true}            （立即返回，不等 agent 跑完）
 │
 │ GET /api/sessions/{id}/events  （SSE 长连接，订阅事件流）
 │ <──── event: message, data: {"type":"user",...}
 │ <──── event: message, data: {"type":"tool-call",...}
 │ <──── event: message, data: {"type":"progress",...}
 │ <──── event: message, data: {"type":"result",...}
 │ <──── event: message, data: {"type":"assistant",...}
 │
 │ （断线/首屏）GET /api/sessions/{id}/messages  一次性回放全部历史
 │───────────────────────────────────────────────>
 │ <──── 200 {messages:[ ...同一套 6 类消息 JSON... ]}
```

要点：

1. `POST /messages` **只落库 + 返回 ack**，真正的 agent 循环在后台 `tokio::spawn` 跑。
2. 事件**同时落 `messages` 表**（DB 是持久化真相）并**推 SSE**（只负责实时），两者共用同一条
   消息 id。因此「实时 SSE」与「历史回放」产出**完全同构**的 JSON。
3. SSE 断线不续传（见 §6），前端靠 `GET /messages` 回放补齐。

---

## 3. 端点目录

| 方法 | 路径 | 作用 | 状态 |
|---|---|---|---|
| GET | `/health` | 存活探针 | ✅ 已实现 |
| GET | `/api/sessions` | 会话列表（status 按最新任务推导） | ✅ 已实现 |
| POST | `/api/sessions` | 新建会话 | ✅ 已实现 |
| PATCH | `/api/sessions/{id}` | 重命名会话 | ✅ 已实现 |
| DELETE | `/api/sessions/{id}` | 删除会话 | ✅ 已实现 |
| GET | `/api/sessions/{id}/messages` | 回放历史消息 | ✅ 已实现 |
| POST | `/api/sessions/{id}/messages` | 发用户消息 → 触发 agent → 202 | ✅ 已实现 |
| POST | `/api/sessions/{id}/attachments` | 上传附件 → 回发 `attachment` 消息 | ✅ 已实现 |
| GET | `/api/sessions/{id}/events` | SSE 订阅事件流 | ✅ 已实现 |
| GET | `/api/tasks/{id}/curves` | 曲线数据 | ✅ 已实现 |
| GET | `/api/tasks/{id}/report` | 报告下载 | ✅ 已实现 |

---

## 4. 端点明细

### 4.1 `GET /health`

- 响应：`200`，`text/plain` 体为 `ok`。用于存活探测，无 JSON。

### 4.2 `GET /api/sessions` — 会话列表

- 响应 `200`：

```json
{
  "sessions": [
    {
      "id": "1",
      "title": "检查模型并跑时序仿真",
      "status": "completed",
      "updatedAt": "2026-10-21T13:25:41.945+00:00"
    }
  ]
}
```

字段：`id` 字符串；`title` 字符串；`status` ∈ `completed | running | failed`
（按该会话最新任务状态推导：`succeeded→completed`、`pending/running→running`、
`failed/cancelled→failed`、无任务→`completed`）；`updatedAt` ISO 字符串，按最近活跃排序。

### 4.3 `POST /api/sessions` — 新建会话

- 请求体：

```json
{ "title": "" }
```

`title` 可省略（默认空字符串；首条用户消息截断回填为标题是后续优化）。

- 响应 `201`：

```json
{
  "id": "2",
  "title": "",
  "status": "completed",
  "updatedAt": "2026-10-21T13:25:36.546+00:00"
}
```

### 4.4 `GET /api/sessions/{id}/messages` — 回放历史

- 响应 `200`：

```json
{
  "messages": [
    { "type": "user", "id": "14", "timestamp": "...", "content": "..." },
    { "type": "tool-call", "id": "15", "timestamp": "...", "toolName": "inspect_model", "args": { "model_path": "/tmp/m.dss" }, "status": "running" },
    { "type": "result", "id": "20", "timestamp": "...", "taskId": "sim_...", "title": "...", "metrics": { "maxVoltage": 0.0, "minVoltage": 0.0, "violationNodes": 0, "violationDuration": "0 h" } }
  ]
}
```

- `messages` 是**有序**数组（按消息 id 单调递增），每项即 §5 定义的事件消息 JSON。
- 语义与 SSE 完全同构：前端对实时流与回放走**同一套渲染逻辑**。

### 4.5 `POST /api/sessions/{id}/messages` — 发消息并触发 agent

- 请求体：

```json
{ "message": "检查模型 /tmp/m.dss，然后跑时序仿真" }
```

- 响应 `202 Accepted`：

```json
{ "ok": true }
```

- 副作用：① 落一条 `user` 消息并推送；② 刷新会话 `updatedAt`；③ 后台 `spawn` agent run，
  随后事件经 SSE 逐个到达（见 §5 示例的完整时序）。**响应体不承载任何 agent 结果。**

### 4.6 `GET /api/sessions/{id}/events` — SSE 事件流

- 见 §6。无请求体，响应为 `text/event-stream`。

### 4.7 `PATCH /api/sessions/{id}` — 重命名会话

- 请求体：

```json
{ "title": "新的标题" }
```

- 响应 `200`：同 §4.3 的 `Session` 形状（`status` 已按最新任务推导）。
- 会话不存在 → `404 {"error":"资源不存在"}`。

### 4.8 `DELETE /api/sessions/{id}` — 删除会话

- 响应 `204 No Content`（无响应体）。会话不存在 → `404`。
- 副作用：级联删除该会话的消息/任务（FK `ON DELETE CASCADE`），并注销其 SSE 通道。

### 4.9 `POST /api/sessions/{id}/attachments` — 上传附件

- 请求：`multipart/form-data`，字段名 `file`（含文件名与内容）。
- 响应 `201`：一条 `attachment` 消息（§5.6），该消息同时落库并推送 SSE。
- 上限 50MB；会话不存在 → `404`。

### 4.10 `GET /api/tasks/{id}/curves` — 曲线数据

- 响应 `200`：`CurveData`（§9.1 的形状）。无曲线数据 → `404 {"error":"该任务没有曲线数据"}`。

### 4.11 `GET /api/tasks/{id}/report` — 报告下载

- 响应 `200`：二进制下载，`Content-Type` 按报告类型、`Content-Disposition: attachment; filename="…"`。
- 无报告 → `404 {"error":"该任务没有报告"}`。

---

## 5. 事件消息 schema（6 类）

所有事件都是**带 `type` 标签的扁平 JSON**（discriminated union）。公共字段：

| 字段 | 类型 | 说明 |
|---|---|---|
| `type` | string | 判别字段，取值见下 |
| `id` | string | 消息 id（落库自增，字符串化） |
| `timestamp` | string | ISO 8601，消息创建时刻 |

### 5.1 `user`

```json
{ "type": "user", "id": "14", "timestamp": "2026-10-21T13:25:36.546+00:00", "content": "检查模型 /tmp/m.dss，然后跑时序仿真" }
```

| 字段 | 类型 | 说明 |
|---|---|---|
| `content` | string | 用户原文 |

### 5.2 `assistant`

```json
{ "type": "assistant", "id": "22", "timestamp": "2026-10-21T13:25:41.945+00:00", "content": "两步都已完成…" }
```

| 字段 | 类型 | 说明 |
|---|---|---|
| `content` | string | Markdown 文本 |

### 5.3 `tool-call` — 工具调用卡片

```json
{ "type": "tool-call", "id": "15", "timestamp": "2026-10-21T13:25:38.268+00:00", "toolName": "inspect_model", "args": { "model_path": "/tmp/m.dss" }, "status": "running" }
```

```json
{ "type": "tool-call", "id": "16", "timestamp": "2026-10-21T13:25:38.268+00:00", "toolName": "inspect_model", "args": { "model_path": "/tmp/m.dss" }, "status": "success", "summary": "model ok" }
```

| 字段 | 类型 | 说明 |
|---|---|---|
| `toolName` | string | 工具名（`inspect_model` / `run_simulation`） |
| `args` | object | 工具入参 |
| `status` | string | `running` → `success` / `failed` |
| `summary` | string? | 仅在 `success`/`failed` 时出现；一句话结果（如 `"task_id: sim_..."`） |

- 生命周期：同一工具先发一条 `running`，结束再发一条 `success`/`failed`（两条 `id` 不同，靠
  `toolName` + `args` 关联）。前端按 `status` 切换卡片状态。

### 5.4 `progress` — 阶段推进

```json
{ "type": "progress", "id": "18", "timestamp": "2026-10-21T13:25:39.982+00:00", "taskId": "sim_1792589139982_0", "stages": [
  { "key": "inspect", "label": "模型检查", "status": "done" },
  { "key": "sim",     "label": "时序仿真", "status": "running" },
  { "key": "analyze", "label": "结果分析", "status": "pending" },
  { "key": "report",  "label": "报告生成", "status": "pending" }
] }
```

| 字段 | 类型 | 说明 |
|---|---|---|
| `taskId` | string | 所属仿真任务 id |
| `stages` | array | 阶段数组，每次**全量**下发当前各阶段状态 |
| `stages[].key` | string | 阶段标识（`inspect/sim/analyze/report`） |
| `stages[].label` | string | 展示名 |
| `stages[].status` | string | `pending` → `running` → `done`，失败为 `failed` |

- 同一 task 会收到多条 `progress`（逐档推进），前端应**替换**该 task 的阶段展示而非追加。

### 5.5 `result` — 仿真结果卡片

```json
{ "type": "result", "id": "20", "timestamp": "2026-10-21T13:25:39.985+00:00", "taskId": "sim_1792589139982_0", "title": "配电网时序仿真结果", "metrics": { "maxVoltage": 0.0, "minVoltage": 0.0, "violationNodes": 0, "violationDuration": "0 h" } }
```

| 字段 | 类型 | 说明 |
|---|---|---|
| `taskId` | string | 所属任务 id |
| `title` | string | 结果标题 |
| `metrics.maxVoltage` | number | 最高电压（p.u.，引擎 summary/曲线聚合，取不到为 `0.0`） |
| `metrics.minVoltage` | number | 最低电压（p.u.，同上） |
| `metrics.violationNodes` | number | 越限节点数 |
| `metrics.violationDuration` | string | 越限时长（展示文本，如 `"0 h"` / `"2.5 h"`） |

### 5.6 `attachment`（已定义，由上传端点产出）

```json
{ "type": "attachment", "id": "...", "timestamp": "...", "file": { "fileName": "report.pdf", "fileSize": "1.2 MB", "fileType": "pdf" } }
```

| 字段 | 类型 | 说明 |
|---|---|---|
| `file.fileName` | string | 文件名 |
| `file.fileSize` | string | 大小（展示文本） |
| `file.fileType` | string | 类型 |

---

## 6. SSE 线格式

- 响应头 `Content-Type: text/event-stream`，带 keep-alive 心跳（axum `KeepAlive` 默认发注释行）。
- 每条事件：

```
event: message
data: {"type":"user","id":"14","timestamp":"...","content":"..."}

```

  - `event:` 固定为 `message`；`data:` 是 §5 的完整 JSON，**单行**。
  - 事件间以空行分隔。
- **没有** `id:` 字段 → 不产生 `Last-Event-ID`，**不支持断点续传**。
- 底层是 `broadcast` 有界通道（容量 256），订阅者若积压滞后，**最旧事件被静默丢弃**。
  因此 SSE 只保证「实时尽力送达」，完整性靠 `GET /messages` 回放兜底。
- 订阅在「懒创建」下从订阅时刻起收事件；历史消息不会在 SSE 里重放（首屏请先 `GET /messages`
  再开 SSE，或开 SSE 后立即补一次回放）。

---

## 7. 错误处理

- 统一响应体：`{"error": "<可读信息>"}`。
- `404`：`RepoError::NotFound`（会话不存在、删除/重命名目标缺失）以及显式 `not_found`
  （无曲线/报告、缺 `file` 字段、未知会话上传）。
- `500`：其余仓储错误与适配器错误（`From<AdapterError>`）。
- 前端约定：收到非 2xx 时读取 `body.error` 展示；agent 运行期错误不体现在 HTTP 状态上，而是以
  `assistant` 消息形式进事件流（如「agent 运行失败：…」）。

---

## 8. 命名对齐（已完成）

**字段命名已对齐**：后端 `application/events.rs` 的 `AppEvent` 已按前端权威类型出网，并有
`wire_format_matches_frontend_types` 单测锁定（含 `attachment` 断言与 `args` 归一）：

| 概念 | 前端类型（权威） | 后端出网（现状） | 状态 |
|---|---|---|---|
| 工具调用类型标签 | `'tool-call'` | `"tool-call"` | ✅ 已对齐 |
| 工具名字段 | `toolName` | `toolName` | ✅ 已对齐 |
| 任务 id 字段 | `taskId` | `taskId` | ✅ 已对齐 |
| metrics 字段 | `maxVoltage/minVoltage/violationNodes/violationDuration` | 同左 | ✅ 已对齐 |
| 阶段字段 | `key/label/status` | 同左 | ✅ 已对齐 |
| 附件字段 | `file.fileName/fileSize/fileType` | 同左 | ✅ 已对齐 |

实现要点（`#[serde(tag = "type", rename_all = "camelCase")]` 只改变体名、不改字段名）：
`ToolCall` 变体加 `#[serde(rename = "tool-call", rename_all = "camelCase")]`，`Progress`/`Result`
变体加 `#[serde(rename_all = "camelCase")]`，把 `tool_name`→`toolName`、`task_id`→`taskId`。

第 5 步补齐的契约要点：

- 会话 `status` 已按最新任务状态推导（§4.2）。
- `metrics.maxVoltage`/`minVoltage`/`violationDuration` 已由适配器聚合真实值（尽力解析，取不到回退 `0.0`/`"0 h"`）。
- `attachment` 事件已由上传端点产出。
- `tool-call.args` 在事件层收口归一：`null` → `{}`，前端 `Object.entries` 永不抛异常。

---

## 9. 曲线与报告契约（已实现）

### 9.1 `GET /api/tasks/{id}/curves` — `CurveData`

返回 `frontend/src/types/index.ts` 的 `CurveData`：

```json
{
  "variable": "电压",
  "unit": "p.u.",
  "timeAxis": ["00:00", "00:15", "…"],
  "series": [
    { "name": "bus1.1", "values": [1.02, 1.01, "…"] }
  ]
}
```

- 数据来源是引擎 `--result-dir` 落盘的 CSV（`db-design.md` §5.2「曲线存文件」）。内核曲线 CSV
  格式尚未固化，后端用**尽力启发式**解析（时间列 `{time,time_s,t}` 否则合成下标；系列键由
  `{bus,phase,element,name,feeder}` 拼 `.`；值列 `{v_mag,v,p,q,value}`），失败软回退。真实格式
  落地时只改 `adapters/process.rs` 的解析函数，出网形状不变。

### 9.2 `GET /api/tasks/{id}/report` — 报告下载

- `reports/{task_id}.md` 存在则返回；否则由 `summary.csv` 合成一份 Markdown 兜底；都没有则 `404`。

### 9.3 对前端契约的影响

- `progress.stages` 的 UI 阶段键（`inspect/sim/analyze/report`）与 `simulation_tasks.stage`
  （`model_check→script_gen→running→collecting`）是**两套状态**，各自推进、互不合并。
- 上述第 5 步变化**只改数值/取值与新增端点，不改既有消息结构与字段命名**，前端无需改渲染逻辑。
