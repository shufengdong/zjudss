# 前端功能设计（面向后端 agent 的对接说明）

> 受众：**后端 agent**。本文说明前端如何消费后端数据、依赖哪些行为，以及后端还需补齐/对齐什么。
> **权威类型定义**：`frontend/src/types/index.ts`，所有字段名与枚举值以它为准，前端不做任何转换。
> 通信契约（端点 / SSE 线格式）见 `docs/api-design.md`；开发状态见 `docs/development-status.md`。
> §1–§6 记录 v1 历史契约；v2 已完成 §7.1 的 8 类消息、operation/callId 归并与 §7.7 的消息/SSE
> 恢复，工作区、任务快照和通用结果界面仍按后续任务实施。当前契约始终以源码权威类型为准。

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

### 1.2 消息（v1 历史合同）

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

一次成功仿真的典型事件顺序（前端按落库 id 渲染，不自行重排）：

```
user → tool-call(running) → [progress…] → result → tool-call(success) → assistant
```

工具失败时没有 `result`，顺序为 `… → progress(failed) → tool-call(failed) → assistant`；
模型检查工具没有 progress/result。工具内部先发布仿真进度和结果，rig 随后产生 `ToolResult`，因此
终态 tool-call 位于 result 之后。

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
| `PATCH /api/sessions/{id}` | `{ title: string }` | 200 `Session` | 重命名 |
| `DELETE /api/sessions/{id}` | — | 204 | 删除后前端切换到剩余会话 |
| `GET /api/sessions/{id}/messages` | — | `{ messages: Message[] }` | 有序（id 升序），与 SSE 同构 |
| `POST /api/sessions/{id}/messages` | `{ message: string }` | 202 `{ ok: true }` | 前端不等结果，靠 SSE |
| `POST /api/sessions/{id}/attachments` | multipart `file` | 201 `AttachmentMessage` | v1 每次请求一个文件 |
| `GET /api/sessions/{id}/events` | — | SSE，`event: message`，`data` 为单行 JSON | 前端用 `EventSource.onmessage`，事件名须为 `message` |
| `GET /api/tasks/{id}/curves` | — | `CurveData` | 已实现（形状见 §1.3 / api-design §9） |
| `GET /api/tasks/{id}/report` | — | 报告文件 | 已实现（Markdown 下载） |

**错误约定**：非 2xx 统一返回 `{ "error": "<可读信息>" }`，前端读取 `error` 展示。

---

## 4. v1 前后端对齐结果

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

前端上传流程不创建 `local-att-*` 乐观附件；它使用 POST 的 201 响应加入真实消息，随后收到同一
消息的 SSE 时按 id 去重。因此当前不存在临时附件重复项。

---

## 5. 前端的渲染行为（隐含契约）

- **v2 SSE 已支持恢复**：当前实现先等待 EventSource open，再读取消息快照；后端用持久消息 ID
  补发 `Last-Event-ID` 之后的消息，游标不可用或 broadcast lag 时发送 `resync-required`。
- **`id` 一律字符串**：前端从不把 id 当整数解析；后端所有出网 id 用 `.to_string()`。
- **时间戳一律 ISO 8601 字符串**（含时区偏移），DB 存毫秒整数、出网转 ISO。
- **`progress` 是替换语义**：后端若想更新某一档，必须重发完整 stages；前端不会「增量叠加」。

---

## 6. 前端组件与数据流速览（后端不必关心细节，仅备查）

- 三栏布局：260px 侧栏（会话分组/新建/重命名/删除）· 中心对话流 · 380px 结果面板（<1280px 变抽屉）。
- 会话/面板状态仍在 `App.tsx`；消息状态已拆到纯 reducer `state/messageReducer.ts`，
  `api/client.ts` 封 REST，`api/events.ts` 提供 `opened`、消息/控制事件回调和关闭方法。
- 消息按 ID 去重；progress 按 `operation.kind + operation.id`、tool-call 按 `callId`、模型状态和比较按
  各自稳定资源键归并。旧 session 的迟到响应由 reducer 丢弃。
- 图表 `components/charts/LineChart.tsx`：ECharts 直连，props 传 `CurveData`，支持 dataZoom/legend/resize。

---

## 7. v2 前端目标：模型工作区、仿真历史与对比

### 7.1 状态模型与类型扩展

