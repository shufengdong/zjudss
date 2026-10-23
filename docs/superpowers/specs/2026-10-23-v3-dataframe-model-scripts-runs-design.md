# V3 DataFrame 模型、脚本与运行结果生命周期设计

> 状态：已批准。配套身份设计见
> [V3 固定用户登录与数据隔离设计](2026-10-23-v3-fixed-user-auth-design.md)。本文只定义 V3，不为未上线的
> V1/V2 保留数据库、接口、工具或数据兼容。

## 1. 意图

V3 把“模型”“脚本”和“运行结果”拆成三个生命周期清晰的领域对象：

```text
外部模型文件 ──import──> 可变 DataFrame 草稿 ──用户确认──> 不可变 ModelVersion
                                                            │
上传/Agent 生成 .rss ──> 会话级 ScriptResource ──用户点击运行┘
                                                            │
                                                            ▼
                                                    SimulationRun
                                                            │
                                                            ▼
                                                0..N ResultDataset
```

`.dss`、目录/压缩包和受支持的 `.rssbundle` 只是模型导入介质；CSV、Excel、Parquet 是直接表输入。
无论来源如何，版本管理只面向规范、具名的 Parquet DataFrame 集合。成功导入后不再保留源文件，后续修改、
切换和运行都不重新读取 DSS 或 bundle。

单个 `.rss` 不再被当作模型，而是当前会话中的可执行脚本资源。脚本不会自动运行；用户必须在代码块卡片
上点击“运行”，系统才把脚本与当时的 active 模型版本绑定，创建一条运行记录并保存具名 Parquet 结果。

## 2. 已确认的产品边界

### 目标

- 所有正式模型版本只包含 DataFrame，不包含 `.dss`、`.rss`、bundle 文件树或服务器路径。
- 支持单 DSS、DSS 目录、压缩包、可模型化 RSSBundle，以及 CSV、Excel、Parquet 直接输入。
- 支持分多批形成草稿；只有用户点击“生成版本”才发布不可变版本并自动激活。
- 支持通过按钮把任一历史版本设为 active，并支持删除非 active 历史版本。
- 会话级脚本可由用户上传或 Agent 新增；以可折叠代码块显示，可展开、收起、复制全文和运行。
- 每次运行绑定确切模型版本和当次脚本快照；前端统一称“运行记录”。
- 单张结果 DataFrame 可删除；已有派生结果和比较不级联删除。
- 案例库只把案例模型导入可变草稿，不导入脚本、不运行研究、不直接生成版本。

### 非目标

- 把原始 DSS/RSSBundle 作为模型版本的一部分长期保存。
- 支持不能完整导出模型输入 DataFrame 的多 Solve、Harmonic、AutoAdd、RelCalc 等执行型 bundle。
- 单 `.rss` 模型、RSS 文本参数补丁或 bundle 直接运行。
- 脚本在线编辑、覆盖、版本历史、回退或单独删除。
- Agent 生成脚本后自动运行。
- 单条聊天消息删除或修改；整个会话仍可删除。
- 单独删除运行记录；运行记录只会随其模型版本或整个会话删除。
- V1/V2 数据迁移、旧 source kind、旧 `SimulationTask` API 或旧工具兼容。

### 反目标

- 不允许脚本通过 `import_*`、本地路径、URL 或未绑定文件绕过 active DataFrame 模型。
- 不把每批上传、每次表替换或未完成草稿制造成模型版本。
- 不把引擎执行结果误当成模型输入表。
- 不在删除来源结果时连带删除已经物化的派生结果。
- 不让浏览器或 LLM 传服务器路径、命令、SQL、任意 ECharts option 或 artifact 物理位置。

## 3. 方案选择

### 方案 A：DataFrame 单一模型边界（已选）

所有模型来源先进入 import/规范化阶段，产出可变草稿；用户确认后才冻结为 DataFrame 版本。脚本是独立的
会话资源，运行时与确切模型版本绑定。

该方案使模型修改、版本比较、脚本复用、运行重现和结果清理都有唯一语义；代价是 DSS/RSSBundle 导入
必须先完整物化 Parquet，不能保留 V2 的直接运行捷径。

### 方案 B：保留 DSS/RSSBundle/DataFrame 多来源版本（已排除）

