# HTTP API 设计（前端开发契约）

> 版本：v1 已实现契约 + v2 目标契约
> 配套：架构见 `docs/architecture.md`（尤其 §9）、数据库见 `docs/db-design.md`、
> 前端类型权威定义见 `frontend/src/types/index.ts`
> 本文是**给前端开发 agent 的单点契约**：按本文实现，无需阅读 Rust 代码即可对接。
> 反向视角（前端如何消费、后端还需补齐什么）见 `docs/frontend-design.md`。
> 已实现端点均与当前代码一致（含一次真实 DeepSeek 冒烟测试的 SSE 实况）；§8 列出命名对齐结论与剩余待办。
> 模型工作区的目标端点和新增消息见 §10；完整设计见 `docs/model-workspace-design.md`。标为
> `🎯 目标` 的内容尚未实现，不得作为当前联调依据。

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
| GET | `/api/tasks/{id}/datasets` | 通用结果 DataFrame 目录 | ✅ |
| GET | `/api/datasets/{id}/rows` | Parquet 分页与列投影 | ✅ |

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
    { "type": "result", "id": "20", "timestamp": "...", "taskId": "7", "title": "...", "metrics": { "maxVoltage": 0.0, "minVoltage": 0.0, "violationNodes": 0, "violationDuration": "0 h" } }
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
- 生命周期：后端为这条消息创建一次短生命周期 agent run，并从该 session 的数据库消息恢复上下文；
  不为用户或 session 常驻一个 agent。v1 会直接 `spawn`，尚无 agent run 总量限制或同 session 串行化。
- v2 admission control：同一 session 同时只运行一个 run，其他消息按 ID 排队；实例活动 run 默认上限
  8（可配置），等待队列有界。队列满时返回下述 `429`，且不得落 user 消息或启动后台任务。

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

### 4.10 v1 曲线/报告端点（已删除）

`/api/tasks/{id}/curves` 与 `/report` 属于未投入使用的 v1 固定结果模型，v2 已删除。Task 12
提供任务数据集目录、Parquet 分页/投影与下载端点；图表由版本化视图规范读取数据集。

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
| `summary` | string? | 仅在 `success`/`failed` 时出现；一句话结果（如 `"task_id: 7"`） |

- 生命周期：同一工具先发一条 `running`，结束再发一条 `success`/`failed`（两条 `id` 不同，靠
  `toolName` + `args` 关联）。前端按 `status` 切换卡片状态。

### 5.4 `progress` — 阶段推进

```json
{ "type": "progress", "id": "18", "timestamp": "2026-10-21T13:25:39.982+00:00", "taskId": "7", "stages": [
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
{ "type": "result", "id": "20", "timestamp": "2026-10-21T13:25:39.985+00:00", "taskId": "7", "title": "配电网时序仿真结果", "metrics": { "maxVoltage": 0.0, "minVoltage": 0.0, "violationNodes": 0, "violationDuration": "0 h" } }
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

## 6. v1 SSE 线格式（历史记录）

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
- v1 订阅在「懒创建」下从订阅时刻起收事件；历史消息不会在 SSE 里重放。当前前端先
  `GET /messages` 再开 SSE，存在短暂丢事件窗口；v2 不沿用该顺序，恢复契约见 §10.8。

---

## 7. 错误处理

- 统一响应体：`{"error": "<可读信息>"}`。
- `404`：`RepoError::NotFound`（会话不存在、删除/重命名目标缺失）以及显式 `not_found`
  （无曲线/报告、缺 `file` 字段、未知会话上传）。
- `500`：其余仓储错误与适配器错误（`From<AdapterError>`）。
- v2 `429`：agent 等待队列已满，响应
  `{"error":"当前任务较多，请稍后重试","code":"agent_queue_full"}` 并携带 `Retry-After`；前端
  保留待发送文本并允许用户稍后重试。该响应遵守 §10.9 的 v2 稳定错误体。
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
  "unit": "V",
  "timeAxis": ["3600", "7200", "10800"],
  "series": [
    { "name": "SourceBus.1", "values": [66395.7, 66395.7, 66395.7] }
  ]
}
```

- 数据来源是引擎 `--result-dir` 落盘的 CSV（`db-design.md` §5.2「结果 DataFrame 存文件」）。内核 QSTS 曲线 CSV
  已实测对齐：结果文件名 = RSS 变量名（如 `qsts_voltage.csv`），电压表列为
  `bus,phase,v_re,v_im,v_mag,v_ang[,time,load_multiplier,generation_multiplier]`，**`v_mag` 单位是 V
  （相电压），非标幺**。后端启发式解析：时间列 `{time,time_s,t}`（有则以其为 X 轴、按
  `{bus,phase,element,name,feeder}` 拼系列，每系列跨时步取一列值）；无时间列则每行一个点、合成下标。
  值列 `{v_mag,v,p,q,value}`，`v_mag`→`("电压","V")`。失败软回退，真实格式变化只改 `process.rs` 解析函数。

### 9.2 `GET /api/tasks/{id}/report` — 报告下载

- `reports/{task_id}.md` 存在则返回；否则由结果目录的 summary CSV（`summary.csv` 或 `*_summary.csv`）
  合成一份 Markdown 兜底；都没有则 `404`。

### 9.3 对前端契约的影响

- `progress.stages` 的 UI 阶段键（`inspect/sim/analyze/report`）与 `simulation_tasks.stage`
  （`model_check→script_gen→running→collecting`）是**两套状态**，各自推进、互不合并。
