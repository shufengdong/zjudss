# 模型工作区、版本、结果分析、历史检索与仿真对比设计

> 状态：v2 已实现；本文保留完整产品语义和验收依据。
> 适用范围：单 `.rss`、ZIP 打包目录、`.rssbundle` 目录、分批上传的 DataFrame 模型
> 配套：后端分层见 `architecture.md`，HTTP/SSE 契约见 `api-design.md`，持久化见
> `db-design.md`，前端行为见 `frontend-design.md`，实现状态见 `development-status.md`。

## 1. 目标与成功条件

用户在一个会话中通过自然语言完成以下闭环：

1. 上传单个 `.rss`、打包模型目录，或分多次上传 CSV/Excel/Parquet DataFrame；
2. 查看系统识别出的模型表、依赖、歧义和缺失项；
3. 指示智能体执行潮流、短路、QSTS 或其他受支持分析；
4. 用自然语言查询任务结果；智能体按结果 DataFrame 的 schema 在必要时生成 RSS 脚本，产出可追溯的
   新 DataFrame，并以分页表格或多个语义图表展示；
5. 比较修改前后的结果；
6. 刷新或重新进入会话后恢复聊天、模型版本、仿真任务和比较记录。

成功条件是任一仿真任务都能追溯到不可变输入版本、完整脚本和输出 DataFrame；任一结果回答都能
追溯到用户消息、生成脚本、输入数据集及输出数据集；任一比较都能追溯到两个具体任务和比较脚本。
上传路径、临时目录或会被覆盖的文件不能作为历史事实来源。

## 2. 内核能力与边界

内核文档以 `../eig-rs/sparrowzz/dss/docs/` 为准。当前可用合同包括：

- `.dss` 严格转换、`.rss` 的 `tensoreval --df` 执行和自包含 `.rssbundle` 执行；
- 可重复的 `--input NAME=FILE`，支持 CSV、单工作表 `.xls/.xlsx` 和 Parquet；
- 固定位置的网络 DataFrame schema，以及潮流、QSTS、短路、谐波等 `ds_*` 入口；
- `--result-dir DIR --output-format parquet` 把终端命名 DataFrame 直接写为 Snappy 压缩的
  `DIR/NAME.parquet`；不传 `--output-format` 时默认 CSV。该参数已在 eig-rs 源码和单测中确认，项目
  `.env` 指向的运行时二进制仍需重编译/替换并通过启动探测。

浏览器上传的多工作表 Excel 由本系统先按工作表拆成规范化文件，再作为多个 `--input` 传给
内核。该预处理属于工作区服务；内核命令构造、执行和错误翻译仍只能位于
`backend/src/adapters/`。

服务部署不能依赖一个可任意变化的相邻 `../eig-rs` 工作树。候选发布 artifact 同时提供与目标
`tensoreval` 对应的只读资源根，包含合同固定的 catalog、文档、案例和案例依赖；manifest 记录 catalog
版本、eig-rs superproject、sparrowzz submodule、来源状态和内容 hash。服务启动时从
`TENSOREVAL_CWD` 校验并原子发布 `${DATA_DIR}/system/engine-resources/<snapshot-id>`，运行期间的
docs/catalog 查询只读该快照。案例导入仍从已校验的资源根复制完整依赖闭包到用户 artifact。
`rss_language_reference.md`、`rsdss_function_reference.md` 中允许 agent 使用的 DataFrame 语法提炼进
版本化 skill；任一资源或合同不匹配时服务启动失败。

Prompt 不打包成一份永久注入的长文。系统规则、按需领域技能、机器 catalog 和 Rust 工具合同分别管理；
需要生成脚本的 agent run 必须先加载对应技能，任务与分析运行保存实际 bundle/contract 版本。完整规则
见 `docs/agent-prompt-skills-design.md`。

## 3. 领域对象

```text
Session 1──1 Workspace 1──N Ingestion
                    │
                    └──N ModelVersion 1──N VersionArtifact N──1 Artifact
                              │
                              └──N SimulationTask 1──N ResultDataset

Session 1──N ResultAnalysisRun N──N ResultDataset（输入血缘）
ResultAnalysisRun 1──N ResultDataset（派生输出）
SimulationTask N──N SimulationTask（通过 TaskComparison，差值由 ResultAnalysisRun 生成）
```