继续允许版本直接引用源文件可减少一次导入物化，但同一“模型版本”会有三种运行方式，修改和删除语义
也不同；它违反“版本只管理 DataFrame 模型”和“成功导入后释放源文件”。

### 方案 C：每批导入立即生成不可变版本（已排除）

事件历史最完整，但分批上传会产生大量缺表、冲突或不可运行版本；它违反“用户确认后才生成版本”。

## 4. 领域对象与关系

```text
User 1──N Session 1──1 Workspace
                         │
                         ├──1 MutableDraft 1──N DraftDataFrame
                         ├──N ModelVersion 1──N ModelDataFrame
                         ├──N ScriptResource
                         └──N SimulationRun 1──N ResultDataset
                                  │                    │
                                  └── model_version    └──N DatasetLineage
                                  └── script snapshot
```

- **MutableDraft**：会话内唯一的可变 DataFrame 集合，有递增 `revision`；不是模型版本。
- **ModelVersion**：用户确认后冻结的完整 DataFrame 集合；内容不可修改，可激活或删除。
- **ScriptResource**：当前会话内独立脚本；每次上传或 Agent 生成都是新资源，不形成版本关系。
- **SimulationRun**：用户点击脚本“运行”产生的一次执行记录，绑定确切版本和任务内脚本快照。
- **ResultDataset**：运行或分析产生的一张具名 Parquet DataFrame；可以单张删除。
- **DatasetLineage**：结果间血缘；来源删除后仍保留足以解释关系的墓碑信息。

V3 UI 和公共 DTO 统一使用“运行记录”；内部表和类型使用 `simulation_runs`/`SimulationRun`，不再使用
`task`/`SimulationTask` 命名。

## 5. 模型导入与规范化

### 5.1 输入分类

| 输入 | V3 行为 |
|---|---|
| 单个 `.dss` | 使用静态 import 形成模型 DataFrame 集合 |
| DSS 目录或 ZIP | 安全重建目录和相对依赖后 import；支持 `Redirect/Compile` |
| `.rssbundle` 目录或 ZIP | 校验 manifest/hash，只接受可完整投影为模型输入表的 bundle |
| CSV/Parquet | 识别标准表名和 schema 后写入草稿 |
| Excel | 每个非空 sheet 作为候选表，规范化为 Parquet 后写入草稿 |
| 单个 `.rss` | 创建 `ScriptResource`，不进入模型导入 |

所有归档继续拒绝绝对路径、`..`、NUL、重复规范路径、符号链接、条目/解压大小超限和压缩炸弹。所有
`tensoreval` 调用仍只能通过 `backend/src/adapters/`；application service 只传受控 staging tree 和
结构化 import spec。

### 5.2 DSS import 合同

静态声明式 DSS 使用 `import_dss`，产生固定 21 张表：20 张网络表和 `para`。没有对象的可选表仍以合法
零行 Parquet 存入草稿，保持完整位置与 schema 合同。

具有单个受支持 `Solve` 的执行型输入可以使用 `import_dss_execution`，其网络、solution、shape、控制、
保护、meter、fault 和 `para` 表进入模型草稿；`diagnostics` 与 `events` 只登记为本次摄取诊断，不作为
模型表。

RSSBundle 根据已校验 manifest 的 source entry 和 study 分类选择上述 import 方法。下列输入直接返回
稳定错误 `model_import_unsupported`，不创建或修改草稿：

- 多个 Solve epoch；
- 只能通过 `ds_*_dss_plan` 执行、不能导出完整输入表的生命周期；
- Harmonic/HarmonicT、AutoAdd、RelCalc 等只产生研究结果的 specialist bundle；
- 缺少源入口或 import 合同与当前 engine catalog 不一致的 bundle。

系统不执行 bundle 的 `main.rss` 来“猜”模型表，也不把其结果 DataFrame 混入模型。

### 5.3 直接 DataFrame 输入

- 表类型首先由标准 schema catalog 识别；文件名和 sheet 名只作展示提示。
- 多个候选匹配、未知 schema、必需列缺失或 dtype 冲突进入待确认状态，不静默选择。
- 同一批次内两个文件最终映射到同一标准表名时，批次保持待确认，直到用户明确取舍。
- 一旦映射确定，应用批次时同名表按整表替换，其他草稿表保留；不做行级拼接。
- CSV/Excel 解析结果统一写 Snappy Parquet；输入 Parquet 也要重读并按 catalog 验证，不能因扩展名跳过。

