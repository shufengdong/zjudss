# 前端功能设计（面向后端 agent 的对接说明）

> 受众：**后端 agent**。本文说明前端如何消费后端数据、依赖哪些行为，以及后端还需补齐/对齐什么。
> **权威类型定义**：`frontend/src/types/index.ts`，所有字段名与枚举值以它为准，前端不做任何转换。
> 通信契约（端点 / SSE 线格式）见 `docs/api-design.md`；开发状态见 `docs/development-status.md`。

---

## 1. 前端消费的数据契约（逐条，字段名必须一字不差）

### 1.1 Session

```ts
interface Session {
  id: string          // DB 整型主键，出网统一 .to_string()
  title: string
  status: 'completed' | 'running' | 'failed'
  updatedAt: string   // ISO 8601，侧栏按它分组（今天/昨天/更早）与排序
}
```

### 1.2 消息（6 类 discriminated union，`type` 为判别字段）

公共字段：`id: string`、`timestamp: string`（ISO 8601）。

| `type` | 额外字段（精确命名） | 说明 |
|---|---|---|
| `user` | `content: string` | 用户原文 |
| `assistant` | `content: string` | Markdown 文本（**一条完整消息，前端不做 token 级流式渲染**） |
| `tool-call` | `toolName: string`<br>`args: Record<string, unknown>`<br>`status: 'running' \| 'success' \| 'failed'`<br>`summary?: string` | 工具调用卡片 |
| `progress` | `taskId: string`<br>`stages: { key, label, status }[]` | `status` ∈ `'pending' \| 'running' \| 'done' \| 'failed'` |
| `result` | `taskId: string`<br>`title: string`<br>`metrics: { maxVoltage, minVoltage, violationNodes, violationDuration }` | `maxVoltage/minVoltage/violationNodes` 为 number，`violationDuration` 为**字符串**（展示文本，如 `"0 h"`） |
| `attachment` | `file: { fileName, fileSize, fileType }` | 全为字符串 |

### 1.3 曲线 `CurveData`（`/curves` 端点目标形状，已定稿，见 api-design §9）

```ts
interface CurveData {
  variable: string      // 如 "电压"
  unit: string          // 电压表 v_mag 为 "V"（伏特），非 p.u.
  timeAxis: string[]    // 时间轴刻度（QSTS 为各时步秒值，如 "3600"/"7200"/"10800"）
  series: { name: string; values: number[] }[]  // 每条馈线/母线一条
}
```

---

## 2. 前端依赖的消息生命周期与顺序（后端必须遵守）

一次对话的事件顺序（前端按此渲染，不回退重排）：

```
user → tool-call(running) → [progress…] → tool-call(success|failed) → result → assistant
```

各类型的**硬性行为约定**：

1. **`user` 回显**：后端落库后**必须把该 user 消息经 SSE 再推一遍**。前端发送时先本地乐观展示
   （临时 id 前缀 `local-user-`），收到 SSE 的 user 事件后用真实 id 替换。若后端不回显，前端会
   残留乐观消息且拿不到真实 id。
2. **`tool-call` 是两条消息**：同一工具先发 `running`，结束再发 `success`/`failed`，**两条 id 不同**。
   前端各渲染成一张卡，**不合并**。请勿把状态合并进一条消息。
3. **`progress` 每次全量下发 stages**：同一 task 会收到多条 progress（逐档推进），前端按 `taskId`
   **替换**（非追加）。所以每次的 `stages` 必须是**完整数组**（四个阶段 `inspect/sim/analyze/report`
   都带当前状态），不能只发变化的那一档。
   - `taskId` 必须与 `tool-call` 的 summary、`result` 里的 `taskId` **同源**（同一个任务 id）。
4. **`result`**：一个 task 一条；`metrics` 四个字段齐全。
5. **`assistant`**：最终回复**整段**作为一条消息发（`content` 即完整 Markdown）。前端不消费 token
   级流，别按增量 token 发多条。
6. **顺序**：回放 `GET /messages` 返回的数组按 `id` 单调递增；前端渲染即按此顺序。

---

## 3. 前端依赖的端点与请求/响应