- **Workspace**：会话内的模型工作区，持有 active/draft 两个版本指针。
- **Ingestion**：一次可幂等重试的上传批次，记录解包、推断和校验状态。
- **Artifact**：一个内容不可变的文件或 Excel 工作表；以 SHA-256 标识内容。
- **ModelVersion**：某一时刻完整的输入清单、DataFrame 绑定、参数补丁和校验结论。
- **SimulationTask**：在一个明确的模型版本上执行一个明确的 `study_spec`。
- **ResultDataset**：原始仿真或派生分析产生的一张不可变 DataFrame 文件及其中立 schema 元数据。
- **ResultAnalysisRun**：用户一次结果查询对应的 RSS 脚本执行，保存意图、脚本、输入别名、输出和错误。
- **TaskComparison**：两个历史任务的比较关系；实际数值差异由关联 ResultAnalysisRun 生成 DataFrame。

模型版本状态为 `collecting | ready | invalid`，清单、绑定、补丁和状态在发布后都不可修改。这里的
`collecting` 表示该不可变快照仍缺输入或等待用户确认，不表示后台正在原地更新它；后台处理过程属于
Ingestion，完成一个摄取或确认事务后再发布下一个版本。Workspace 的 `active_version_id`
指向默认仿真使用的已确认版本，`draft_version_id` 指向最新候选版本；新增文件、确认映射、修改参数
或脚本只推进 draft 并保留 `parent_version_id`。已有 active 时，draft 收集不完整或校验失败不会阻塞
active 上的查询与仿真。

## 4. 输入与摄取流程

### 4.1 支持的输入

| 输入 | 行为 |
|---|---|
| 单个 `.rss` | 保存并检查语法及本地依赖；缺依赖时保持 `collecting` |
| ZIP 目录 | 安全解包，保留相对目录；检测 `.rssbundle/manifest.json`、入口 RSS/DSS 和数据资产 |
| `.rssbundle` 目录 | 通过目录选择按相对路径多文件上传，或先压成 ZIP；后端重建目录并验证 manifest、entry、文件哈希和路径 |
| CSV/Parquet | 按表头推断 DataFrame 类型，再用 dtype 做 schema 校验 |
| Excel | 每个工作表作为独立候选 DataFrame；空工作表忽略并记录诊断 |

一次上传可以包含多个文件，也允许多次追加。摄取完成前只更新 Ingestion；每次成功摄取或人工确认
才一次性发布一个不可变 draft 版本，
因此中途状态同样可以追溯。初次上传没有 active 时，首个校验通过的 ready 版本可以直接激活；已有
active 后，新 draft 必须在 ready 且用户通过自然语言确认使用后才提升为 active。上传本身不自动
运行仿真；用户通过自然语言明确分析目标后才创建任务。

多文件批次允许局部失败：全部文件成功时批次为 `succeeded`，成功与失败项并存时为
`completed_with_errors`，批次级致命错误或没有可用项时为 `failed`。成功项仍可发布候选版本；失败项
保留独立错误并可用原 `clientFileId` 重试。若成功项不足以组成完整模型，候选版本为 `collecting`。

每个上传批次由客户端生成稳定 `clientBatchId`，每个原始文件生成稳定 `clientFileId`；重试复用
这些 ID，且相同 ID 的重试必须保持内容哈希不变，否则返回 `idempotency_conflict`。用户选择了修正后
的新文件时生成新 ID，再通过自然语言建立替换关系。服务端以会话/工作区和批次 ID 保证幂等，并用
文件 ID 关联乱序响应。ZIP 解包文件和 Excel
拆分工作表是原始文件的派生 artifact，不另占 clientFileId，通过来源 artifact 追溯。

### 4.2 传错、替换、移除与回退

用户文件不原地覆盖，重新上传也不会改写旧 artifact：

1. 同一 `clientBatchId + clientFileId` 的重试按请求幂等处理；内容哈希相同只复用底层 blob，不合并
   不同 clientFileId、相对路径或角色的逻辑输入，是否产生新版本由清单是否变化决定；