### 5.4 原始来源生命周期

上传字节先进入 staging artifact，仅在 import/解析期间存在。成功生成、校验并原子登记全部草稿 Parquet
后，释放原始 DSS 文件树、bundle、CSV、Excel 或源 Parquet 的 artifact 引用。模型版本不依赖这些文件。

长期只记录：来源类型、展示名、原始整体 SHA-256、使用的 import 方法、engine contract、生成表清单和
结构化诊断摘要。失败时不更新草稿并清理 staging；用户通过重新上传重试。服务端路径和源文件内容不进入
LLM prompt 或公共 DTO。

### 5.5 异步导入批次与待确认状态

模型导入是持久异步作业，而不是在 multipart HTTP 请求内等待引擎完成。`ModelImport` 至少具有
`accepted | processing | awaiting_resolution | ready_to_apply | applied | failed | discarded` 状态，并通过
SSE 发布完整阶段快照。上传成功返回 202 和稳定 import ID；刷新后可恢复进度和待确认项。

待确认的候选 Parquet 不直接写入草稿，而属于独立 import batch。批次记录开始时看到的可空
`base_draft_id + base_draft_revision`、候选表、映射证据和合并预览。用户通过固定工作区 UI 选择入口、
表映射或取舍重复表；Agent 可以解释诊断，但不能替用户作最终选择。

staging 只保留继续处理所需的最小资源：DSS 尚未选择入口时保留安全解包树；CSV/Excel 已完整解析成候选
Parquet 后可释放原始字节。候选 artifact 在批次 pending 期间计入当前用户配额。应用、失败或用户明确
“放弃导入”后释放不再被草稿引用的 staging artifact。

应用批次必须携带当前可空 `expectedDraftId + expectedDraftRevision`；空工作区应用第一批时原子创建草稿。
如果草稿在导入期间已变化，返回
`409 draft_revision_conflict`，重新基于最新草稿计算新增/覆盖预览并要求用户再次确认；不能把旧预览自动
套到新草稿。服务重启把 processing 批次收敛为 `failed/service_restarted` 并清理临时进程目录，已持久化
候选和诊断仍可展示，用户重新上传或重新发起 import，不在后台猜测续跑。

### 5.6 DSS 入口与 import profile 选择

RSSBundle 必须使用 manifest 中已校验的 source entry，不允许另选入口。普通 DSS 目录/ZIP 由 adapter
解析 include 图并生成受控入口候选：

- 只有一个根入口时自动选择；
- 多个互不引用的根入口时进入 `awaiting_resolution/entry_selection`，前端显示虚拟相对名称，但提交稳定
  candidate ID，不提交服务器路径；
- 没有可用根入口、依赖缺失或 include 图非法时失败关闭。

入口选定后由引擎分类器决定 `import_dss` 或 `import_dss_execution`，用户不能强行指定 profile。纯静态
模型进入前者；可证明为受支持单 Solve ExecutionStudy 的入口进入后者；其他生命周期统一返回
`model_import_unsupported`。应用层不根据文件名、文本关键字或 UI 选择复制分类规则。

## 6. 可变草稿

### 6.1 草稿语义

每个 workspace 最多一份草稿。草稿记录：

- `draft_id`：每次从空或 active 创建草稿时生成的新稳定身份；
- `revision`：每次成功变更递增；
- `base_version_id`：从哪个 active 版本派生，可为空；
- 具名 DataFrame → Parquet artifact、schema、行数、内容 hash 的映射；
- 缺失表、映射歧义、dtype/引用冲突和 import diagnostics；
- 最近一次变更时间。

首次导入从空草稿开始。已有 active 时，首次修改或新导入从 active 的 DataFrame 引用复制出草稿；复制只
增加 artifact 引用，不复制文件。上传、Agent 修改、表替换/移除和案例导入都修改草稿本身，不产生版本。

revision 只在同一 `draft_id` 内单调递增；发布、放弃或切换版本清空草稿后，下一个草稿使用新 draft ID，
revision 从 1 开始。所有草稿写入携带 `expectedDraftId + expectedRevision`。任一不匹配返回
`409 draft_revision_conflict`，前端重新加载后再提交，避免旧草稿请求误命中新草稿，也避免多标签页、
上传与 Agent 同时修改造成静默覆盖。

### 6.2 案例合并