- 上述第 5 步变化**只改数值/取值与新增端点，不改既有消息结构与字段命名**，前端无需改渲染逻辑。

---

## 10. v2 模型工作区契约（分阶段实施中）

本节中的身份提取、`GET /api/me` 以及现有 session/message/task/SSE 端点的 scope 校验已经实现；
其余端点按实施计划逐步落地。表内端点不能仅凭列出即视为已经可用。

### 10.1 端点目录

| 方法 | 路径 | 作用 |
|---|---|---|
| GET | `/api/me` | 返回当前认证主体和 access scope，供前端隔离导航偏好 |
| GET | `/api/examples` | 搜索当前引擎资源快照中的可导入 RSS/rssbundle 案例 |
| GET | `/api/examples/{id}` | 案例详情、依赖摘要、验证状态和运行能力要求 |
| GET | `/api/sessions` | v2 支持关键词/任务筛选和游标分页；空查询保持最近活跃列表 |
| GET | `/api/sessions/{id}/messages` | v2 支持最近页、向前分页和围绕命中消息取窗口 |
| POST | `/api/sessions/{id}/messages` | 提交自然语言和可选资源 context；通过 admission control 后触发 agent |
| GET | `/api/sessions/{id}/events` | 支持 event ID、`Last-Event-ID` 补发和 `resync-required` 的 SSE |
| GET | `/api/sessions/{id}/workspace` | 工作区、active/draft 版本、输入识别与活动摄取 |
| POST | `/api/sessions/{id}/attachments` | 一次上传一个或多个带 clientFileId 的文件 |
| GET | `/api/sessions/{id}/versions` | 按创建时间倒序列出模型版本 |
| GET | `/api/sessions/{id}/tasks` | 列出历史仿真，支持 `studyType/status/versionId` 过滤 |
| GET | `/api/ingestions/{id}` | 查询上传批次进度与结构化错误，供刷新恢复 |
| GET | `/api/versions/{id}` | 版本清单、DataFrame 绑定、补丁和诊断 |
| GET | `/api/tasks/{id}` | 任务输入版本、study spec、实际脚本和状态 |
| GET | `/api/tasks/{id}/datasets` | 原始仿真输出的 DataFrame 目录 |
| GET | `/api/analysis-runs/{id}` | 一次自然语言结果分析的脚本、输入、状态与输出目录 |
| GET | `/api/datasets/{id}` | DataFrame 元数据、schema、血缘和视图目录 |
| GET | `/api/datasets/{id}/rows` | 分页读取表格行 |
| GET | `/api/datasets/{id}/views` | 列出该数据集的已校验图表视图 |
| GET | `/api/dataset-views/{id}/data` | 预算内返回某个语义视图所需的完整数据投影 |
| GET | `/api/datasets/{id}/download` | 下载原始或派生 DataFrame 文件 |
| GET | `/api/task-comparisons/{id}` | 读取持久化比较结果 |
| GET | `/api/storage-usage` | 读取当前实例或登录 owner 的历史文件数量、容量和配额 |

所有资源先通过所属 session 校验，防止用猜测 ID 跨会话读取。版本、任务等会话内时间线使用
`limit + beforeId`（默认 50、最大 200）；会话搜索使用绑定查询条件的不透明 `cursor`（§10.10），
消息列表另支持 `beforeId` 和 `aroundId`（§10.10）。

#### 身份与 scope

v2 支持两种部署模式：`AUTH_MODE=instance` 只用于可信私有单租户；公开多人部署必须使用
`AUTH_MODE=trusted_proxy`。可信认证代理负责登录，并在转发前移除外部同名请求头，再注入
`X-Zjudss-Subject`、可选 `X-Zjudss-Tenant` 和 `X-Zjudss-Proxy-Token`。后端以常量时间比较
`AUTH_PROXY_TOKEN`，随后生成不可变 access scope；浏览器不能自行提交 owner 或 storage scope ID。

`GET /api/me` 返回：

```json
{
  "subject": "alice@example.com",
  "scopeKind": "owner",
  "scopeKey": "owner:alice@example.com"
}
```

instance 模式的 `scopeKind` 为 `instance`，`scopeKey` 为固定非密钥标识。该响应只供界面显示和给
`localStorage` 导航偏好分区；上次会话键固定为 `zjudss:last-session:<scopeKey>`。所有授权仍以
服务端 extractor 为准。可信代理模式缺少主体、缺少 token 或 token 错误均返回 401；已知但不属于
当前 scope 的资源与不存在资源使用相同的 404 响应。

#### 引擎文档与案例资源

后端启动时从 `${TENSOREVAL_CWD}/sparrowzz/dss` 建立只读 `engineResourceSnapshot`。Agent 只能通过
`search_engine_docs/read_engine_doc_section` 读取索引后的文档 section；这些是内部工具，不暴露接受
文件路径的 HTTP 接口。

`GET /api/examples?query=IEEE13&studyType=fault_study&sourceKind=rssbundle&limit=20&cursor=...`
返回只读案例目录：

```json
{
  "engineResourceSnapshot": "sha256:...",
  "examples": [
    {
      "id": "ex_7f0c2cba98a84d6f916a4b9e11",
      "title": "IEEE 123 Bus Short Circuit",
      "description": "IEEE123 多故障场景短路案例",
      "sourceKind": "rssbundle",
      "studyType": "fault_study",
      "solver": null,
      "tags": ["IEEE123", "short-circuit"],
      "requiredFeatures": [],
      "fileCount": 5,
      "sizeBytes": 184320,
      "verification": "self_executed",
      "importable": true,
      "engineContractVersion": "eig-rs-02718c716a31-sparrowzz-d6a502d3cd1e-schema2"
    }
  ],
  "nextCursor": null
}
```