| 端点 | 前端发的 | 前端要的 | 备注 |
|---|---|---|---|
| `GET /api/sessions` | — | `{ sessions: Session[] }` | 按 `updatedAt` 降序 |
| `POST /api/sessions` | `{ title: string }` | 201 `Session` | 前端**传非空 title**（默认 `"新的仿真"`），后端须原样存回 |
| `GET /api/sessions/{id}/messages` | — | `{ messages: Message[] }` | 有序（id 升序），与 SSE 同构 |
| `POST /api/sessions/{id}/messages` | `{ message: string }` | 202 `{ ok: true }` | 前端不等结果，靠 SSE |
| `GET /api/sessions/{id}/events` | — | SSE，`event: message`，`data` 为单行 JSON | 前端用 `EventSource.onmessage`，事件名须为 `message` |
| `GET /api/tasks/{id}/curves` | — | `CurveData` | 已实现（形状见 §1.3 / api-design §9） |
| `GET /api/tasks/{id}/report` | — | 报告文件 | 已实现（Markdown 下载） |

**错误约定**：非 2xx 统一返回 `{ "error": "<可读信息>" }`，前端读取 `error` 展示。

---

## 4. 后端必须补齐 / 对齐的清单（按优先级）

> 这是前端当前无法工作的点，逐条可验证。

- [x] **字段命名对齐（已落地，api-design §8）**：后端出网已改为 `tool-call` / `toolName` / `taskId`，
      `tool-call`/`progress`/`result` 三类消息联调可正常渲染。
- [x] **`args` 必须是对象，不能是 `null`**：后端在事件层收口归一（`null` → `{}`），
      `ToolCallCard` 直接 `Object.entries(message.args)` 不会再抛异常。
- [x] **会话 `status` 动态化**：`GET /api/sessions` 按该会话最新任务状态推导
      （`running`/`failed`/`completed`）。
- [x] **`result.metrics` 真实值**：由适配器聚合 QSTS 结果 CSV（`*_summary.csv` 多行按最坏情形取、
      `load_state.v_min_pu` 取 min 电压、`voltage.v_mag` 按源母线归一到 p.u.），取不到回退 `0.0`/`"0 h"`。
- [x] **重命名 / 删除端点**：已补 `PATCH/DELETE /api/sessions/{id}`。前端可把本地乐观切换为真持久化。
- [x] **文件上传端点**：已补 `POST /api/sessions/{id}/attachments`（multipart 字段 `file`），
      回发一条 `attachment` 消息（落库 + SSE）。
- [x] **`/curves` 与 `/report`**：已实现（`GET /api/tasks/{id}/curves`、`/report`），前端结果面板可接入。

> ⚠️ **前端侧遗留（非后端范围）**：`App.tsx` 的 `appendEvent` 目前只替换 `local-user-*` 乐观消息，
> **未替换 `local-att-*`**。接入上传后，前端需补一条「SSE 收到 `attachment` 时替换本地 `local-att-*`
> 临时附件」的规则，否则会出现本地临时附件与真实附件并存的重复项。

---

## 5. 前端的渲染行为（隐含契约）

- **SSE 无回放、无 `Last-Event-ID`**：前端在 `selectSession` 时**先 `GET /messages` 回放、再开
  EventSource**。后端 broadcast 有界（丢最旧），断线靠回放兜底——请勿假设前端能断点续传。
- **`id` 一律字符串**：前端从不把 id 当整数解析；后端所有出网 id 用 `.to_string()`。
- **时间戳一律 ISO 8601 字符串**（含时区偏移），DB 存毫秒整数、出网转 ISO。
- **`progress` 是替换语义**：后端若想更新某一档，必须重发完整 stages；前端不会「增量叠加」。

---

## 6. 前端组件与数据流速览（后端不必关心细节，仅备查）

- 三栏布局：260px 侧栏（会话分组/新建/重命名/删除）· 中心对话流 · 380px 结果面板（<1280px 变抽屉）。
- 状态集中在 `App.tsx`（React 单 state，无 Redux）；`api/client.ts` 封 REST，`api/events.ts` 封 SSE。
- 消息合并逻辑 `appendEvent()`（`App.tsx` 模块级）：user 替换乐观、progress 按 taskId 归并、其余按 id 去重。
- 图表 `components/charts/LineChart.tsx`：ECharts 直连，props 传 `CurveData`，支持 dataZoom/legend/resize。