“应用案例”是固定 UI 动作，并走与手动导入相同的 adapter、schema 校验和 staging 事务。应用前显示：

- 将新增的标准表名；
- 将整表覆盖的同名表及其当前/案例行数；
- import warnings 和合并后的完整性结论。

用户确认应用后，同名表由案例表整表覆盖，案例新增表加入，案例没有的现有草稿表保留，不做行级拼接。
案例只更新草稿；不导入案例脚本、不执行 study、不创建运行记录或结果。

案例目录只发布能由当前 engine contract 完整生成模型表的项目。V2 的根 RSS 案例和不能模型化的执行型
bundle 不进入 V3 模型案例库。

## 7. 生成、激活与删除模型版本

### 7.1 生成版本

模型工作区提供明确的“生成版本”按钮，这是 V3 对领域写操作主要走自然语言规则的显式例外。按钮只有在
草稿无待确认映射、满足完整性与跨表引用校验时可用。

请求携带 `expectedDraftId`、`expectedDraftRevision` 和前端生成的 `clientPublishId`。后端在单一事务中：

1. 再次校验 revision、所有 Parquet hash/schema 和完整模型合同；
2. 计算与文件顺序无关的集合 content hash；
3. 创建不可变 `ModelVersion` 和全部 `ModelDataFrame` 关系；
4. 将新版本设为 workspace active；
5. 清空草稿及其临时引用。

发布回执以 `(draft_id, revision, client_publish_id)` 唯一，网络重试返回同一结果。若内容 hash 与当前未删除
历史版本完全相同，则直接激活该既有版本并清空草稿，不制造重复版本；若相同内容的旧版本已经删除，允许
创建新的版本 ID。数据库对未删除版本使用 workspace + content hash 的部分唯一约束，而不是让墓碑阻止
重新导入。任一步失败时 active 和草稿保持原状。

### 7.2 后续草稿与版本切换

发布后再次上传或修改时，以当前 active 版本作为新草稿基线。历史版本列表提供“设为当前版本”按钮；
切换只移动 active 指针，不创建新版本，不影响已有运行记录。

若存在未确认草稿，切换前显示明确的放弃草稿确认；确认后释放草稿专属引用，再切换 active。取消则不做
任何更改。

### 7.3 删除历史版本

只允许删除非 active 版本。删除前 API 返回影响摘要，包括将删除的运行记录、尚存原始结果和预计释放
字节；前端二次确认。若存在 queued/running 的运行记录，返回 `409 model_version_busy`，完成或失败后
才能删除。

busy 检查还覆盖正在消费该版本结果的 queued/running 分析、比较或视图物化。任一活动消费者存在时均
拒绝删除，不取消后台工作，也不依赖已经解绑的输入继续执行。影响预览与最终删除使用同一影响 token；
预览后新增活动消费者或影响集合变化会使删除返回 409，要求重新预览。

确认后级联隐藏并释放：

- 该模型版本的 DataFrame artifact 引用；
- 绑定该版本的所有运行记录；
- 这些运行记录的任务内脚本快照；
- 这些运行记录仍存在的原始结果 DataFrame。

已经物化的派生分析、视图数据和比较结果不继续级联。为保持其解释能力，版本、运行记录和输入结果留下
只读墓碑（稳定 ID、名称/hash、删除时间和必要 schema 摘要），但不再出现在普通版本、运行和结果列表，
也不能恢复或重新下载。

## 8. 会话级脚本资源

### 8.1 创建和生命周期

脚本只有两种来源：

- 用户在当前会话上传单个 `.rss`；
- Agent 根据对话生成并调用 `create_script` 工具保存。

每次创建都是新的 `ScriptResource`，字段包括 session、显示名、来源 `upload|agent`、完整内容 artifact、
SHA-256、静态校验状态、所需标准 input aliases、声明输出、诊断、创建时间，以及 Agent 生成时的
bundle/skill refs。脚本之间没有 parent、版本号或替换关系。

不提供在线编辑、覆盖、回退、版本历史或单独删除。用户复制全文、在外部修改后重新上传，即得到新的独立
脚本资源。删除整个会话时，脚本及其无其他引用的 artifact 一并释放。

### 8.2 脚本约束与校验

脚本运行时只允许引用 active 模型版本按标准表名注入的 DataFrame，以及脚本内纯计算产生的变量。校验
必须拒绝：