`GET /api/examples/{id}` 另返回入口相对路径、受控文件树摘要、tree hash、来源/许可证说明、预计运行规模
和 capability 要求。响应不得包含 `TENSOREVAL_CWD` 或服务器绝对路径。不存在、源文件 hash 已变化、
依赖不完整和当前 tensoreval 缺少 MUMPS/MKL 等能力时，分别标记不可导入或返回稳定错误。

导入没有公开 POST 旁路。用户自然语言或案例卡片产生可见消息后，agent 调用
`import_example(exampleId, engineResourceSnapshot, expectedBaseVersionId)`；服务端重新校验 snapshot/hash，
把完整依赖复制进 artifact 存储并按普通 ingestion/workspace 事务发布版本。`expectedBaseVersionId`
取当前 draft，若无 draft 则取 active；仅当工作区二者都不存在时允许为 `null`。这样首次导入和并发
导入都使用同一套乐观并发语义。成功导入只发布模型版本和消息事件，不隐式创建仿真任务；运行仍需
用户在同一对话中明确提出。

### 10.2 工作区摘要

`GET /api/sessions/{id}/workspace`：

```json
{
  "id": "7",
  "sessionId": "3",
  "active": {
    "versionId": "11",
    "parentVersionId": "10",
    "sourceKind": "dataframes",
    "originKind": "upload",
    "status": "ready",
    "inputs": [],
    "missing": [],
    "diagnostics": [],
    "createdAt": "2026-10-22T08:30:00+08:00"
  },
  "draft": {
    "versionId": "12",
    "parentVersionId": "11",
    "sourceKind": "rssbundle",
    "originKind": "example",
    "origin": {
      "exampleId": "ex_7f0c2cba98a84d6f916a4b9e11",
      "engineResourceSnapshot": "sha256:...",
      "sourceTreeHash": "sha256:..."
    },
    "status": "ready",
    "inputs": [],
    "missing": [],
    "diagnostics": [],
    "createdAt": "2026-10-22T09:00:00+08:00"
  },
  "activeIngestions": [
    {"id":"31","originKind":"upload","status":"inferring","clientBatchId":"batch-01","updatedAt":"2026-10-22T09:00:02+08:00"}
  ]
}
```

版本 `status` 为 `collecting | ready | invalid`；`sourceKind` 为
`rss | rssbundle | dataframes | mixed`，`originKind` 为 `upload | example | generated`。example origin
必须返回稳定 example ID、resource snapshot 和源 tree hash，不能返回服务器路径。`resolution` 为
`pending | inferred | user_confirmed | rejected`。歧义或未知时使用 `pending` 并返回 `candidates`；
文件名和工作表名仅展示，`tableKind` 由表头（列名）推断
或用户确认；dtype 只用于 schema 校验和 `typeConflicts` 诊断。

`active` 是默认仿真使用的已确认版本，`draft` 是最近发布的待确认、不完整或校验失败候选版本。
工作区摘要只在 `activeIngestions` 中列出非终态摄取；摄取完成后一次性发布新的不可变 draft，同一 versionId
不会从 `collecting` 原地改成 `ready`。已有 active 时，摄取成功只能推进 draft，不能自动替换 active；
用户通过 agent 确认使用一个 `ready` draft 后
再原子更新 active。没有 active 的初次上传可在校验通过后直接激活。`inputId` 是版本内输入项身份，
替换、移除和确认映射均使用它；`artifactId` 只表示不可变内容。

### 10.3 多文件上传

`POST /api/sessions/{id}/attachments` 使用 `multipart/form-data`：

- 每个二进制 part 使用字段 `file.<clientFileId>`，不接受缺少 clientFileId 的旧 `file` 字段；
- v2 必填 `clientBatchId`：客户端生成的幂等批次 ID；
- 目录增强模式另交 `relativePaths` JSON 对象，以 clientFileId 为键保存规范化相对路径；
- 标准文件上传可直接接收 ZIP；`.rssbundle` 是目录，须通过目录增强模式上传其文件树，或由用户
  先压成 ZIP。后端不信任归档或目录上传中的相对路径；
- 成功接收返回 `202`，摄取、推断和校验经 SSE 更新。

```json
{
  "ok": true,
  "ingestionId": "31",
  "accepted": [
    {
      "clientFileId": "file-01",
      "artifactId": "41",
      "fileName": "model.zip",
      "deduplicated": false
    }
  ]
}
```

同一 `sessionId + clientBatchId` 重试必须返回同一摄取任务，不重复创建版本。服务端持久化
`clientFileId`，即使同名文件并存或处理乱序，响应和 attachment 事件也能与本地文件一一对应。
相同 batch/file ID 必须对应相同内容哈希；若字节已改变，返回 `409 idempotency_conflict`。纠正错传文件
属于一次新上传，应生成新的 batch/file ID，随后由用户通过自然语言指定替换关系。
可重试错误允许用相同 batch/file ID 重新处理原失败项；若工作区基线已变化，则返回
`409 version_conflict`，客户端改用新批次重新上传，不能把旧批次结果合并进新 draft。
同一 workspace 的摄取按接收顺序串行；版本修改请求携带 `expectedDraftVersionId`（没有 draft 时
携带 active version id），与服务端基线不一致时返回 `409 version_conflict`，客户端刷新工作区后
通过自然语言重新发起操作。