2. 内容不同的新文件先作为新 artifact 摄取；
3. 用户明确“用新文件替换某个输入”时，新版本排除旧输入并加入新输入；
4. 用户明确“删除/忽略某个输入”时，新版本只排除该输入，blob 在仍被历史版本引用时保留；
5. 用户只上传、未说明替换关系，而 active/draft 已有同类型表时，新 draft 保持 `collecting` 并要求
   确认“替换、并存还是忽略”；active 继续可运行，禁止依据文件名或上传先后自动覆盖；
6. 用户确认使用一个 ready draft 时，原子更新 active 指针并清空 draft 指针；
7. 用户要求撤销时，把 active 指针重新指向指定历史版本，并记录一条消息；已完成任务仍绑定原版本。

若错误文件已运行过仿真，替换后重跑会产生绑定新版本的新任务，旧任务和结果保持可查询，二者可
直接比较。ZIP、RSS 和 `.rssbundle` 的入口文件也遵循相同规则，替换目标使用版本输入 ID，不使用
可能重复的文件名。

### 4.3 安全与资源限制

- ZIP 拒绝绝对路径、`..`、符号链接和解包后越界路径；
- 同时限制单文件大小、文件数、压缩包大小、解包总大小和压缩比；
- 用户文件不作为可执行程序运行，输出文件不能覆盖输入 blob；
- 自动生成的结果分析脚本禁止 `#include_url/import_df_url` 和未登记文件访问，只能消费应用层按
  dataset ID 映射的 `--input`；模型仿真脚本若使用远程输入，必须保存 URL 和内容哈希或明确标记为
  非完全可复现；
- 面向用户只返回稳定错误码与可读摘要，原始 stdout/stderr 进入受控诊断文件。

### 4.4 历史文件配额与保留

首版默认给每个存储归属主体保存最多 **20,000 个唯一 artifact、20 GiB 逻辑容量**。两个限制任一达到
即拒绝会增加唯一 artifact 数或逻辑字节的提交。v2 的 `AUTH_MODE=instance` 按整个可信私有实例计费；
`AUTH_MODE=trusted_proxy` 按认证得到的 owner/tenant 计费，不能共用实例总额度。默认值可由
`OWNER_ARTIFACT_LIMIT`（20,000）和 `OWNER_STORAGE_LIMIT_BYTES`（21,474,836,480）配置，并允许部署
层按 owner 覆盖。

计费口径如下：

- 用户上传、解包/Excel 规范化产物、模型与结果脚本、仿真/分析 Parquet、报告和保留的诊断均计入；
- “20,000 个文件”指上述持久化 artifact，不是会话、聊天消息、模型版本、任务或视图等数据库记录数；
  这些记录关联的实际文件仍按 artifact 计费；
- staging、失败后待清理的临时文件不计入历史配额，但受独立的全局临时空间上限约束；
- 同一 owner 对同一 artifact 内容无论被多少版本、任务或路径引用，只计 1 个文件和 1 份字节；界面
  仍保留全部逻辑引用。跨 owner 复用同一物理 blob 时，各 owner 分别计费，避免配额和存在性泄漏；
- 达到 80% 时只告警；达到硬上限后，历史查询、表格、图表、下载、纯视图创建和对已有 artifact 的
  重复引用仍可用。上传、仿真及结果分析若会增加唯一文件或字节，则返回 `storage_quota_exceeded`。
  结果大小事先未知时先写隔离 staging，提交 artifact 的事务内原子检查增量；超限则不发布
  dataset/result 消息并清理 staging；
- 首版**不按天数自动删除**历史文件，也不采用“只保留最近 N 个任务”。用户删除会话后，GC 在所有
  历史引用消失时释放计费和物理 blob。这样不会静默破坏版本回退、结果查询或前后对比。

### 4.5 从内置案例创建模型