实现 v2 时，`frontend/src/types/index.ts` 仍是出网字段的权威。工作区必须区分“当前可运行模型”
和“正在整理的新输入”，防止一次错传让已经可运行的模型失效：

```ts
export type VersionStatus = 'collecting' | 'ready' | 'invalid'
export type MappingResolution = 'pending' | 'inferred' | 'user_confirmed' | 'rejected'

export interface ModelInput {
  inputId: string             // version_artifacts.id；替换、移除和确认映射使用它
  artifactId: string          // 内容身份，只用于追溯和去重
  fileName: string
  sheetName?: string
  tableKind?: string
  confidence?: number
  resolution: MappingResolution
  candidates: { tableKind: string; confidence: number }[]
  matchedRequiredColumns: string[]
  missingRequiredColumns: string[]
  typeConflicts: string[]
}

export interface ModelSnapshot {
  versionId: string
  parentVersionId?: string
  sourceKind: 'rss' | 'rssbundle' | 'dataframes' | 'mixed'
  originKind: 'upload' | 'example' | 'generated'
  origin?: {
    exampleId: string
    engineResourceSnapshot: string
    sourceTreeHash: string
  }
  status: VersionStatus
  inputs: ModelInput[]
  missing: string[]
  diagnostics: string[]
  createdAt: string
}

export interface IngestionSummary {
  id: string
  clientBatchId: string
  originKind: 'upload' | 'example'
  status: 'accepted' | 'extracting' | 'inferring' | 'validating' |
          'succeeded' | 'completed_with_errors' | 'failed'
  updatedAt: string
}

export interface EngineExampleSummary {
  id: string
  title: string
  description: string
  sourceKind: 'rss' | 'rssbundle'
  studyType: string
  solver: string | null
  tags: string[]
  requiredFeatures: string[]
  fileCount: number
  sizeBytes: number
  verification: 'regression_tested' | 'self_executed'
  importable: boolean
  engineContractVersion: string
}

export interface WorkspaceSummary {
  id: string
  sessionId: string
  active: ModelSnapshot | null // 默认仿真只使用这个版本
  draft: ModelSnapshot | null  // 最近发布的候选版本；处理中批次见 activeIngestions
  activeIngestions: IngestionSummary[]
}

export interface SessionSummaryV2 extends Session {
  titleSource: 'default' | 'auto' | 'user'
  lastMessagePreview: string
  messageCount: number
  match?: {
    kind: 'title' | 'message' | 'attachment' | 'task'
    messageId?: string
    snippet: string
    matchedTerms: string[]
  }
}

export interface SessionPage {
  sessions: SessionSummaryV2[]
  nextCursor: string | null
}
```

摄取处理中只更新 `activeIngestions`，完成后才原子发布新的不可变 draft；同一个 versionId 的内容和
状态不会被后续事件改写。初次上传时没有 active；首个 `ready` 版本可成为 active。已有 active 后，
后续上传、映射确认和修改
先推进 draft，只有 draft 为 `ready` 且用户通过自然语言确认使用后才提升为 active。回退只改变
active 指针，历史版本和任务不变。

v2 对现有消息类型做以下扩展：

