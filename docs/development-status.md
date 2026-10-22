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
| 前端 v2 工作台 | ✅ 完成 | 按 session 状态、可恢复 SSE、历史检索、工作区、多文件/目录上传、案例库、任务历史和通用结果工作台 |
| 前端接入真实后端 | ✅ 完成 | v2 REST/SSE 已接线；旧曲线/报告、旧 multipart 和演示日志已删除 |
| 后端分层架构 | ✅ v2 Tasks 1–17 完成 | 身份、调度、SSE、artifact、工作区、摄取、仿真、分析、比较与历史已落地 |
| 后端数据库 schema | 🚧 待发布收口 | v2 增量表已齐备；Task 18 将压成一份仅支持空库初始化的 initial migration |
| 后端真实引擎接入 | ✅ v2 编排完成 | 启动探测 `TENSOREVAL_BIN` 的 `--result-dir`、`--output-format parquet` 与命名 Parquet input；运行物化任务私有输入树并登记可复现任务 |
| 后端结果端点 | ✅ 通用数据集 | 旧 CSV 指标、曲线和报告适配器/路由已删除；提供 ResultDataset 元数据、分页、列投影和 Parquet 下载 API |
| 会话状态/重命名/删除/附件 | ✅ v2 完成 | 前后端使用 `clientBatchId`、多 `file.<clientFileId>`、规范目录相对路径、幂等摄取和刷新恢复 |
| 前后端字段命名对齐 | ✅ 已对齐 | 后端出网 `tool-call`/`toolName`/`taskId` 与前端类型一致（§3） |
| 历史聊天浏览 | ✅ 基础功能完成 | 全量会话列表、今天/昨天/更早分组、打开回放、重命名和删除 |
| 历史聊天检索 | ✅ 完成 | FTS5 trigram、短查询有界 LIKE、消息分页/定位、侧栏搜索和 agent 历史工具 |
| 模型工作区与不可变版本 | 🚧 摄取闭环完成 | active/draft、规范 manifest hash、乐观基线、激活/回退、只读 API、model-status，以及 DataFrame、RSS、ZIP、rssbundle 摄取已落地 |
| 表头推断与多工作表 Excel | ✅ 摄取闭环完成 | CSV 有界采样、Excel 非空 sheet 转 Parquet、Parquet footer schema、catalog 列名评分、局部失败与 collecting draft 已实现 |
| 参数修改、任务历史与仿真对比 | ✅ 完成 | 不可变子版本、可复现任务、稳定键兼容性、差值分析与前端任务历史已接通 |
| 通用结果 DataFrame 分析 | ✅ 完成 | 原生 Parquet、自然语言 RSS 分析、持久血缘、分页表格和六类受控 ECharts 视图 |
| Agent 调度与容量保护 | ✅ 完成 | 全局 8、owner 2、同 session 1、等待队列 32；满载返回 `429 agent_queue_full` 且不写消息 |
| 历史文件配额 | ✅ 基础链路完成 | 内容寻址 blob、scope 唯一计数/逻辑字节、并发原子硬限制、80% 告警接口和无引用 GC 已实现；领域资源在后续任务接入引用增减 |
| 多用户隔离 | ✅ 基础边界完成 | instance/trusted_proxy、`/api/me`、storage/access scope 和 Session/Message/Task/SSE 404 隔离已落地 |
| Agent prompt 与领域技能 | ✅ 工作区阶段完成 | 每轮只注入 system、技能索引和资源摘要；技能/文档/案例按稳定 ID 有界加载，脚本修改缺技能返回 `skill_required`，当前 run 记录已加载技能集 |
| 内置案例库 | ✅ 导入闭环完成 | 启动时校验并索引根 RSS 与 rssbundle；案例按快照 ID 导入完整依赖闭包，复制并复核 hash，并已注册自然语言检索、查看与导入工具 |
| v2 详细实施方案 | 🚧 发布收口 | Tasks 1–17 已完成；正在执行 Task 18 的 schema、E2E、部署和发布门禁 |