服务启动时从 `${TENSOREVAL_CWD}/sparrowzz/dss` 建立只读引擎资源快照和案例目录。当前来源包含根目录
16 个 `.rss` 与 `rssbundle/` 下 57 个已发布 `.rssbundle`；具体数量属于当前合同事实，升级后以新快照
为准。根目录 RSS 必须把 `import_dss/import_bmopf`、`ds_*_plan` 和 `#include` 引用解析成完整依赖
闭包；依赖可位于 `models/`、`psa/tests/bmopf` 或 `sparrowzz/rspower/lib`，但不得逃逸
`TENSOREVAL_CWD`。bundle 必须验证 `manifest.json`、entry、文件 hash 和 tree hash。

用户可用自然语言搜索和导入案例，例如“导入 IEEE13 短路案例”“给我一个能修改负荷曲线的 QSTS
例子”。多个候选都合理时 agent 先返回标题、分析类型、solver、规模和运行要求供选择。用户和模型只
使用稳定 `example_id`，不得传入 `TENSOREVAL_CWD` 路径。

导入时后端复制案例及依赖到当前 scope 的 artifact 存储，并通过普通 Ingestion 发布不可变版本；不在
共享引擎目录上修改文件。案例的入口和依赖以相对 `TENSOREVAL_CWD` 的虚拟路径进入版本 manifest；运行
时物化为任务私有 input tree 并以其根作为 cwd，不继续读取共享源目录。空工作区的首个 ready 案例可
自动 active，已有 active 时导入结果只成为 draft。
导入只创建模型版本，不自动启动仿真；用户可以先继续用自然语言修改参数，也可以明确要求按原案例运行。
版本 manifest 保存 example ID、源 tree hash、engine resource snapshot 和来源信息。用户后续通过自然
语言修改参数时产生子版本，源案例和已完成任务均保持不变。

## 5. 基于表头的 DataFrame 类型推断

文件名和工作表名只用于展示，不决定 DataFrame 类型。后端加载与当前内核版本匹配的
`dataframes.json` catalog，只用规范化后的表头（列名集合）推断候选类型。dtype 用于推断后的
schema 校验和诊断，不参与候选类型排名。

每个候选的推断记录：

```text
artifact_id, sheet_name, table_kind, confidence,
candidate_scores,
matched_required_columns, matched_optional_columns,
missing_required_columns, type_conflicts,
resolution = pending | inferred | user_confirmed | rejected
```

推断顺序：

1. 规范化 BOM、首尾空白和大小写，但保留原始列名供展示；
2. 缺少必需列的 schema 不能判为高置信度；
3. 必需列覆盖率优先，其次比较可选列和稳定键列名；
4. 唯一高置信候选可自动绑定；
5. 前两名得分接近或无合格候选时标记 `ambiguous/unknown`，绑定 resolution 为 `pending` 并保存候选分数；
6. 同一版本出现多张同类型表时不自动拼接或覆盖，由智能体说明差异并请用户确认。

用户确认只改变派生版本中的绑定，不改原文件。执行前还要按 schema 验证必需列、dtype、单位、
稳定键唯一性和跨表引用。

## 6. 不可变版本与模型修改

`ModelVersion.manifest` 至少包含：

- 版本的全部 artifact、相对路径、文件角色和 SHA-256；
- Excel 工作表到规范化 artifact 的来源关系；
- DataFrame 类型绑定及推断证据；
- RSS/`.rssbundle` 入口和依赖检查结果；
- 从父版本应用的结构化补丁；
- catalog/适配器合同版本和校验诊断。

模型修改使用结构化补丁，不让 LLM 按自由文本直接覆盖文件：

```json
{
  "tableKind": "load",
  "rowKey": {"name": "load_1"},
  "changes": {"p": {"from": 100000, "to": 120000, "unit": "W"}}
}
```

应用前必须检查父版本、行唯一性、旧值、字段类型和单位。成功后生成新 artifact 和新版本；
任一检查失败时不产生半成品。RSS 修改同时保存原脚本、补丁和新脚本，并重新做语法与依赖检查。

从案例导入的版本也是普通不可变基线。用户说“把负荷提高 20%”“把故障母线改为 632”“把时步改为
15 分钟”时，agent 先读取该版本可编辑的模型表、脚本参数和 study spec，再生成同样的结构化补丁；
修改后的 RSS/DataFrame/DSS 产物进入子版本，绝不写回案例文件。无法由当前 catalog/解析器唯一定位的
执行计划命令或文本位置必须返回 `invalid_model_patch` 并说明缺口，不能用字符串替换猜测修改。