```ts
export interface OperationRef {
  kind: 'ingestion' | 'simulation' | 'analysis'
  id: string
}

export interface ProgressMessageV2 extends BaseMessage {
  type: 'progress'
  operation: OperationRef
  stages: ProgressStage[]       // 每次仍为完整快照
}

export interface AttachmentMessageV2 extends BaseMessage {
  type: 'attachment'
  file: Attachment
  clientBatchId: string
  clientFileId: string
  ingestionId: string
  uploadStatus: 'accepted' | 'ingesting' | 'ready' | 'failed'
}

export interface ResultColumn {
  name: string
  dtype: string
  nullable: boolean
  unit?: string
}

export interface ResultOutputRef {
  datasetId: string
  name: string
  rowCount: number
  columns: ResultColumn[]
  previewRows?: unknown[][]
  previewTruncated: boolean
  defaultViewId: string | null
  views: DatasetViewRef[]
}

export interface ResultMessageV2 extends BaseMessage {
  type: 'result'
  taskId?: string
  analysisRunId?: string
  title: string
  outputs: ResultOutputRef[]
}

export interface ComparisonMessageV2 extends BaseMessage {
  type: 'comparison'
  comparisonId: string
  baselineTaskId: string
  candidateTaskId: string
  analysisRunId: string
  title: string
  outputs: ResultOutputRef[]
}

export interface DatasetTablePage {
  datasetId: string
  columns: ResultColumn[]
  rows: unknown[][]
  nextCursor: string | null
}

export type VisualizationKind =
  | 'line'
  | 'bar'
  | 'scatter'
  | 'heatmap'
  | 'boxplot'
  | 'histogram'

export interface DatasetViewRef {
  id: string
  name: string
  kind: VisualizationKind
  isDefault: boolean
}

export type VisualizationSpec =
  | { version: 1; kind: 'line'; x: string; y: string[]; seriesBy?: string; smooth?: boolean }
  | { version: 1; kind: 'bar'; x: string; y: string[]; seriesBy?: string; stack?: boolean }
  | { version: 1; kind: 'scatter'; x: string; y: string; colorBy?: string; sizeBy?: string }
  | { version: 1; kind: 'heatmap'; x: string; y: string; value: string }
  | { version: 1; kind: 'boxplot'; category: string; min: string; q1: string;
      median: string; q3: string; max: string }
  | { version: 1; kind: 'histogram'; binStart: string; binEnd: string;
      count: string; seriesBy?: string }

export interface DatasetView extends DatasetViewRef {
  datasetId: string
  spec: VisualizationSpec
}

export interface DatasetViewData {
  viewId: string
  datasetId: string
  columns: ResultColumn[]
  rows: unknown[][]
}
```

普通 `result` 的 `taskId/analysisRunId` 恰有一个存在；前端以该生产者 ID 和 datasetId 归并，不能按
标题或 DataFrame 名称猜测身份。消息里的 `defaultViewId/views` 是发布快照；纯换图成功后后端用
dataset 原生产者再发一条 result，前端以相同归并键更新卡片。真正打开结果时重新读取 `/views`，
不能把历史消息中的 view 列表当作当前目录。

摄取、仿真和结果分析分别以 `ingestion:{id}`、`simulation:{id}`、`analysis:{id}` 归并，禁止只按
裸 ID 合并。`result.outputs` 引用后端持久化的 DataFrame，不包含预设统计字段；分页表格对每个
dataset 始终可用，一个 dataset 还可以有多个 `DatasetView` 图表视图。
v2 图表使用 `VisualizationSpec + DatasetViewData`，不复用 v1 只适合共享时间轴的 `CurveData`。
spec 只表达列到视觉通道的语义映射，前端再编译成 ECharts option；消息和 API 都不接受模型生成的
任意 ECharts option。
结果卡先用至多 20 行的 `previewRows` 展示即时答案；`previewTruncated=true` 时明确显示“仅预览”，
展开后再读取分页表格或图表端点。

`MessageType` 增加 `model-status | comparison`，原有 `attachment` 扩展为上述上传项快照；
`tool-call` 增加稳定 `callId`。同一 `callId` 的
running/success/failed 更新一张卡片，消息自身的 `id` 仍用于落库排序和 SSE 恢复。v1 原型未投入使用，
前端直接采用 v2 消息合同，不保留缺少 `callId`、裸 `taskId` progress 或旧四指标 result 的兼容分支。

v2 的 assistant 消息可选携带上下文建议，但建议本身仍是一句将要发出的自然语言：

```ts
interface ChatSuggestion {
  id: string
  label: string
  message: string
  context?: PostMessageRequest['context']
}
```

点击建议后，前端把 `message` 作为新的 user 消息正常发送并显示在聊天流中。建议不能直接调用领域
写接口；用户始终可以编辑建议文本或完全自己输入。

### 7.2 页面信息架构

- 左侧会话栏表示聊天历史；每个会话恢复自己的工作区和任务历史；
- 中央对话流是命令入口，展示用户/助手消息、工具状态、上传进度、模型状态和比较摘要；
- 右侧工作台只保留三个一级视图：**模型、仿真、结果**；
- “模型”展示 active 与 draft、识别表、缺失项、歧义、摄取进度和版本链；空工作区及模型页可打开
  只读“案例库”，但不增加第四个一级工作台；
- “仿真”展示任务历史、分析类型、绑定版本、状态、时间和原始 DataFrame 目录；详情中的“运行信息”可
  折叠展示实际 `agentBundleVersion`、`agentSkillRefs` 与 `engineContractVersion`，便于复现和排查；