上传只负责摄取，不按同名文件或同类型表自动覆盖当前输入。用户通过自然语言要求替换/移除后，
agent 以版本输入 ID 调用相应领域工具；同类型冲突会通过 `model-status.ambiguities` 返回。重复内容
返回已有 artifact，并在响应中标记 `deduplicated: true`；该标记只表示 blob 复用，不得合并不同
clientFileId、相对路径或版本清单项。

v2 的上传 attachment 消息在现有 `file` 外增加 `clientBatchId`、`clientFileId`、`ingestionId` 和
`uploadStatus`；`uploadStatus` 为 `accepted | ingesting | ready | failed`。这些字段只描述上传项，
模型中的替换关系仍以工作区返回的 `inputId` 为准。它由 ingestion item 状态映射：
`accepted → accepted`，`processing → ingesting`，`ready → ready`，`failed → failed`；
`queued/uploading` 仅存在于请求到达后端之前的本地状态。
服务端只在上述粗粒度状态发生变化时追加一条新的 attachment 快照消息；同一文件的快照共享
`clientBatchId + clientFileId`，但各自拥有新的消息 `id`。前端据此前者更新文件项、据后者完成历史排序
和 SSE 去重，不能把每次状态更新追加成新的文件。

`GET /api/ingestions/{id}` 返回批次状态
`accepted/extracting/inferring/validating/succeeded/completed_with_errors/failed`、完整 stages、产出的
artifact/版本 ID 和结构化错误；每个原始上传项还返回
`clientFileId/status(accepted|processing|ready|failed)/artifactId/error`。全成功为 `succeeded`，成功与
失败项并存为 `completed_with_errors`，批次级致命错误或没有可用项才为 `failed`。这使页面刷新后不
依赖内存中的 SSE 事件恢复进度，并能单独重试失败文件。
活动摄取同时出现在 `GET /workspace` 的 `activeIngestions` 中，前端刷新后无需从消息文本查找 ID。

### 10.4 任务与结果

任务详情至少返回：

```json
{
  "id": "52",
  "sessionId": "3",
  "modelVersionId": "12",
  "status": "succeeded",
  "stage": "collecting",
  "studyType": "power_flow",
  "scriptArtifactId": "72",
  "agentBundleVersion": "v2.1",
  "agentSkillRefs": [
    {"id":"study-planning","version":"1.0.0","hash":"sha256:..."},
    {"id":"rss-generation","version":"1.0.0","hash":"sha256:..."}
  ],
  "engineContractVersion": "eig-rs-02718c716a31-sparrowzz-d6a502d3cd1e-schema2",
  "studySpec": {
    "solver": "radial",
    "results": ["bus_phase", "branch_flow", "summary"]
  },
  "createdAt": "2026-10-22T09:10:00+08:00",
  "startedAt": "2026-10-22T09:10:01+08:00",
  "finishedAt": "2026-10-22T09:10:04+08:00"
}
```

`studyType` 是筛选标签而不是封闭枚举；首批 UI 识别 `power_flow | short_circuit | qsts`，其他内核
分析类型仍以字符串返回。`studySpec` 必须使用带单位的领域字段，不能暴露服务器路径或拼好的命令行。
`scriptArtifactId` 指向实际执行的完整 RSS；`agentBundleVersion`、`agentSkillRefs` 和
`engineContractVersion` 记录生成与校验脚本时实际使用的知识版本，用于复现和诊断，不由客户端提交。

`GET /api/tasks/{id}/datasets` 返回 `--result-dir` 中每个命名 DataFrame 的目录：

```json
{
  "datasets": [
    {
      "id": "91",
      "name": "bus_phase",
      "rowCount": 14400,
      "format": "parquet",
      "columns": [
        {"name":"time","dtype":"f64","nullable":false,"unit":"s"},
        {"name":"bus","dtype":"string","nullable":false},
        {"name":"phase","dtype":"i64","nullable":false},
        {"name":"v_mag","dtype":"f64","nullable":false,"unit":"V"}
      ],
      "producer": {"kind":"simulation","id":"52"},
      "defaultViewId": null,
      "views": []
    }
  ]
}
```

这里没有 `converged/pLoss/maxVoltage` 等固定统计。用户询问具体结论后，agent 查看这些 schema，生成
RSS 分析脚本并调用 `run_result_analysis`。工具参数只包含脚本文本、dataset ID、input alias、预期
输出名和可选的初始语义视图；应用层解析真实路径，适配器执行：

```json
{
  "intent": "画出节点 632 三相电压",
  "inputs": [{"datasetId":"91","inputName":"voltage"}],
  "script": "selected = filter(all(), col(bus) == \"632\", voltage);",
  "outputs": [
    {
      "name": "selected",
      "views": [
        {
          "name": "三相电压趋势",
          "kind": "line",
          "spec": {"version":1,"kind":"line","x":"time","y":["v_mag"],"seriesBy":"phase"},
          "isDefault": true
        }
      ]
    }
  ]
}
```

示例只表达工具边界；生产实现必须使用与目标内核版本一致的 RSS 语法校验器，不能把模型文本直接
拼进 shell。

```text
tensoreval --df --result-dir <controlled-output-dir> --output-format parquet \
  --input <alias>=<resolved-dataset-file>... <generated-script.rss>
```