## 7. 智能体工具与适配器边界

建议提供以下领域工具：

| 工具 | 职责 |
|---|---|
| `inspect_workspace` | 返回 active/draft、输入、推断、缺失项和校验状态 |
| `search_engine_docs` | 在启动时建立的文档索引中查找相关 section，只返回受限片段和稳定 ID |
| `read_engine_doc_section` | 按 doc/section ID 读取一个受限文档片段，不接受文件路径 |
| `search_examples` | 按名称、分析类型、solver、标签和运行能力检索内置案例 |
| `inspect_example` | 返回案例说明、入口类型、文件树摘要、验证状态和资源要求 |
| `import_example` | 按 example ID、资源快照和预期基线把已验证案例复制为当前工作区的新模型版本 |
| `confirm_dataframe_mapping` | 固化用户确认的表类型，派生新版本 |
| `replace_model_input` | 用新上传的 artifact 替换 draft/active 基线中的指定 input_id |
| `remove_model_input` | 从新版本中排除传错或不再需要的输入 |
| `activate_model_version` | 激活 ready draft 或回退到已有历史版本，不改写版本内容 |
| `modify_model` | 校验并应用结构化模型补丁，派生新版本 |
| `run_study` | 在指定版本上创建并执行分析任务 |
| `list_result_datasets` | 返回任务或分析运行的 DataFrame 名称、列、dtype、单位、行数和稳定 ID |
| `run_result_analysis` | 接收生成的 RSS 脚本和 dataset ID→input alias 映射，运行并持久化派生 DataFrame |
| `create_dataset_view` | 对已有 dataset 校验并追加版本化 `VisualizationSpec`；纯换图不调用内核 |
| `compare_simulations` | 创建两个具体任务的比较关系，并通过结果分析运行生成差值 DataFrame |
| `list_simulation_history` | 解析“上次”“修改前”等历史指代 |
| `search_chat_history` | 按自然语言关键词、日期和任务条件检索可访问的历史会话 |
| `read_chat_context` | 围绕命中消息读取受限窗口，避免整段历史进入模型上下文 |
| `get_storage_usage` | 返回当前实例或 owner 的 artifact 数量、逻辑字节、限制和告警状态 |

工具参数使用 `workspace_id/model_version_id/input_id/task_id`，不接受 LLM 生成的任意绝对路径。
替换、移除和确认映射使用版本内 `input_id`，不能使用可能重复的文件名或全局 artifact ID。所有会
改变 draft 的工具还必须携带 `expected_draft_version_id`（无 draft 时使用 active 版本 ID 作为基线；
仅当二者都不存在时允许为 null）。`import_example` 还必须携带用户选择时看到的
`engine_resource_snapshot`，防止服务升级后把旧案例选择静默解析成新内容。
应用层按 workspace 串行化变更，并在事务中校验基线；不一致时返回 `version_conflict`，智能体重新
检查工作区后再执行，禁止把并发上传或修改静默覆盖。
`run_study` 和 `run_result_analysis` 都会触发内核计算，并且只能调用适配器：

```text
单 RSS       → tensoreval --df --result-dir <task-output> --output-format parquet <script>
RSS bundle   → tensoreval --result-dir <task-output> --output-format parquet <bundle>
DataFrame 集 → tensoreval --df --result-dir <task-output> --output-format parquet --input <name>=<normalized-file>... <generated-script>
结果查询     → tensoreval --df --result-dir <analysis-output> --output-format parquet --input <alias>=<dataset-file>... <generated-script>
```

DataFrame 模型的仿真 RSS 由 agent 根据用户意图、已确认绑定和版本化的内核文档生成，应用层校验后
形成 `RunSpec`。结果分析 RSS 由 agent 根据用户问题和数据集 schema 生成。两类工具都只传稳定 ID、
脚本文本和结构化声明；实际路径解析、命令构造、输出目录与执行由适配器实现。工具层禁止直接启动
`tensoreval` 或调用原始内核 API。