- “结果”展示当前任务和派生分析的数据集，可在数据表、多个已保存可视化、文件信息和血缘之间切换；
- 版本差异、大表格和多图表对比使用可调整宽度的工作台或全宽详情层，不能挤在固定 380px 中；
- 窄屏继续使用抽屉，抽屉记住当前一级视图和选中的任务。

界面不建设“仿真操作按钮墙”或默认参数表单。模型、仿真和结果区域以查看状态、选择上下文为主，
下一步操作由输入框承接；智能体需要用户确认时，可在回复下方给出少量、随上下文变化的自然语言
建议。

重新上传同类型表后，draft 显示“等待确认”，active 仍显示“当前运行版本”。用户确认替换、并存、
忽略或回退后，界面刷新两个快照；被替换输入仍可从历史版本进入查看。

文件上传、确定性摄取和无 active 时首个 ready 版本的自动激活按既定流程执行。上传后的
替换/忽略/设为当前、任务比较以及按参数重跑等其他领域写操作都走 agent：它们把选中的稳定 ID
作为隐藏结构化上下文，并发送可见自然语言消息。
读取历史、展开结果和下载报告可直接调用只读 REST API。

### 7.3 上传与状态合并

Composer 的文件选择支持多选 `.rss/.zip/.csv/.xls/.xlsx/.parquet`；另提供目录选择以接收模型文件夹
和 `.rssbundle` 目录。`.rssbundle` 在内核中是包含 `manifest.json/main.rss/dss/inputs` 的目录，
不能假定为单个文件。标准流程可上传用户预先生成的 ZIP；目录模式由后端接收带安全相对路径的
多个文件，前端不默认把大型目录完整压缩到浏览器内存。

每批上传生成稳定 `clientBatchId`，每个文件生成稳定 `clientFileId`。失败重试保留二者，后端据此
保证整批幂等并把乱序返回准确关联到本地文件。前端文件状态为
`queued | uploading | accepted | ingesting | ready | failed`，不能按文件名或 FIFO 猜测关联关系。
重试同一失败内容才复用 ID；用户重新选择了修正后的文件时生成新 ID，避免幂等键对应不同字节。

摄取事件顺序：

```text
attachment(local) → attachment(server) → progress(ingestion:31)
                  → model-status(draft) → assistant
```

`attachment(server)` 携带 `clientBatchId/clientFileId/ingestionId`，按
`clientBatchId + clientFileId` 替换对应本地项；后续 `uploadStatus` 快照也按同一键更新，不能按消息
`id` 追加成多个文件。消息 `id` 只用于历史排序和 SSE 去重。`model-status` 按
`workspaceId + scope(active|draft)` 归并，`comparison` 按 `comparisonId` 归并，progress 按
`operation.kind + operation.id` 归并。`model-status.snapshot = null` 时必须清空对应 scope，例如 draft
被激活后的清理。不同摄取批次和仿真任务可以同时显示。

批次 `completed_with_errors` 表示至少一个文件 ready、至少一个文件 failed；成功项可以形成 draft，
失败项保留结构化错误和单独重试入口。`succeeded/completed_with_errors/failed` 在当前尝试结束后从
`activeIngestions` 移除；显式重试会让同一 ingestion 重新出现。刷新后通过 ingestion 详情恢复每个
文件的最终状态。

#### 7.3.1 内置案例库

案例库调用 `GET /api/examples`，支持关键词、分析类型、RSS/rssbundle、solver 和运行能力筛选。卡片显示
标题、说明、网络/分析标签、文件数、大小、验证状态，以及 MUMPS/MKL 或长任务要求。详情只展示案例内
安全相对路径与来源信息，不能显示服务器上的 `TENSOREVAL_CWD`。

“使用此案例”是常用入口，但点击后不直接调用导入服务。前端向现有消息端点发送可见文本，例如“导入
IEEE 123 Bus Short Circuit 案例”，并附带 `action=import_example`、`exampleId` 和当前
`engineResourceSnapshot`；agent 再调用 `import_example`。导入进度沿用 ingestion UI，成功后按普通
model-status 更新 active/draft，但不自动运行。已有 active 时必须明确显示“已作为候选版本导入，尚未
替换当前版本”。