- `import_dss*`、`import_bmopf*` 等重新导入模型的函数；
- 本地路径、URL、远程读取和任意外部文件依赖；
- 未绑定 DataFrame alias；
- 绝对路径、命令、SQL 或越出任务私有树的访问；
- 无具名 DataFrame 输出、重复输出名、输出数量/大小超合同。

校验分两层：

1. 创建时执行与模型无关的静态校验，验证语法、函数/catalog、外部依赖限制，并提取所需标准 aliases 和
   声明输出；即使当前没有 active 模型也能保存脚本。
2. 面向当前 active 模型实时计算 run eligibility，检查所需 aliases、schema 和 engine contract。active
   切换不会改写脚本记录，只改变卡片上的兼容状态；真正创建 run 的事务必须再次执行相同兼容校验。

校验通过 adapter 的预检能力完成；不能在 handler 或工具中复制一套字符串黑名单作为事实来源。静态校验
失败的脚本仍可展示和复制，但“运行”禁用并显示诊断；静态有效但与当前模型不兼容时显示缺失/冲突表。

### 8.3 聊天代码块卡片

脚本通过持久 `script` 消息进入聊天流，消息只携带 script ID、名称、来源、状态和有界开头预览；完整脚本
由独立受控 API 按需读取，避免大文本常驻全局聊天 state。

卡片行为：

- 顶栏显示 `RSS`、脚本文件名、来源和校验状态；
- 短脚本完整显示；长脚本默认只显示开头并限制高度；
- 提供“展开全文/收起”；
- “复制”始终复制完整脚本，而不是当前可见片段；
- “运行”绑定当前 active 模型；无 active、脚本无效或已有重复提交时禁用并解释原因；
- 不提供编辑或删除按钮。

折叠阈值和具体像素高度是实施者可调 UI 常量，但测试应覆盖阈值两侧、复制全文和窄屏行为。

## 9. 显式运行与运行记录

### 9.1 唯一启动入口

Agent 创建脚本后不得在同一轮或后续工具调用中自动执行。V3 删除 Agent 的 `run_study` 执行权限；运行
只能由用户点击脚本卡片的固定“运行”按钮触发。自然语言“帮我运行”可以让 Agent 指向对应脚本卡片，
但不能绕过按钮。

运行请求携带 script ID 和界面当前看到的 `modelVersionId`。后端必须在事务中确认：

- script 和版本属于当前用户、当前 session；
- 该版本仍是 workspace active；
- 脚本校验成功；
- 客户端幂等 key 尚未创建运行记录。

active 已变化时返回 `409 active_model_changed`，前端刷新并要求用户再次点击，不能静默换用其他版本。

### 9.2 任务内冻结

每次点击创建新的 `SimulationRun`，即使脚本与模型完全相同。运行记录固化：

- model version ID 和完整模型 content hash；
- script resource ID、实际脚本内容 artifact/hash；
- engine contract、agent bundle/skill refs（若脚本由 Agent 生成）；
- 状态 `queued|running|succeeded|failed`、阶段、时间和结构化错误；
- 用户点击产生的 client run ID。

脚本资源没有版本管理，但任务内脚本快照保证运行期间和历史记录不受后续会话资源变化影响。该快照不是
可浏览或可回退的脚本版本。

### 9.3 引擎执行

application service 把版本内每张 Parquet 物化到任务私有输入树，用稳定标准表名形成 `NamedInput`；脚本
物化为受控相对路径。adapter 显式执行：

```text
tensoreval --dataframe \
  --input <table>=<parquet> ... \
  --result-dir <staging> \
  --output-format parquet \
  <script.rss>
```

命令只用参数数组，不经过 shell。成功时枚举全部具名 Parquet，验证名称、schema、Snappy 编码、行数、
文件数/字节预算和路径边界，再以单一事务登记结果；失败不发布部分结果。

### 9.4 运行记录生命周期

前端列表显示脚本名、模型版本、状态、创建/完成时间和仍存在的结果数量。用户不能直接删除运行记录。
逐张删除全部结果后运行记录仍保留；删除其模型版本或整个会话时，运行记录按第 7.3 节级联清理。

## 10. 结果 DataFrame 删除

每张 `ResultDataset` 独立提供“删除结果”操作。删除前展示名称、来源运行/分析和是否已有派生引用；用户
确认后：