v2 的仿真和结果分析必须显式传 `--output-format parquet`；不能依赖 tensoreval 当前默认的 CSV。
`result_datasets` 只登记校验成功的 Parquet artifact。v2 删除原型 `/curves`、`/report` 和固定 metrics
契约；需要报告时由结果分析生成结构化 DataFrame 或后续定义新的通用 artifact 端点，不沿用旧接口。

一次分析运行详情：

```json
{
  "id": "88",
  "sessionId": "3",
  "requestMessageId": "207",
  "intent": "画出节点 632 三相电压",
  "status": "succeeded",
  "scriptArtifactId": "105",
  "agentBundleVersion": "v2.1",
  "agentSkillRefs": [
    {"id":"result-analysis","version":"1.0.0","hash":"sha256:..."}
  ],
  "engineContractVersion": "eig-rs-02718c716a31-sparrowzz-d6a502d3cd1e-schema2",
  "inputs": [{"datasetId":"91","inputName":"voltage"}],
  "outputs": [
    {
      "datasetId": "106",
      "name": "bus_632_voltage",
      "rowCount": 288,
      "columns": [
        {"name":"time","dtype":"f64","nullable":false,"unit":"s"},
        {"name":"phase","dtype":"i64","nullable":false},
        {"name":"v_mag","dtype":"f64","nullable":false,"unit":"V"}
      ],
      "previewRows": [[900,1,7200.1],[900,2,7189.4]],
      "previewTruncated": true,
      "defaultViewId": "301",
      "views": [
        {"id":"301","name":"三相电压趋势","kind":"line","isDefault":true}
      ]
    }
  ]
}
```

`GET /api/datasets/{id}/rows?limit=100&cursor=...` 返回：

```json
{
  "datasetId": "106",
  "columns": [
    {"name":"time","dtype":"f64","nullable":false,"unit":"s"},
    {"name":"phase","dtype":"i64","nullable":false},
    {"name":"v_mag","dtype":"f64","nullable":false,"unit":"V"}
  ],
  "rows": [[900,1,7200.1],[900,2,7189.4]],
  "nextCursor": "opaque-or-null"
}
```

服务端设置默认和最大 limit，禁止把大表全量装入消息或 SSE。列顺序与 `columns` 一致；非有限浮点、
无法表示的值和缺失值统一返回 `null`，并在 dataset 诊断中记录转换数量。

`GET /api/datasets/{id}/views` 返回同一 DataFrame 上的多个已保存视图。分页表格对所有 dataset
始终可用，不需要创建 `kind=table` 的 view：

```json
{
  "datasetId": "106",
  "defaultViewId": "301",
  "views": [
    {
      "id": "301",
      "name": "三相电压趋势",
      "kind": "line",
      "spec": {"version":1,"kind":"line","x":"time","y":["v_mag"],"seriesBy":"phase"},
      "isDefault": true
    }
  ]
}
```

`GET /api/dataset-views/301/data` 在规模预算内返回完整 DataFrame 投影，前端用该 view 的 spec 编译
ECharts option：

```json
{
  "viewId": "301",
  "datasetId": "106",
  "columns": [
    {"name":"time","dtype":"f64","nullable":false,"unit":"s"},
    {"name":"phase","dtype":"i64","nullable":false},
    {"name":"v_mag","dtype":"f64","nullable":false,"unit":"V"}
  ],
  "rows": [[900,1,7200.1],[1800,1,7198.6],[900,2,7189.4]]
}
```

首批 `kind` 为 `line | bar | scatter | heatmap | boxplot | histogram`。`spec` 的公共字段是
`version: 1` 和同值 `kind`，其余字段按 kind 校验：

| kind | 必需映射 | 可选映射/选项 |
|---|---|---|
| `line` | `x`, `y[]` | `seriesBy`, `smooth` |
| `bar` | `x`, `y[]` | `seriesBy`, `stack` |
| `scatter` | `x`, `y` | `colorBy`, `sizeBy` |
| `heatmap` | `x`, `y`, `value` | — |
| `boxplot` | `category`, `min`, `q1`, `median`, `q3`, `max` | — |
| `histogram` | `binStart`, `binEnd`, `count` | `seriesBy` |

`VisualizationSpec` 只允许上述列映射及受控显示选项；API 不接受或返回任意
`EChartsOption`、JavaScript formatter 或函数。服务端校验列存在、dtype、单位、类别/series 数以及
点数或格子数，前端只负责把规范编译成 ECharts 配置。图表投影超过预算时返回
`422 visualization_data_too_large` 和经过筛选的规模诊断，不得静默截取前 N 行；用户随后可要求 Agent
筛选、聚合或降采样并生成派生 DataFrame。

用户只要求把现有列改画成另一种图时，agent 调用 `create_dataset_view`，在原 dataset 上追加 view，
无需执行 tensoreval。若需要筛选、聚合、透视、分箱、分位数或改变数据语义的降采样，则调用
`run_result_analysis` 先生成派生 DataFrame，再创建 view。箱线图和直方图使用 RSS 产出的明确统计列，
前端不重新计算统计结论。多数据集图先由 RSS join 成带明确血缘的派生 DataFrame。

`GET /api/datasets/{id}/download` 返回对应 `.parquet` artifact，使用
`Content-Type: application/vnd.apache.parquet` 和经过清理的 `<resultName>.parquet` 文件名；API 不为
下载临时转成 CSV。若以后需要 CSV 导出，应作为独立、受大小限制的导出能力设计，不能改变持久化格式。