案例详情提供“在对话中询问”入口，允许用户继续说“把这个案例的故障母线改为 632 后运行”。修改通过
版本工具产生子版本，界面同时展示“来源案例”和当前版本链；源案例不会被改写。

### 7.4 交互状态与数据归属

v2 不再把所有异步状态继续堆进 `App.tsx`，也不要求引入 Redux。至少拆为以下领域 hook 或等价的
reducer/store，组件只消费选择后的视图状态：

- `useSessionFeed(sessionId)`：消息、乐观项、SSE、事件归并和重连；
- `useExamples(filters)`：只读案例目录、详情、分页和当前 engine resource snapshot；
- `useWorkspace(sessionId)`：active/draft、输入映射、摄取和版本链；
- `useSimulationHistory(sessionId)`：任务分页、筛选、选中项和运行状态；
- `useTaskDatasets(taskId)`：原始结果 DataFrame 目录；
- `useAnalysisRun(analysisRunId)`：查询脚本、输入血缘、实际技能与 agent/engine 合同版本、状态和输出数据集；
- `useDataset(datasetId)`：schema、分页表格、视图目录、血缘和文件下载；
- `useDatasetView(viewId)`：视图规范、预算内完整数据、规模诊断和渲染错误；
- `useComparison(comparisonId)`：兼容性、关联分析运行和差值数据集。
- `useStorageUsage()`：当前实例或 owner 的历史 artifact 数量、逻辑容量、告警阈值和硬限制。

服务端资源按 ID 规范化存储，选择会话只改变查询键。旧会话的迟到响应必须通过 sessionId 校验后
丢弃，切换会话不复用前一会话的 selectedTask、draft 或比较选择。

### 7.5 自然语言优先与固定控件白名单

产品默认交互是用户和智能体对话。以下领域意图不设置直接执行的固定按钮或常驻表单：

- 识别或确认 DataFrame 类型；
- 替换、移除、合并模型输入，激活或回退版本；
- 修改模型参数、线路、负荷或分析参数；
- 选择潮流、短路、QSTS 等分析并开始或重新仿真；
- 查询并解释结果、定位异常、比较两次仿真。

用户可以直接输入“用刚上传的负荷表替换原来的，然后重新做潮流”“比较这次和修改前的电压偏差”
等表达。前端只负责把当前选中的 input/version/task 作为结构化 context 带上，不能把自然语言意图
拆成绕过 agent 的多个写接口。

固定控件只保留高频、低歧义且无需领域推理的功能：

| 固定控件 | 行为 |
|---|---|
| 新建、切换、重命名、删除会话 | 会话管理；删除仍需明确确认 |
| 选择文件/目录、上传、失败重试 | 文件传输；上传完成后如何使用由对话决定 |
| 浏览、搜索和筛选案例 | 只读；“使用此案例”转换为可见自然语言消息并走 agent |
| 发送消息 | 对话基础操作 |
| 展开模型/任务/结果、切换视图 | 只改变展示和选择上下文 |
| 搜索、筛选、排序、分页 | 只读列表操作 |
| 查看、Parquet 原文件下载、图表图片导出、图表缩放 | 只读结果操作 |
| 查看历史文件用量 | 只读配额信息；instance 模式按实例，trusted_proxy 模式按当前 owner/tenant |

智能体回复下方可以出现 1–3 个临时自然语言建议，例如“用新文件替换原负荷表”“保留两份并让我
选择”“忽略这次上传”。它们由当前对话生成，不是常驻功能按钮；点击后必须产生一条可见 user 消息
并继续走 agent。极高频且语义唯一的领域操作将来可以增加固定快捷项，但仍必须发送可见自然语言
消息，是否增加应由真实使用频率决定。

例：用户在历史列表中选中任务 52 和 61 后，界面可出现建议“比较任务 52 和 61”。点击时前端向
`POST /messages` 发送该文本，并附带只供后端解析的结构化 context；聊天流显示同一句 user 消息，
agent 再调用 `compare_simulations`。替换输入、确认表类型、激活版本、回退和重跑同理。

为避免 LLM 从展示文本重新猜 ID，消息请求目标扩展为：