1. 将 dataset 标记为 deleted，并记录 `deleted_at_ms`；
2. 解除其 Parquet artifact 引用并按引用计数释放配额/物理文件；
3. 从普通结果列表、表格、图表和下载入口隐藏；
4. 禁止把它作为新的分析、比较或视图输入。

删除不级联到已经物化的派生分析、图表数据或比较结果。这些资源拥有自己的 Parquet artifact，继续可读。
为满足这一语义，保存 DatasetView 时必须同时物化其有界投影/聚合数据为独立 Parquet；只保存依赖来源
dataset 的 VisualizationSpec 不足以在来源删除后继续展示。
`ResultDataset` 墓碑保留稳定 ID、名称、schema/hash 摘要、原生产者和删除时间，派生血缘显示“来源结果已
删除”。直接访问被删除结果的 rows/download 返回 404；血缘 DTO 只返回墓碑摘要，不返回原 artifact。

若结果正被 queued/running 的分析使用，删除返回 `409 result_busy`；运行完成后可删除。没有“删除整个
运行记录及其全部结果”按钮。

## 11. 案例库

V3 案例快照只收录能按当前 engine contract 生成完整模型 DataFrame 集合的案例。搜索和详情显示来源、
适用 import 方法、预期表集合、验证状态和诊断摘要，但不把服务器路径暴露给前端。

“应用案例”按钮执行：预检 → import → Parquet 校验 → 合并预览 → 用户确认 → 原子更新草稿。它与手动
导入共用同一 application service 和 adapter；不能复制一条案例专用模型路径。

应用案例不创建版本、脚本、运行记录或结果。V2 的根 RSS 只可在未来作为脚本示例另行设计，本版本不在
模型案例库展示。

## 12. 数据库设计方向

V3 从空库重写 initial migration。具体列名在实施计划中落到 Diesel schema，但必须表达以下约束：

- `workspace_drafts(id, workspace_id UNIQUE, base_version_id, revision, diagnostics_json, updated_at_ms)`；
- `model_imports(session_id, base_draft_id, base_draft_revision, status, stages_json, diagnostics_json, ...)` 及其
  待确认 candidate/selection 记录；
- `draft_dataframes(draft_id, table_name, artifact_id, schema_json, row_count, content_hash)`，表名唯一；
- `model_versions(workspace_id, content_hash, created_at_ms, deleted_at_ms)`；
- `model_dataframes(version_id, table_name, artifact_id, schema_json, row_count, content_hash)`，发布后禁止更新；
- `script_resources(session_id, artifact_id, name, source_kind, content_hash, validation_status, diagnostics_json, ...)`；
- `simulation_runs(session_id, model_version_id, script_resource_id, script_artifact_id, status, engine_contract, ...)`；
- `result_datasets(..., artifact_id NULL, deleted_at_ms NULL, tombstone_json, ...)`；
- `dataset_views` 关联独立的已物化 view dataset/artifact，使来源删除后仍可展示；
- 分析/比较输入保存 dataset ID 及生产时的名称/schema/hash 快照，使来源删除后仍可解释。

数据库触发器或 repository 约束保证：已发布模型表不可更新；active 只能指向同 workspace 的未删除版本；
运行只能绑定同 session 版本和脚本；一个标准表名在同一草稿/版本内唯一；deleted dataset 不再拥有
artifact 引用。

## 13. HTTP/SSE 合同方向

最终路径可在实施计划中按现有 router 风格细化，但公共行为至少包含：

| 方法 | 路径 | 作用 |
|---|---|---|
| POST | `/api/sessions/{id}/model-imports` | 上传 DSS/bundle/DataFrame，创建异步 import |
| GET | `/api/model-imports/{id}` | 读取 import 阶段、候选、诊断和最新合并预览 |
| POST | `/api/model-imports/{id}/resolve` | 提交入口/映射选择并以 expected draft 应用 |
| DELETE | `/api/model-imports/{id}` | 放弃 pending import 并释放 staging |
| POST | `/api/sessions/{id}/draft/example` | 为案例创建同类异步 import |
| POST | `/api/sessions/{id}/versions` | 以 expected revision 生成并激活版本 |
| POST | `/api/versions/{id}/activate` | 激活历史版本，必要时确认放弃草稿 |
| GET/DELETE | `/api/versions/{id}` | 读取版本；预览影响并删除非 active 版本 |
| GET/POST | `/api/sessions/{id}/scripts` | 列表/上传单个 RSS 脚本 |
| GET | `/api/scripts/{id}` | 脚本元数据和按需完整内容 |
| POST | `/api/scripts/{id}/runs` | 用户按钮创建运行记录 |
| GET | `/api/sessions/{id}/runs` | 会话运行历史 |
| GET | `/api/runs/{id}` | 运行详情 |
| GET | `/api/runs/{id}/datasets` | 运行的未删除结果 |
| DELETE | `/api/datasets/{id}` | 删除单张结果并保留墓碑血缘 |