### 10.5 比较资源（由 agent 创建）

前端不直接 POST 比较资源，而是按 §10.7 发送自然语言消息。agent 的 `compare_simulations` 工具调用
应用服务时使用以下参数：

```json
{"baselineTaskId":"52","candidateTaskId":"61"}
```

应用服务创建并返回：

```json
{
  "id": "8",
  "baselineTaskId": "52",
  "candidateTaskId": "61",
  "status": "succeeded",
  "analysisRunId": "94",
  "outputDatasetIds": ["112"]
}
```

比较必须先选择两侧数据集并校验列、dtype、单位、稳定业务键和维度；`studyType` 不作为封闭兼容
条件。存在时间列时把时间纳入连接键。
比较逻辑由 RSS 脚本执行，差值保存为一个或多个 DataFrame；接口不预设最大变化、损耗或电压等字段。
输入版本和 study spec 差异另从任务元数据读取。不兼容返回 `409 comparison_incompatible`。

### 10.6 新增消息类型

v2 将消息 union 扩展到 8 类。公共字段仍为 `id/type/timestamp`。

```ts
interface ModelStatusMessage {
  id: string
  type: 'model-status'
  timestamp: string
  workspaceId: string
  scope: 'active' | 'draft'
  snapshot: null | {
    versionId: string
    status: 'collecting' | 'ready' | 'invalid'
    recognizedTables: { inputId: string; artifactId: string; tableKind: string; confidence: number }[]
    missing: string[]
    ambiguities: { inputId: string; candidates: string[] }[]
  }
}

interface AttachmentMessageV2 {
  id: string
  type: 'attachment'
  timestamp: string
  file: { fileName: string; fileSize: string; fileType: string }
  clientBatchId: string
  clientFileId: string
  ingestionId: string
  uploadStatus: 'accepted' | 'ingesting' | 'ready' | 'failed'
}

interface ComparisonMessage {
  id: string
  type: 'comparison'
  timestamp: string
  comparisonId: string
  baselineTaskId: string
  candidateTaskId: string
  analysisRunId: string
  title: string
  outputs: ResultOutputRef[]
}

interface ResultOutputRef {
  datasetId: string
  name: string
  rowCount: number
  columns: { name: string; dtype: string; nullable: boolean; unit?: string }[]
  previewRows?: unknown[][]
  previewTruncated: boolean
  defaultViewId: string | null
  views: DatasetViewRef[]
}

interface DatasetViewRef {
  id: string
  name: string
  kind: 'line' | 'bar' | 'scatter' | 'heatmap' | 'boxplot' | 'histogram'
  isDefault: boolean
}

interface ResultMessageV2 {
  id: string
  type: 'result'
  timestamp: string
  taskId?: string
  analysisRunId?: string
  title: string
  outputs: ResultOutputRef[]
}
```

`ResultMessageV2.taskId` 与 `analysisRunId` 恰有一个出现，对应 dataset 的唯一生产者；比较使用独立的
`ComparisonMessage`，其中 outputs 由关联的 analysis run 产生。

消息中的 `defaultViewId/views` 是该消息发布时的视图快照。`create_dataset_view` 成功后不改写旧消息，
而是按 dataset 原生产者再发布一条 `result`：原始仿真 dataset 复用其 `taskId`，派生 dataset 复用其
`analysisRunId`。前端按“生产者 ID + datasetId”归并后得到最新快照；打开结果时仍以
`GET /api/datasets/{id}/views` 为当前事实来源。纯换图不创建虚假的 analysis run。比较上下文可同时
发布新的 `comparison` 快照，但底层 view 目录仍以 dataset 资源为准。

`run_result_analysis` 的工具结果与 `result.outputs` 都可以携带同一份有上限预览，建议最多 20 行且序列化
后不超过 32 KiB；超过任一限制时 `previewTruncated=true`。Agent 用预览生成文字回答，前端用它首屏
展示；完整内容始终从 dataset 读取。用户询问单个数值时，RSS 也必须输出一行 DataFrame，不能依赖
stdout 中的标量文本。

`snapshot: null` 明确清空对应 scope，例如激活 draft 后先更新 active，再把 draft 置空；前端不能通过
“长时间没收到 draft 消息”推断它已被清除。

v2 的 `progress` 不再复用裸 `taskId`：

```ts
interface ProgressMessage {
  id: string
  type: 'progress'
  timestamp: string
  operation: { kind: 'ingestion' | 'simulation' | 'analysis'; id: string }
  stages: { key: string; label: string; status: 'pending' | 'running' | 'done' | 'failed' }[]
}
```

摄取阶段为 `extract/infer/validate/version`；仿真阶段为 `prepare/inspect/sim/collect`；结果分析阶段为
`script/run/collect/present`。同一操作
的每条 progress 仍发送完整 stages 数组，前端按 `operation.kind + operation.id` 归并。

`tool-call` 在 v2 增加稳定 `callId`。同一工具调用的 running 与 success/failed 事件拥有不同消息
`id`、相同 `callId`，前端据此更新一张卡片。`assistant` 仍只在
`StreamAssistantItem::End` 收到完整文本时产生；中间 token 不作为前端增量事件发送。

assistant 消息可选增加 `suggestions`：

```ts
interface ChatSuggestion {
  id: string
  label: string
  message: string
  context?: {
    action: 'import_example' | 'run_study' | 'rerun_task' | 'query_results' | 'visualize_results' | 'compare_tasks' |
            'modify_model' | 'replace_input' | 'remove_input' |
            'activate_version' | 'confirm_mapping'
    refs: Record<string, string>
  }
}
```