```ts
interface PostMessageRequest {
  message: string
  context?: {
    action: 'run_study' | 'rerun_task' | 'query_results' | 'visualize_results' | 'compare_tasks' |
            'modify_model' | 'replace_input' | 'remove_input' |
            'activate_version' | 'confirm_mapping'
    refs: Record<string, string>
  }
}
```

服务端必须校验 context 中所有资源属于当前 session；`message` 仍是审计记录和用户可见文本，context
只解决引用消歧，不能直接执行写操作。

### 7.6 历史聊天查询

v1 只会全量读取全部 session，并按“今天 / 昨天 / 更早”分组；打开会话后再全量读取全部消息。
这只能用于少量历史记录。v2 将“普通浏览”和“自然语言查询”建立在同一个历史检索服务上：

#### 固定侧栏查询（高频只读操作）

- 侧栏顶部保留搜索框，输入停顿 300ms 后请求；新请求取消或忽略旧请求；
- 空查询按最近活跃排序，分为“今天 / 昨天 / 近 7 天 / 近 30 天 / 更早”；
- 有关键词时按相关度、最近活跃时间排序，不再按日期分组；
- 可选筛选项为日期范围、分析类型和任务状态，收进一个筛选入口，不常驻展示大量按钮；
- 每项展示标题、最近消息摘要、更新时间、仿真状态和命中摘要；命中摘要按纯文本渲染；
- 会话列表使用游标和无限滚动；打开会话先取最近 50 条，向上滚动再加载更早消息；
- 命中具体消息时，打开会话后读取该消息附近窗口并滚动定位，不能只跳到聊天底部；
- 命中定位模式提供“查看最新消息”，并可向前/向后加载；用户发送新消息前先退出定位模式、加载最新页；
- 清空查询恢复最近会话列表；搜索失败保留关键词并显示局部重试。

#### 自然语言查询

用户可以在聊天中说“找到上周改过负荷后做短路分析的对话”“打开最近一次 IEEE13 潮流”“比较这次
和上个月那次结果”。agent 调用 `search_chat_history`，返回候选会话卡片；卡片展示命中原因、摘要、
日期和相关任务。“打开会话”属于只读导航固定控件，可以直接切换并定位到命中消息；“比较”仍生成
一条可见自然语言 user 消息，再由 agent 执行。

若“上次”“修改前”等指代在当前会话内已有唯一任务，优先使用当前会话；只有当前会话无法解析时
才跨会话搜索。多个候选分数接近时，智能体列出最多 3 个候选让用户确认，不能静默选择。

#### 标题与参考边界

新会话先显示“新的仿真”。首条有意义的用户文本落库后，如果标题没有被用户手动修改，后端用
规范化文本截断生成标题；用户手动重命名后标记为 `user` 来源，后续不再自动覆盖。只上传文件的
会话可用文件名生成临时 auto 标题，第一条用户文本到来时再替换一次。