删除版本需要明确的影响预览与确认参数，不能让普通 DELETE 在未知影响下立即执行。所有路由从认证 Cookie
取得用户身份，并通过 session join 校验归属；跨用户、墓碑资源的普通读取和不存在资源统一返回 404。

SSE 至少新增 `script` 消息，并将 V2 `progress/result` 中的 task 字段改为 run 字段。完整脚本不进入 SSE；
结果删除和版本删除发送持久状态事件，使其他标签页能移除对应 UI。消息先提交 SQLite 再发布事件，继续
支持 `Last-Event-ID` 和 resync。

## 14. Agent 与工具边界

V3 Agent 可以：

- 检查草稿和版本 DataFrame schema；
- 对可变草稿执行受控表修改、替换和移除；
- 生成新的 RSS 并通过 `create_script` 保存为会话脚本；
- 查询既有运行和未删除结果，生成派生分析或比较；
- 搜索引擎文档和可模型化案例。

V3 Agent 不可以：

- 发布/激活/删除模型版本；这些由明确 UI 按钮完成；
- 运行脚本；`run_study` 从 Agent 工具清单移除；
- 删除结果或运行记录；
- 把 DSS/RSSBundle 作为版本输入；
- 读取脚本/模型的服务器路径或绕过 adapter。

工具、system prompt、skill、manifest、功能目录和双向枚举测试必须同步更新。`model-intake` 改为草稿
语义，`rss-generation` 以创建脚本为终点，`study-planning` 不再授权执行工具。

## 15. 前端结构

- 左侧栏：会话列表与已批准的固定用户区；仍可删除整个会话，不能删除单条消息。
- 对话区：九类持久消息，新增 script 代码块卡片；上传路由按文件类型把 `.rss` 送到脚本入口，其他模型
  文件送到 model import。
- 右侧模型工作区：可变草稿的表、诊断和 revision；“生成版本”按钮；active 与历史版本列表；激活和
  删除历史版本按钮。
- 运行历史：使用“运行记录”术语，显示绑定版本/脚本、状态和结果数量，不提供删除按钮。
- 结果详情：分页表格、视图、下载和“删除结果”；删除确认不暗示会删除派生结果。

完整脚本采用惰性请求并可在卡片中展开，避免把大文本复制进 message reducer。复制必须以 API 返回的
完整内容为准；请求失败时不能退化为复制截断预览。

## 16. 并发、失败与恢复

- 多批上传通过 draft revision 串行化；失败批次不改变草稿。
- 发布版本、自动激活和清草稿为单一事务。
- 点击运行使用 client run ID 幂等；按钮立即进入提交中，成功 SSE 到达后归并同一 run。
- active 在点击与服务端创建 run 之间改变时失败关闭，不自动选择新版本。
- 服务重启把 queued/running run 收敛为 failed；已发布结果不受影响。
- 结果和版本删除先释放数据库可见性，再由幂等 GC 处理物理文件失败。
- 会话删除级联其草稿、版本、脚本、运行、结果和消息；全局 CAS 仅在所有引用归零后物理删除。
- 草稿恢复、脚本展开状态和结果删除状态在刷新后以服务端为准；展开/收起本身可以只保存在组件状态。

## 17. 验证与功能追溯

### 后端与 adapter