建议是一次性自然语言回复选项。点击后前端把 `message` 作为新的 user 消息提交；服务端不得把建议
当成已经获得执行授权。携带资源 ID 的 context 应由应用层根据当前会话资源生成或重新校验，不能
信任模型自由生成的 ID。

### 10.7 自然语言动作上下文

领域交互以自然语言为默认入口。文件上传、案例只读浏览和确定性摄取通过固定控件启动；没有 active 时首个校验通过
的 ready 版本可自动激活。模型映射确认、替换/移除、参数修改、后续版本激活/回退、运行/重跑和任务
比较不提供绕过 agent 的前端写入口。用户直接输入文本时可以不带 context；从模型、版本或任务列表
发出的上下文建议向现有消息端点发送可见文本和可选的消歧 context：

```json
{
  "message": "比较任务 52 和 61",
  "context": {
    "action": "compare_tasks",
    "refs": {"baselineTaskId":"52","candidateTaskId":"61"}
  }
}
```

`action` 首批为 `run_study | rerun_task | query_results | visualize_results | compare_tasks | modify_model |
replace_input | remove_input | activate_version | confirm_mapping | import_example`。案例卡片的 refs 使用
`exampleId` 与当前 `engineResourceSnapshot`；服务端校验后仍由 agent 调用导入工具。其他 refs 中资源
必须属于路径中的 session，随后把
文本和可信资源引用交给 agent；
context 本身不能绕过工具调用直接写库。比较创建逻辑位于 application service，由
`compare_simulations` 调用；公开 HTTP 只提供 `GET /api/task-comparisons/{id}` 读取结果。

前端可直接调用的固定控件限于高频、低歧义操作：会话管理、文件选择/上传/失败重试、发送消息、
只读视图切换、列表筛选分页、Parquet 原文件下载、图表图片导出和图表操作。极高频领域快捷项即使做成按钮，
也必须生成可见 user 消息并走本节协议，不能直接调用 application service。

### 10.8 SSE 恢复契约

该契约的消息补发、`id:`、`resync-required`、先订阅后查询及前端消息快照恢复已在 v2 Task 4
实现；workspace/tasks 快照将在相应资源端点实现后并入同一恢复流程。

每个持久化消息的 SSE 帧设置 `id: <messageId>`。客户端建立连接并收到 open 后再读取消息、工作区
和任务快照；快照期间收到的事件先缓存，随后按消息 ID 和资源更新时间归并。断线重连携带
`Last-Event-ID` 时，后端先从 messages 表补发该 ID 之后的同 session 消息，再切换到实时 broadcast。
若游标已超出保留范围，服务端发送 `resync-required` 事件，客户端重新读取三类快照。

`resync-required` 是传输层控制事件，不属于 §10.6 的 8 类 Message，也不写入 messages 表或携带
消息 `id`：

```text
event: resync-required
data: {"reason":"cursor_unavailable"}

```

其余持久化事件仍使用 `event: message` 和 `id: <messageId>`。客户端收到控制事件后丢弃当前暂存
窗口并重新读取消息、工作区和任务快照，再恢复正常消费。

为消除服务端补发阶段的竞态，SSE handler 必须先订阅 broadcast 并暂存实时消息，再查询数据库游标、
发送补发记录，最后按消息 ID 去重并释放暂存消息。不能先查数据库再订阅 broadcast。

### 10.9 v2 错误体

v2 错误体同时提供可读信息和稳定机器码：

```json
{
  "error": "检测到两个可能的 DataFrame 类型",
  "code": "dataframe_schema_ambiguous",
  "details": {"inputId":"35","artifactId":"21","candidates":["load","generator"]}
}
```

错误码全集见 `model-workspace-design.md` §10。`details` 不得包含服务器绝对路径、密钥或未经筛选的
内核 stderr。视图 spec 引用不存在或不兼容的列时返回 `422 invalid_visualization_spec`；图表数据超过
series、类别、点数或格子预算时返回 `422 visualization_data_too_large`，`details` 只给实际规模、
上限和建议的自然语言操作，不返回部分图表数据。

引擎资源启动预检失败返回 `503 engine_resource_unavailable` 或 `engine_contract_mismatch`；未知案例返回
`404 example_not_found`；案例未通过发布校验返回 `422 example_not_importable`；目录在 snapshot 建立后
发生 hash 变化返回 `409 example_source_changed`；当前二进制缺少案例声明的 solver feature 返回
`409 engine_feature_unavailable`。这些错误不得回显源目录绝对路径。

### 10.10 历史聊天查询与分页

#### 会话列表/搜索

`GET /api/sessions` 支持以下参数；不带参数时返回最近活跃会话：

| 参数 | 类型 | 说明 |
|---|---|---|
| `query` | string? | 标题、用户/助手文本和附件展示名关键词；去首尾空白后最长 200 字符 |
| `studyType` | string? | `power_flow/short_circuit/qsts` 等任务类型 |
| `taskStatus` | string? | 任务状态 |
| `from` / `to` | ISO 8601? | 会话最近活跃时间范围 |
| `limit` | integer? | 默认 30，最大 100 |
| `cursor` | string? | 服务端生成的不透明游标，客户端不得自行拼接 |