v2 不直接在共享 `TENSOREVAL_CWD` 上执行用户版本。application 先把版本 manifest 引用的 artifact
物化成任务私有 input tree，适配器只接受已校验的 tree 根并把它设为子进程 cwd；脚本中的相对路径因此
仍可工作，同时历史任务不受共享示例升级或删除影响。

## 8. 仿真、查询与比较

`study_type` 不作为封闭枚举限制内核能力；它是便于筛选的规范标签，首批 UI 识别
`power_flow/short_circuit/qsts`，未知值仍可保存和展示。`study_spec_json` 保存用户意图、求解器、
故障条件、时间设置、期望结果和单位明确的参数。任务同时绑定 `model_version_id` 和实际执行的 RSS
script artifact，共同构成可复现输入快照。

仿真完成后，适配器枚举 `--result-dir` 里的每个命名 Parquet，校验 footer 并读取 schema/行数和 catalog
中已知单位，逐个登记为 ResultDataset。数据库不提取或预设收敛、网损、电压、越限等统计字段。
v1 原型未投入使用，不迁移其 CSV 结果。v2 发布时删除旧 CSV 结果链路，只登记经过校验的 Parquet
ResultDataset。

用户继续说“找出电压最低的十个节点”“画出节点 632 三相电压”“比较修改前后每条线路的损耗”时：

1. agent 调用 `list_result_datasets` 获取候选表的名称和 schema；
2. 指代不唯一或缺少所需原始结果时，agent 说明候选或要求重跑以产生缺失 DataFrame；
3. agent 依据版本化的 `rss_language_reference/rsdss_function_reference` 生成 RSS，显式使用
   `select/filter/join/group_by/aggregate/sort/limit` 等函数；
4. `run_result_analysis` 校验 dataset 归属、input alias、脚本能力和输出名，应用层创建分析运行；
5. 适配器把 dataset ID 解析为受控路径，使用 `--input` 和独立 `--result-dir` 执行；
6. 输出继续登记为 ResultDataset，保存脚本 artifact、输入文件哈希和完整血缘；
7. 工具向 agent 和 result 消息返回同一份有上限预览，使智能体能回答具体数值；标量结论也输出为
   一行 DataFrame，不解析 stdout；
8. 分页表格对每个 ResultDataset 始终可用；初始 `VisualizationSpec` 作为 `dataset_views` 记录保存，
   同一 dataset 可追加多个命名图表，不把 DataFrame 全量塞进消息/SSE。

若请求只改变现有列的视觉映射，例如“把这个结果改成散点图”，agent 使用 `create_dataset_view`，
不运行 tensoreval。若需要筛选、聚合、透视、分箱、分位数或改变数据语义的降采样，则先执行上述
RSS 分析流程。首批视图为折线、柱状、散点、热力、箱线和直方图；Agent 不生成任意 ECharts option，
只提交后端可按列、dtype、单位和数据规模校验的版本化 `VisualizationSpec`。

比较两个任务也是一次受约束的结果分析：

1. 先选择两侧要比较的结果 DataFrame，检查列、dtype、单位、稳定业务键和维度；`study_type` 只用于
   解释来源，不作为一票否决条件；
2. 按 catalog 定义的稳定业务键连接，禁止按行号比较；
3. QSTS 额外按时间轴连接，缺失设备或时点单独报告；
4. RSS 脚本输出包含稳定键、基准值、候选值、绝对差和相对差的 DataFrame；基准为零时相对差为
   null，并可另输出诊断表；
5. 输入版本差异和分析参数差异来自 manifest/study spec，数值结论从派生 DataFrame 读取；
6. 不兼容时返回 `comparison_incompatible`，不伪造差值或固定指标摘要。

## 9. 消息、API 与历史恢复

v2 使用以下 8 类消息：

- `model-status`：active/draft 范围及其完整快照；null snapshot 用于明确清空 draft；
- `comparison`：基准/候选任务、关联 analysis run 和差值数据集；
- `progress`：以 `operation.kind + operation.id` 区分 ingestion、simulation 与 analysis；
- `result`：出网使用 `outputs[]` 引用 ResultDataset，不再发送预设 `MetricValue[]`；
- `tool-call`：出网以稳定 `callId` 合并 running 与终态。