- `import_dss` 固定 21 表、零行表、schema、Parquet 编码和源释放。
- `import_dss_execution` 只把模型输入表写入草稿，diagnostics/events 不混入模型。
- 多 Solve 和 specialist bundle 返回 `model_import_unsupported`，草稿不变。
- 多入口 DSS 进入固定 UI 选择；引擎而非文件名决定 static/execution profile。
- DSS 目录/ZIP 依赖、路径攻击、压缩边界和失败清理。
- 异步 import 的阶段恢复、待确认 candidate、源最小保留、revision 冲突和重启收敛。
- CSV/Excel/Parquet 分批、映射冲突、整表替换和 revision 并发。
- 发布版本原子性、内容 hash 幂等、自动激活、历史激活和草稿放弃确认。
- 删除版本的影响预览、active/busy 拒绝、run/result 级联与派生墓碑保留。
- 脚本禁用 import/路径/URL，校验失败不可运行；Agent 不能执行。
- 脚本静态校验可无模型完成；active 切换更新兼容状态；创建 run 再做权威校验。
- 用户按钮运行的版本/脚本归属、active 竞态、幂等、失败原子性和多 Parquet 结果。
- 单 dataset 删除、配额释放、busy 拒绝、派生结果保留和墓碑血缘。

### 前端组件

- 草稿 revision、分批状态、生成版本按钮可用性和自动 active。
- 历史版本激活、草稿放弃确认、删除影响摘要和 busy 错误。
- script 卡片短/长边界、展开/收起、完整复制、无 active/无效/提交中禁用。
- Agent 生成脚本后不出现自动运行；点击一次只创建一个 run。
- “运行记录”术语、无删除入口、单结果删除和剩余数量更新。
- 案例覆盖预览、同名整表替换和未出现表保留。

### 浏览器全链路

1. DSS 目录 → import → 源释放 → 草稿 → 按钮生成并激活版本 → 上传脚本 → 展开/复制 → 点击运行 → 多结果。
2. 分批 CSV/Excel/Parquet → 冲突确认 → 草稿替换 → 发布版本 → 历史版本切换。
3. 静态/单 Solve RSSBundle 成功导入；多 Solve/specialist bundle 被拒且草稿不变。
4. 案例导入到已有草稿，同名表覆盖、其他表保留，且不自动发布或运行。
5. Agent 生成脚本只展示；用户点击后才运行，active 竞态失败关闭。
6. 删除单张结果后派生结果仍可读，原 rows/download 404，配额下降。
7. 删除历史版本后相关运行和原始结果消失，派生血缘显示墓碑。
8. 两个用户、两个会话之间模型、草稿、脚本、run、dataset、SSE 和删除操作完全隔离。

所有新增/删除的路由、`script` 消息、工具、skill、上传格式、错误码和 UI 行为必须同步更新：

- `docs/architecture.md`
- `docs/api-design.md`
- `docs/db-design.md`
- `docs/frontend-design.md`
- `docs/model-workspace-design.md`
- `docs/agent-prompt-skills-design.md`
- `docs/development-status.md`
- V3 deployment 文档
- `docs/integration-testing-design.md`
- `frontend/src/types/`
- `integration-tests/catalog/features.yaml`

功能目录必须关联真实行为测试；路由/工具/枚举测试不能替代 import、按钮发布、显式运行、删除和隔离旅程。

## 18. 发布与数据重置

V3 候选部署从空 SQLite 和空 DATA_DIR 启动，不迁移 V2 开发数据。initial migration、engine resource
manifest、Agent bundle 和前端公共类型一次性切换到 V3 合同。

启动 preflight 除既有 binary/catalog/docs hash 外，必须验证：

- `import_dss` 与 `import_dss_execution` 返回名/顺序/schema 合同；
- Parquet named input 和 result-dir 合同；
- 案例目录中每项都可生成模型表，不能只验证 bundle 可执行；
- Agent 工具清单不含自动运行能力；
- CSV 固定用户 seed 合同。

候选发布真实依赖旅程至少覆盖一次 DSS import、一次受支持 RSSBundle import、一次直接 DataFrame 草稿、
按钮发布、用户触发脚本运行和单结果删除；不能用 V2 已通过证据代替。

## 19. 留给实施计划的局部选择

以下不改变产品行为，可由实施者在计划中明确：

- 草稿表、版本表和墓碑表的具体 Diesel struct 拆分。
- 长脚本默认折叠的行数/高度常量和语法高亮库复用方式。
- 删除版本影响预览使用同一路由的 dry-run 参数还是独立 preview 子资源。
- 结果墓碑保留在原表还是拆到独立 lineage snapshot 表。
- import-only RSS 模板由 adapter 静态生成还是由 engine 增加专用结构化子命令；无论选择哪种，都必须
  固定在 engine contract 并只通过 adapter 调用。