> v2 实施以 `docs/superpowers/plans/2026-10-22-v2-simulation-workspace.md` 的 Task 编号为准；当前
> 已完成 Task 1–17。Task 18 将删除开发期增量 migration 并完成发布验收。
> 上表 `🎯` 表示已评审目标，不能理解为代码已具备该能力；目标实施顺序见
> `docs/model-workspace-design.md` §11。

### 2.1 v2 身份与恢复语义

当前代码已实现 `AUTH_MODE=instance|trusted_proxy`。公开部署必须使用可信代理；所有现有
Session/Message/Task/SSE 读写均由服务端 principal 注入 scope，跨 scope 和不存在资源返回相同 404。
聊天仍持久化在 SQLite；前端已使用“先打开 SSE、再读快照”的顺序恢复消息，并按稳定键去重。
`GET /api/me` 提供 `scopeKey`；前端以 `zjudss:last-session:<scopeKey>` 保存导航偏好，历史真相仍在服务端。

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
  `GET /api/sessions/{id}/events`（SSE）、工作区/版本/摄取只读端点、`GET /api/me`、`GET /health`。
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
- **引擎选择**：生产启动必须设置 `TENSOREVAL_BIN`；`ProcessEngine` 先执行最小 Parquet
  input/output smoke probe，不满足 v2 合同就启动失败。`StubEngine` 只用于显式注入的测试。

### 后端目录（实际）

```
backend/src/
├── main.rs            # 装配 AppState、迁移、路由、引擎选择（env）
├── adapters/          # v2 合同；process/{command,output}；测试 StubEngine
├── agent/             # provider、PromptAssembler、版本化 knowledge
├── api/               # mod.rs（router）、sessions.rs、events.rs（SSE）
├── application/       # AppState、chat、events（EventBus/AppEvent）、scheduler
├── db/                # pool/迁移/schema/models/repositories（全部显式接收 scope）
└── tools/             # 稳定 ID 的知识、案例和工作区自然语言工具
```

### tensoreval Parquet v2 边界（Task 11）

- `RunSpec` 只接受 application 物化的绝对任务输入树、规范相对脚本/输入路径、合法 input alias、
  绝对 staging result-dir 和 timeout；子进程 cwd 固定为任务输入树，参数数组不经过 shell。
- RSS、DataFrame、结果分析使用 `--df`；bundle 独立运行；全部显式传
  `--result-dir ... --output-format parquet`。
- 启动探测先读帮助合同，再真实运行一份最小命名 Parquet input/output；缺能力或输出不是可读的
  Snappy Parquet 时启动失败，不支持旧二进制降级。
- 输出校验拒绝 symlink、目录外逃逸、非 Parquet、损坏 footer、非法/重复变量名及文件数、单文件和
  总字节超限；`ParquetInspector` 提供有序 schema、row count、分页和列投影。
- v1 的 CSV 聚合、固定 `RunMetrics`、`read_curves/read_report` 和 `/curves`、`/report` 已删除。
  合法输出现已提交 ArtifactService 并登记 ResultDataset；当前仓库无可执行真实 binary，真实
  smoke test 以显式 ignored integration test 保留。

---

## 5. 前端实现要点

- `types/` 按 session/message/workspace/task/dataset 拆分并由 `index.ts` 汇总；出网 ID 均为字符串。
- `useSessionFeed` 以 session ID 键控消息，先建立 SSE 再合并最近消息页，支持向前分页且不会把迟到
  响应写入当前会话。localStorage 只保存当前 scope 的导航偏好。
- Composer 支持文件、多文件和目录；每次选择生成新 batch/file ID，目录文件携带规范相对路径。错传
  文件重新选择会产生新 ID，历史摄取和模型版本不会被覆盖。
- 案例库是只读 API；“使用此案例”、激活候选版本、比较任务等动作均转换成可见的自然语言用户消息。
- 工作台展示 active/draft、识别证据、缺失输入、活动摄取和仿真历史。结果消息按 dataset ID 打开独立
  DataFrame 工作台，大表数据不进入聊天全局状态。
- 每张 DataFrame 先提供 schema 顺序的 cursor 分页表格和 Parquet 下载。已保存视图只接受
  line/bar/scatter/heatmap/boxplot/histogram 六类受控 spec，本地 builder 不执行 formatter。

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