`progress.stages` 保持“每次发送完整数组、前端按对象 ID 替换”的语义，但归并键和阶段由操作决定：

- 摄取：`extract → infer → validate → version`；
- 仿真：`prepare → inspect → sim → collect`；
- 结果分析：`script → run → collect → present`。

完整已实现端点以 [api-design.md](api-design.md) 为准。与模型工作区直接相关的读取和上传端点为：

```text
GET  /api/sessions/{id}/workspace
GET  /api/examples?query=...&studyType=...&cursor=...
GET  /api/examples/{id}
POST /api/sessions/{id}/attachments
GET  /api/sessions/{id}/messages?beforeId=...
POST /api/sessions/{id}/messages
GET  /api/sessions/{id}/events
GET  /api/sessions/{id}/versions
GET  /api/sessions/{id}/tasks
GET  /api/ingestions/{id}
GET  /api/versions/{id}
GET  /api/tasks/{id}
GET  /api/tasks/{id}/datasets
GET  /api/analysis-runs/{id}
GET  /api/datasets/{id}
GET  /api/datasets/{id}/rows
GET  /api/dataset-views/{id}/data   # 预算内完整投影；超限返回稳定错误
GET  /api/datasets/{id}/download
GET  /api/task-comparisons/{id}
GET  /api/storage-usage
```

SSE 只负责实时通知，数据库仍是历史真相。重新进入会话时，前端先建立 SSE 并等待订阅就绪，再
并行读取消息、工作区摘要和任务列表，最后合并订阅期间暂存的事件。持久化消息使用消息 ID 作为
SSE event id，断线通过 `Last-Event-ID` 补发；不能采用“先 GET、后订阅”的丢事件流程。Workspace
摘要包含 active ingestions，使刷新后能恢复每个上传批次。会话删除时级联删除其元数据，内容 blob
仅在没有其他引用后清理。

文件传输、只读案例浏览以及由确定性规则完成的解包、表头推断、候选版本发布属于固定交互范围；点击
“使用此案例”会发送一条可见用户消息，由 agent 调用 `import_example`。没有 active 时，首个校验通过的
ready 版本可按 §4.1 自动激活。除此之外，映射确认、替换、移除、修改、激活/回退、
运行/重跑和比较等领域状态变更均由自然语言触发，前端不提供直接执行按钮；智能体可给出少量上下文
自然语言建议，点击后仍发送一条可见 user 消息。
context 只用于资源消歧；后端校验引用属于当前 session 后交给 agent，agent 再调用领域工具。固定
控件限于会话管理、上传/重试和只读浏览等高频低歧义操作；结果目录、表格、多图表和文件下载可由前端直接
访问 REST API。

聊天历史检索与仿真历史是两个相关但不同的入口：`search_chat_history` 用于定位“哪次对话”，
`list_simulation_history` 用于在已知会话或候选集合内解析“哪次任务”。自然语言出现“上次”“修改前”
时先查当前会话的任务/版本时间线；无法唯一解析才扩大到跨会话全文检索。搜索结果只返回摘要和
稳定 ID，需要更多上下文时再通过 `read_chat_context` 读取命中附近窗口。

## 10. 稳定错误码

```text
archive_invalid              archive_unsafe_path
artifact_too_large           unsupported_file_type
idempotency_conflict
dataframe_schema_unknown     dataframe_schema_ambiguous
duplicate_table_requires_confirmation
missing_dependency           missing_required_table
model_input_not_found        version_conflict
invalid_model_patch          unsupported_study
model_validation_failed      solver_failed
not_converged                comparison_incompatible
result_dataset_not_found     result_schema_mismatch
analysis_script_rejected     analysis_output_limit_exceeded
dataset_view_not_found       invalid_visualization_spec
visualization_data_too_large
agent_queue_full
invalid_cursor               invalid_pagination
history_query_too_long       history_search_unavailable
storage_quota_exceeded
engine_resource_unavailable engine_contract_mismatch
example_not_found           example_not_importable
example_source_changed      engine_feature_unavailable
```

HTTP 错误体扩展为 `{ "error": "可读信息", "code": "稳定错误码", "details": {...} }`。
`details` 只能包含经过筛选的字段、缺失项和冲突证据，不暴露服务器绝对路径或内核原始错误。