空 `query` 按 `updatedAt DESC, id DESC`；非空 `query` 按全文相关度优先，再按
`updatedAt DESC, id DESC`。
筛选条件使用 AND 组合。响应：

```json
{
  "sessions": [
    {
      "id": "3",
      "title": "IEEE13 负荷修改与短路分析",
      "titleSource": "auto",
      "status": "completed",
      "updatedAt": "2026-10-22T09:10:04+08:00",
      "lastMessagePreview": "已完成三相短路分析……",
      "messageCount": 28,
      "match": {
        "kind": "message",
        "messageId": "117",
        "snippet": "……将 load_1 的有功负荷调整为 120 kW……",
        "matchedTerms": ["负荷", "短路"]
      }
    }
  ],
  "nextCursor": "opaque-or-null"
}
```

`match` 只在搜索时出现；`snippet` 是纯文本，前端不得当 HTML 渲染。`matchedTerms` 仅用于安全文本
高亮。没有更多数据时 `nextCursor` 为 `null`。游标包含排序位置和规范化查询哈希；查询或筛选改变后
旧游标失效，返回 `400 invalid_cursor`。

#### 消息分页与命中定位

```text
GET /api/sessions/{id}/messages?limit=50
GET /api/sessions/{id}/messages?limit=50&beforeId=117
GET /api/sessions/{id}/messages?aroundId=117&before=20&after=20
```

- 无游标时返回最新一页，但数组内部仍按消息 ID 升序，便于直接渲染；
- `beforeId` 返回更早一页和 `nextBeforeId`，前端向上滚动时前插并按 ID 去重；
- `aroundId` 返回命中消息前后窗口及 `hasEarlier/hasLater`，用于搜索结果定位；
- `aroundId` 与 `beforeId` 互斥，非法组合返回 `400 invalid_pagination`；
- 所有 session/message ID 先做归属校验，未授权统一表现为 404，避免枚举资源。

v2 分页响应为 `{ "messages": [...], "nextBeforeId": "..." | null }`；around 模式额外返回
`anchorMessageId/hasEarlier/hasLater`。前端只接受此分页响应，不保留 v1 全量数组兼容分支。

#### agent 历史检索工具

`search_chat_history` 只调用 application history service，不直接拼 SQL。参数包括自然语言关键词、
可选日期范围、分析类型、任务状态和 limit（默认 5、最大 20），返回会话 ID、标题、命中摘要、
相关任务 ID 和匹配原因。`read_chat_context` 以 session ID 和可选 message ID 读取受限窗口，不把整段
长会话一次塞入模型上下文。

“上次”“修改前”等指代先在当前 session 的任务/版本时间线解析；找不到唯一对象才调用跨会话搜索。
相近候选必须让用户确认。工具只检索当前身份可访问的 session；instance 模式的范围是当前可信私有
部署实例，公开多用户部署必须使用 trusted_proxy 身份范围。

#### 自动标题

`Session.titleSource` 为 `default | auto | user`。首条有意义的 user 文本写入后，如果来源不是 user
且此前没有有意义的用户文本，后端在同一应用流程中用规范化文本截断生成 auto 标题；只有上传时
可用文件名生成临时 auto 标题，并在第一条用户文本到来时替换一次。PATCH 重命名把来源改为 user，
此后自动标题逻辑不得覆盖。标题生成不额外依赖 LLM，后续若增加模型摘要标题，也必须异步执行并
遵守 titleSource 乐观锁。

### 10.11 历史文件配额

`GET /api/storage-usage` 是高频只读端点。instance 模式返回整个可信私有实例的使用量；trusted_proxy
模式只返回当前 owner/tenant，不能接受客户端传入任意 owner ID：

```json
{
  "scope": "owner",
  "artifactCount": 1260,
  "artifactLimit": 20000,
  "logicalBytes": 4831838208,
  "byteLimit": 21474836480,
  "warningRatio": 0.8,
  "isWarning": false,
  "isExceeded": false
}
```

配额按当前 owner 引用的唯一 artifact 计数和计逻辑字节；同一 owner 的重复引用只计一次，跨 owner
分别计费。会话、消息、版本、任务和视图等纯数据库记录不计入 `artifactCount`。达到 80% 不阻止
操作；达到数量或字节硬上限后，所有只读 API、纯视图创建和对已有
artifact 的重复引用继续可用。上传、仿真和结果分析的提交若会增加唯一 artifact 数或逻辑字节，则
失败并返回：

```json
{
  "error": "历史文件空间已满，请删除不再需要的会话后重试",
  "code": "storage_quota_exceeded",
  "details": {
    "artifactCount": 20000,
    "artifactLimit": 20000,
    "logicalBytes": 20100000000,
    "byteLimit": 21474836480,
    "requestedBytes": 430000000
  }
}
```

仿真和分析输出先进入 staging，只有在提交 artifact/dataset 的事务内通过配额检查才发布成功消息；
并发提交必须对同一计费主体串行或使用原子条件更新。首版不设置自动过期天数，删除会话后由 GC 在
引用归零时释放额度。

结果分析通过 `run_result_analysis` 工具创建持久化 analysis run，输入只接受有序的 dataset ID/alias；
实际 RSS 脚本、agent bundle/skill refs 和引擎合同随运行保存。`GET /api/analysis-runs/{id}` 恢复运行与
派生数据集，`GET /api/dataset-views/{id}/data` 返回经过预算校验的视图投影。视图仅支持
line/bar/scatter/heatmap/boxplot/histogram 六类受控 spec，不接受 ECharts option 或 formatter。