界面借鉴 DeepSeek 一类对话产品的左侧历史栏、跨端历史和会话管理思路，但不以复刻其具体网页为
目标。当前“分组、搜索、仿真筛选、命中定位”均按本项目需求定义，不能标注为 DeepSeek 官方交互。
可核验的官方参考仅包括 DeepSeek 公布的
[跨平台聊天历史同步](https://www.deepseek.com/news/deepseek-app/)和隐私政策中的
[历史访问、管理、复制与删除](https://cdn.deepseek.com/policies/en-US/deepseek-privacy-policy.html)；
官方资料未给出本项目所需的仿真筛选和跨会话自然语言检索契约。

### 7.7 历史恢复与 SSE

聊天历史以服务端数据库为真相来源，不依赖浏览器缓存。当前 v1 刷新后会重新拉取全部 session，按
`updatedAt` 降序打开第一条；它没有持久化刷新前的 `selectedId`。v2 在身份系统落地后，可以在
`localStorage` 中按当前 owner/tenant 保存最后选中的 session ID，并仅把它作为导航偏好：该 ID 必须
仍出现在服务端返回的可访问会话中才能恢复，否则回退到最近会话。浏览器存储的 ID 不能用于认证、
归属判断或 API 过滤；instance 模式也不能靠浏览器随机 ID 获得多用户隔离。

v2 页面启动时先调用 `GET /api/me`，以返回的 `scopeKey` 组成
`zjudss:last-session:<scopeKey>`。trusted_proxy 模式的登录和退出由同源认证代理负责；前端不保存
密码或代理 token，也不允许在 API 参数里选择 owner/tenant。

选择会话时使用“订阅就绪后读取快照”的顺序：

```text
1. 创建 EventSource，等待 open；暂存随后收到的事件
2. 并行 GET /messages、/workspace、/tasks?limit=50
3. 以快照初始化各领域 store
4. 按消息 id / 资源 updatedAt 回放暂存事件
5. 进入实时消费
```

这避免 GET 完成到订阅建立之间的丢事件窗口。SSE 的每个持久化消息还应写 `id: <messageId>`；自动
重连携带 `Last-Event-ID`，后端先补发该 ID 之后的持久化消息，再接实时 broadcast。客户端对消息
ID 去重；模型和任务快照按 `updatedAt` 单调合并，旧响应不能覆盖新 SSE 状态。

`GET /workspace` 必须包含 `activeIngestions`，因此刷新后无需从聊天文本猜 ingestion ID。三个快照
请求彼此独立；单个失败只显示对应区域的重试状态，不清空另外两个成功结果。任务和版本列表使用
游标分页，筛选变化时取消或忽略旧请求。

后端发出的 `event: resync-required` 是独立的传输层控制事件，不进入 Message union，也不作为聊天项
渲染。收到后清空本轮暂存事件，重新读取消息、工作区和任务三个快照；其他持久化事件仍通过
`event: message` 处理。

### 7.8 DataFrame 表格、多视图可视化与对比展示

结果卡只展示输出 DataFrame 的名称、行数、关键列、默认 view ID 和视图入口，不假设存在收敛、网损、
电压或越限字段。分页数据表对每个 dataset 始终可用；同一个 dataset 可以追加多个命名图表视图，
例如“电压趋势”“母线×时刻热力图”和“相别电压分布”。结果工作台使用
`数据表 | <视图一> | <视图二>...` 切换，已有视图切换属于只读高频操作，可以使用固定标签。

首批开放 `line/bar/scatter/heatmap/boxplot/histogram`。前端建立 `ChartRenderer` 和按 kind 注册的
option builder；公共层统一处理主题、颜色、legend、tooltip、dataZoom、resize、空状态、导出和
可访问性，各 builder 只负责把 `VisualizationSpec` 与 `DatasetViewData` 编译为 ECharts option。
其中 `histogram` 是领域语义，前端可编译成相邻 `bar` series；不能因为底层 series 相同就丢掉分箱边界。
不得把后端或 LLM 返回值直接当成 `EChartsOption` 执行，也不接受 formatter 函数或任意组件配置。

创建新视图仍以自然语言为入口。若用户说“把当前结果改成散点图”，且现有列可直接映射，agent 调用
`create_dataset_view` 追加一个 view，不执行 tensoreval。若用户要求筛选、join、聚合、透视、分箱、
分位数或改变结论的数据降采样，agent 先运行 RSS 生成派生 dataset，再为新表创建 view。箱线图和
直方图只消费 RSS 已产生的统计列，前端不计算业务统计。大结果不进入消息或全局状态；服务端限制
series、类别、点数和热力格子数。超限时前端展示规模诊断及“可在对话中要求筛选/聚合”的提示，
不能把前 N 行画成完整结果。

v2 的原始/派生结果下载统一为 `.parquet`；下载按钮直接保存后端 artifact，不在浏览器转换 CSV。
分页表格和 ECharts 数据仍消费 JSON API，因此前端渲染层不需要解析 Parquet。以后若增加 CSV 导出，
应显示为独立导出动作，并遵守服务端大小限制。

拓扑图、桑基图、仪表盘和雷达图不直接加入通用白名单：拓扑/桑基需要节点边等专门 schema，后两者
容易掩盖量纲和比例。真实需求出现后，为其定义新的 discriminated spec 和 renderer，再扩展契约。

对比卡先展示两个任务的版本、分析参数、比较脚本和输出 dataset。差值表中的业务键、基准值、候选
值、绝对差和相对差均来自 RSS 输出 schema，不在前端重新计算。逐设备或逐时点差值可以使用图表
组件，但必须显示单位和稳定业务键；未匹配对象应由脚本输出单独诊断表。比较创建后进入历史记录，
刷新后通过 comparisonId → analysisRunId → datasetId 恢复。

### 7.9 加载、错误与可访问性

- 模型、任务、结果各自维护 loading/error/empty，不使用覆盖全屏的统一 spinner；
- 上传失败保留文件项、结构化错误和“重试”，重试复用 clientBatchId/clientFileId；
- 案例源 hash 变化、依赖缺失或运行能力不足时保留用户消息和选择，刷新案例详情并展示稳定错误；
- active 可运行而 draft 失败时，输入区保持可用，并提示“可以继续使用当前版本分析”；
- 关键状态不能只靠颜色表达；progress、校验结果和越限同时提供文字与图标；
- 长任务在会话侧栏显示运行标记，离开该会话不会取消任务；
- 删除、回退和替换展示将要影响的 input/version，服务端冲突后刷新并要求用户重新发起。
- 发送消息遇到 `429 agent_queue_full` 时保留输入框内容和附件选择，显示“当前任务较多，请稍后重试”；
  不插入一条无法被后端回放的正式 user 消息。已进入同会话队列的消息显示“等待智能体处理”，并保持
  按发送顺序排列。
- 历史 artifact 数量或逻辑容量达到 80% 时在上传区显示非阻塞告警；达到硬限制后仍允许选择文件，
  因为内容去重后可能不增加额度。上传、仿真或分析收到 `storage_quota_exceeded` 时保留待提交内容并
  引导删除不需要的会话；历史查看、下载和纯视图切换/创建不禁用。

### 7.10 v2 前端待办与验收

- [ ] 以 v2 合同替换消息类型，覆盖 progress/result/tool-call/model-status/comparison；删除旧固定指标
      result card 和裸 taskId progress；
- [ ] 将事件归并键改为 operation kind + ID，并给工具卡增加稳定 callId；
- [ ] 将 SSE 改为订阅就绪后加载快照，支持 message event ID 和断线补发；
- [ ] 实现 active/draft 工作区、版本输入 ID 和刷新后的摄取恢复；
- [ ] 实现只读案例库搜索、筛选、详情和运行要求；“使用此案例”发送可见自然语言消息，导入后展示
      来源案例、resource snapshot 和派生版本链；
- [ ] 实现带 clientBatchId/clientFileId 的多文件上传、局部失败和幂等重试；
- [ ] 拆分会话流、工作区、任务历史、分析运行、数据集和比较的状态管理；
- [ ] 实现 dataset schema、分页表格、`dataset_views` 目录和血缘视图，禁止把大 DataFrame 放进消息/SSE；
- [ ] 实现 `ChartRenderer` 注册表及 line/bar/scatter/heatmap/boxplot/histogram builders，统一图例、
      缩放、导出、错误和数据超限提示；
- [ ] 支持一个 dataset 的多个命名视图与默认 view；新增视图走自然语言，已有视图用固定标签只读切换；
- [ ] 实现会话搜索、筛选、游标分页、消息向上分页和命中消息定位；
- [ ] 实现自然语言跨会话检索卡片，并对模糊的历史指代要求用户确认；
- [ ] 实现自动标题与手动标题锁定，避免后续消息覆盖用户标题；
- [ ] 实现“模型 / 仿真 / 结果”三级工作台及大内容详情层；
- [ ] 按固定控件白名单精简界面，领域操作通过自然语言消息 + 结构化 context 调用 agent；
- [ ] 支持 assistant 上下文建议，点击后生成可见 user 消息而非直接执行写操作；
- [ ] 验证用户只输入自然语言、不使用任何领域快捷项，也能完成上传后的确认、仿真、修改、重跑和对比；
- [ ] 覆盖 RSS、bundle、分批 DataFrame、根 RSS/rssbundle 案例导入、错传替换、移除、回退、重跑和
      前后对比；
- [ ] 覆盖 SSE 建连窗口、断线恢复、同 ID 不同操作、乱序上传、会话切换和旧响应覆盖；
- [ ] 覆盖空查询排序、中文关键词、跨会话筛选、深层消息定位、分页去重和删除后的索引清理；
- [ ] 覆盖未知分析类型、任意列 schema、无时间轴表格、六类图表、多输入 join、聚合/降采样、纯换图
      不运行 tensoreval，以及派生 dataset/多视图的刷新恢复。
- [ ] 接入 `/api/storage-usage`，覆盖 80% 告警、数量/容量任一超限、删除会话后额度释放。