## 11. 已完成的实施顺序与验收基线

1. 建立可测试的应用入口、身份 scope、运行队列和可恢复 SSE，以 v2 合同直接替换原型接口；
2. 建立 `TENSOREVAL_CWD` 文档/案例快照、artifact/配额、工作区 active/draft、不可变版本和 catalog
   表头推断；
3. 完成多文件、Excel、Parquet、ZIP/`.rssbundle` 摄取、内置案例导入，以及结构化模型修改工具；
4. 接入通用 `run_study`、原生 Parquet 结果、`run_result_analysis`、`dataset_views` 与表格/多图表读取；
5. 完成任务比较、聊天全文/自然语言检索和历史上下文恢复；
6. 完成前端工作台、四条端到端路径、容量测试，并收口为只支持空库初始化的干净 v2 schema。

逐任务文件、接口、失败测试和提交边界以
[`docs/superpowers/plans/2026-10-22-v2-simulation-workspace.md`](superpowers/plans/2026-10-22-v2-simulation-workspace.md)
为准。

必须覆盖四条端到端验收路径：单 `.rss`、ZIP 或目录形式的 `.rssbundle`、分批上传 DataFrame、从内置
案例库导入。每条路径均
验证模型修改、重跑、自然语言生成结果分析脚本、分页表格/多图表展示、结果对比和刷新后的血缘恢复。表头推断另以 catalog 中各输入 schema 做
正例，并覆盖未知表、相似 schema、dtype 冲突、重复表和多工作表 Excel。

四条路径都必须能够只靠用户自然语言完成领域操作；上下文建议的验收需确认点击后产生可见 user
消息并经过 agent 工具链，前端不存在替换、修改、运行或比较的旁路写调用。
另需用跨会话中文关键词、日期条件、分析类型、模糊“上次”指代和深层消息定位验收历史查询；任何
结果都不得越过当前身份的 session 归属范围。

## 12. 已确认决策

1. 工作区归属于会话，首版不建设跨会话模型库；
2. 版本不可变，任务只引用具体版本；工作区分别保存 active 与 draft 指针；
3. DataFrame 类型根据表头（列名）推断，文件名不参与决定；dtype 只用于后续校验；
4. 模糊或重复匹配必须由用户确认；
5. Excel 每个工作表作为独立候选表；
6. 重传不覆盖旧文件，替换、移除和回退通过版本关系表达；
7. 模型修改使用结构化补丁；
8. 比较对象是两个具体仿真任务；
9. 进度按操作类型与 ID 归并，摄取 ID 与仿真 ID 不共享命名空间；
10. 文件上传和确定性摄取可由固定控件触发，首个 ready 版本可自动激活；其余领域写操作由自然语言
    驱动，上下文建议也必须形成可见消息；
11. 历史聊天检索与仿真任务检索分层，跨会话命中只读取必要窗口；
12. 所有内核调用继续收敛在 `backend/src/adapters/`。
13. v2 仿真和结果分析都显式使用 `--result-dir --output-format parquet`，以原生 Snappy Parquet 为事实
    来源；数据库不预设领域统计值，并删除未投入使用的 v1 CSV 结果链路；
14. 每次结果查询保存 RSS 脚本、输入 dataset/alias、输出 dataset 和初始视图，比较复用同一机制；
15. DataFrame 始终有分页表格入口，并可拥有多个 `dataset_views`；前端把受控 `VisualizationSpec`
    编译为 ECharts option，模型和 API 不传任意可执行图表配置。
16. 每个归属主体默认保留 20,000 个唯一 artifact、20 GiB；不按时间静默淘汰，删除历史后再由 GC
    释放额度；instance 模式按实例、trusted_proxy 模式按 owner/tenant 计费。
17. 服务启动时预索引 `TENSOREVAL_CWD/sparrowzz/dss/docs`，脚本生成时按需注入相关 section；不把整套
    文档常驻每轮 prompt，也不把“学习”实现为模型微调。
18. 根目录 RSS 和已发布 rssbundle 形成只读案例库；导入必须复制到用户 artifact/版本链，修改永不写回
    共享案例目录。
