# 对话 Agent 直接生成 RSS 的多源结果分析设计

## Status

本设计等待用户批准，尚未实施。它扩展当前 V4 自然语言结果分析，不改变研究配置最终确认、普通脚本运行、
图表工作台或模型发布的既有权限边界。

## Intent

用户用自然语言提出分析要求后，当前对话 Agent 应理解三类现有 DataFrame 的业务含义：

- 当前会话唯一 active 模型中的模型表；
- 当前会话已经发布的本地数据资源；
- 当前会话仍存活的仿真或派生结果数据集。

Agent 根据本次启动发布的引擎 catalog、RSDss 函数合同和实际 DataFrame 元数据，直接生成受控 RSS，
把选中的完整 DataFrame 作为命名变量输入并立即执行，最终生成用户要求的不可变 Snappy Parquet 结果。
生成脚本的模型就是当前对话 Agent，不再引入第二个 AnalysisAuthor 或 ScriptAuthor。

这里的“理解”不是把完整 Parquet 发给 LLM。Agent 只读取受限的 schema、单位、稳定键、来源和必要样例；
执行时后端向 tensoreval 提供完整冻结 artifact。这样大结果不会进入上下文或应用内存中的整表 JSON，结果也不会被截断。

## Goals

1. 一次分析可以同时使用 active 模型表、普通数据资源和结果数据集。
2. 对话 Agent 直接生成最终 RSS，并在信息完整且来源唯一时无二次确认地执行。
3. 只允许当前 active 模型；草稿和历史版本不能作为自然语言分析输入。
4. 所有输入只通过稳定资源 ID 和受控 spec 选择，执行前冻结 ID、schema/content hash、alias 和来源血缘。
5. Agent 依据当前 engine snapshot 的机器 catalog 和按需文档写脚本，不依赖常驻的陈旧函数清单。
6. 在执行前拒绝明显不受控的脚本；可修正的解析、函数、列和类型错误返回稳定且脱敏的诊断，由同一个
   对话 Agent 在固定预算内重写完整脚本。
7. 成功分析的实际脚本、输入快照和输出血缘可在界面恢复查看。
8. 覆盖当前 `bus_voltage.v_mag/v_ang` 等 List 列的通用逐项展开需求，使 active 模型中的相序和电压基准能与
   仿真结果组合，而不是为某一个问题增加固定指标。
9. active 模型与结果数据集混用时，结果的冻结模型来源必须精确等于当前 active 版本；不能只凭同名母线、相同
   schema 或相同内容形状推断对应关系。
10. 影响结果正确性的非空、正值和 List 对齐条件必须由 RSDss 在完整输入上验证；有界样例只用于 Agent 理解，
    不能作为全量数据质量证据。

## Non-goals

- 不开放 Python、pandas、SQL、shell、服务器路径、URL 导入或任意 ECharts option。
- 不让对话 Agent 创建普通脚本资源、普通运行或绕过研究配置的“确认并生成脚本”按钮。
- 不允许 draft 或任意历史模型版本参与分析，也不自动发布草稿或切换 active 版本。
- 不把完整 DataFrame、完整结果文件或服务器物理路径发送给 LLM。
- 不为电压标幺值、阈值、比较键或单位建立 zjudss 私有固定业务公式；这些语义来自当前 catalog 和实际输入。
- 不恢复 CSV 结果链路，不改变 Snappy Parquet 事实来源和 2 GiB 单结果/8 GiB 单次结果安全上限。
- 不给派生分析新增固定墙钟超时或用户取消入口；它仍运行到完成、实际失败、结果安全拒绝或服务停止。

## Anti-goals

以下任一行为即使“能算出结果”也视为失败：

- 为了分析自动读取草稿、最近历史版本或同名但非 active 的模型表；
- 根据文件名、聊天历史或列名猜错输入，或在多个合理来源之间静默选择；
- 模型生成 Python 风格脚本、调用当前引擎不存在的函数，失败后无限重试；
- 由后端再次调用另一个 LLM 生成脚本，使用户看到的对话意图与实际执行脚本分离；
- 用样例行替代完整输入执行，或因为结果较大而只保存部分输出；
- 在缺少 `voltage_base_v`、单位或 List 对齐关系时猜测标幺基准或相序；
- 把旧模型产生的结果与当前 active 模型表直接关联，即使稳定键或 schema 看起来相同；
- 只检查样例行就宣称完整输入的基准、null 或 List 长度合法；
- 成功后只展示结果而无法追溯实际脚本和三类输入的冻结身份。

## Current gap

当前 `run_result_analysis` 已由对话 Agent 提交 `intent + script + inputs + expectedOutputs`，但输入只有
`datasetId` 或 `resourceId`。结果分析 skill 的触发词只有英文内部词，system prompt 又要求每轮先读取工作区，
容易把纯结果分析错误路由到 `model-intake`。`read_model_dataframe` 也只接受 `model-intake` skill。

现有分析服务只做路径字符串和 alias/数量校验，然后立即提交脚本并调用 tensoreval；没有按当前
`functions.json` 拒绝未知、导入或 DSA 函数，也没有对输出绑定和输入 alias 重写做分析专用结构检查。
引擎错误只被压成普通工具错误，没有明确的请求级修正预算。

真实百炼复验已经证明 `intent` 字符串合同修复有效，但随后出现了两个独立问题：Agent 无关地进入模型 intake，
并先后生成 Python/RSS 混合语法和当前 RSDss 不支持的 `explode`。当前 `bus_voltage` 的 `v_mag/v_ang` 是 List，
而 active `bus` 模型表拥有与之对应的 `terminals` 和可空 `voltage_base_v`；现有 RSDss 没有把多列 List 按行锁步展开的
整表函数，因此无法可靠生成 `time,bus,phase,v_pu` 这类长表。

## Approaches considered

### A. 对话 Agent 直接生成并执行 RSS（选定）

对话 Agent 自己发现来源、按需读取当前引擎知识、生成 `run_result_analysis.script`，工具负责冻结、预检和执行。
优点是用户意图、工具选择、脚本和修正都处于同一对话轨迹，架构最贴合当前产品；代价是必须强化路由、知识读取、
结构预检、错误反馈和请求级预算。

### B. 独立 AnalysisAuthor

对话 Agent 只提交结构化目标，由无工具的第二个模型生成和修正脚本。它能像受管研究一样把生成循环放到后端，
但会增加第二套 prompt、模型调用、审计和故障边界，也不符合用户选择的“对话 Agent 直接生成脚本”。不采用。

### C. 结构化分析计划加确定性编译器

Agent 只输出 filter/join/aggregate 等 AST，服务端编译 RSS。它最容易做静态校验，但会形成一门新的受控查询 DSL，
无法覆盖当前 RSDss 的完整能力，并把用户要求的“生成脚本”改成“生成计划”。本阶段不采用。

## Chosen flow

### 1. 路由与领域加载

system prompt 首句改为“先理解用户意图；只有请求实际需要时才读取对应工作区或资源”，不再要求所有消息先检查工作区。
纯筛选、排序、聚合、统计、透视、关联、List 展开、越限和派生 DataFrame 请求直接加载 `result-analysis`，不得加载
`model-intake`，除非同一条用户消息还明确要求修改或发布模型。

`result-analysis` 增加中英文触发词，包括分析、筛选、排序、聚合、统计、透视、关联、展开、越限、派生、
query result、aggregate、filter、join 和 explode。触发词只帮助路由，最终仍以用户意图判断；纯画图继续进入结果工作台。

skill 明确规定：对话 Agent 是脚本作者；信息完整时必须在同一轮调用 `run_result_analysis`；禁止只给示例脚本而不执行。
它还要求在使用非基本函数或不确定签名时先 `search_engine_docs`，再以稳定 section ID 调用
`read_engine_doc_section`，不得凭模型记忆发明函数。

### 2. 按需发现三类来源

Agent 不一次性枚举所有数据，也不把完整 catalog 常驻 prompt：

- 结果：优先使用有界 `currentResultContext`；歧义或裁剪影响判断时调用 `list_simulation_history` 和
  `list_result_datasets`。新增只读 `read_result_dataset`，按稳定 `datasetId` 返回当前会话内的完整元数据和至多
  100 行、32 列的受控样例，不返回路径。
- 本地数据：使用现有 `list_data_resources` 和 `read_data_resource`；这里的“本地文件”只指已经上传、规范化并发布的
  DataFrame 资源，不允许读取任意服务器文件。
- active 模型：只有分析目标需要模型语义或模型列时才调用 `inspect_workspace`。其 active 表摘要增加稳定
  `modelDataframeId`；再按需使用 `read_model_dataframe(scope=active)` 读取受限实际行。

`read_model_dataframe` 的权限改成分支式门禁：`scope=active` 在加载 `result-analysis` 或 `model-intake` 后可读；
`scope=draft` 仍只允许 `model-intake` 且必须提供准确 draft revision。加载 `result-analysis` 永远不能读取草稿。

来源、列、单位、List 分量、join key 或阈值存在实质歧义时，Agent 只询问一个能改变执行结果的必要问题。
名称、schema、单位和稳定键能唯一确定来源时直接继续，不增加脚本确认步骤。

### 3. 工具输入合同

`run_result_analysis.inputs[]` 保持显式 alias，并从两种来源扩展为三种互斥来源：

```json
{
  "intent": "结合 active 母线模型，把当前电压结果展开为逐相标幺值",
  "script": "...受控 RSS...",
  "inputs": [
    {"modelDataframeId": 41, "alias": "bus_model"},
    {"datasetId": 22, "alias": "bus_voltage"}
  ],
  "expectedOutputs": ["phase_voltage_pu"]
}
```

每个输入必须且只能提供 `modelDataframeId`、`datasetId` 或 `resourceId` 中的一个。ID 出网仍为 JSON 十进制整数的
既有工具风格，HTTP DTO 继续按字符串返回公共 ID。alias 必须唯一、合法且不能与 RSDss 保留名、函数名、输出名或
服务端临时命名空间冲突。

`expectedOutputs` 只是脚本应发布的有序结果变量名，不是脚本、schema 或自然语言期望。工具不接受服务器路径、SQL、
Python 代码、任意命令或动态 ECharts 配置。

模型输入解析时，服务端按当前用户和会话查找 `modelDataframeId`，并验证它此刻属于 workspace 的唯一 active 版本。
已切换 active、历史版本、草稿、已删除版本、其他会话或其他用户统一失败；active 竞态使用稳定
`active_model_changed`，其余不存在/越权保持 404 语义。调用成功后冻结实际 `modelVersionId`、
`modelDataframeId`、table name、artifact、schema、content hash 和 alias。它不会在 active 变化后悄悄换成同名表。

同一次分析同时包含 active 模型输入和 dataset 输入时，服务端还必须解析每个 dataset 的冻结模型来源：直接仿真结果
取其 run 的 `modelVersionId`；派生结果沿不可变 dataset lineage 最多递归 64 层，汇总所有直接仿真或模型输入的
`modelVersionId`。每个 dataset 必须恰好得到一个来源版本，并与当前 active 版本 ID 精确相等。无模型来源、多个模型来源、
循环、超深或版本不一致都返回 `analysis_model_origin_mismatch`，不进入脚本预检或执行。这里不接受 content hash 相等替代
版本 ID，也不提供跨版本确认开关；用户需要重新运行当前 active 模型，或把产生结果的版本重新激活后再分析。

三类来源合计仍限制为 1..=20。所有 artifact 都复制到 adapter 的私有输入树并以 alias 注入；LLM 和前端永远看不到
物理路径。所有 tensoreval 调用继续只通过 `backend/src/adapters/`。

### 4. 分析专用脚本预检

新增 `AnalysisScriptInspector`，在提交脚本 artifact 和创建 analysis run 以前使用当前 engine snapshot 的
`functions.json` 做确定性预检。它不是 RSDss 的第二套完整解释器，只检查能在应用边界可靠证明的结构：

1. 顶层程序只能由以分号结束的 `target = function(...)` 声明组成；不允许裸表达式、控制流或直接 alias 赋值。
2. 输入 alias 不得被重新赋值；目标名必须合法且不能覆盖输入。
3. 实际终端输出与 `expectedOutputs` 有序且精确一致，不允许未知、重复或漏报输出。
4. 所有顶层和表达式函数必须存在于当前 `frame_functions`、表达式函数合同或其 catalog alias 中，并满足 catalog 的
   固定/最小 arity。
5. 分析脚本只允许 DataFrame 查询/变换函数；`import_*`、`read*`、`import_df_url`、所有 `ds_*`/投影函数以及其他
   IO、研究或网络入口全部拒绝，即使它们存在于 catalog。

预检不尝试在 zjudss 中重写 Polars 类型推断或完整传播每个派生列。列缺失、join 后缀、List 元素类型、null 和实际
运行语义仍以当前 tensoreval 为权威。这样既能在进程前拒绝当前复验中的 Python 风格/未知函数错误，又不会形成与引擎
漂移的第二套执行器。

预检失败返回 `analysis_script_invalid` 和结构化字段（阶段、函数/输出/alias、当前 snapshot），不创建 analysis run
或脚本 artifact；durable tool-call 消息仍保存这次尝试。

### 5. 直接执行与有界修正

预检通过后，现有 `AnalysisService` 冻结输入、提交脚本、创建 analysis run，并通过 adapter 调用 tensoreval。
执行成功仍只在所有输出名称、安全限制和 Parquet 读取验证通过后原子发布。

可修正错误包括 RSDss 解析/arity、未知列、dtype/List 不兼容、join/aggregate 语义和输出名称合同错误。
服务将 adapter 诊断转换为稳定 code、阶段和最多 64 KiB 的脱敏模型反馈；绝对路径、命令、secret 和未筛选 stderr
不能进入工具消息或模型。路径安全、结果监控、配额、来源消失、存储和基础设施错误不可修正，Agent 必须停止重试并说明。

每个 user request 的 `ToolServices` 记录 `run_result_analysis` 尝试数；初次调用加最多 3 次修正，总计最多 4 次工具调用。
预检失败和实际执行失败都消耗预算；第五次返回 `analysis_repair_budget_exhausted`，不执行。修正由同一个对话 Agent 在
收到诊断后的下一模型轮完成，只能重新提交完整脚本，不能由后端暗中调用第二个模型。

为了容纳 skill 加载、来源检查、文档读取和最多四次执行，本轮 Agent `MAX_TURNS` 从 8 调整为 12；显式的分析调用预算、
研究工具门禁和其他既有授权仍防止无限循环。达到总轮次上限时，回复改为通用且准确的“本轮工具调用已达到上限，操作尚未完成”，
不能误报为“研究配置”失败。

每次预检通过并实际调用引擎的失败仍形成独立失败 analysis run，保存脚本和冻结输入用于审计；下一次修正形成新的 run。
最终成功 run 不覆盖失败记录。修正不增加固定执行超时，也不对超大结果做采样发布。

### 6. RSDss 全量断言与多列 List 展开

在 eig-rs/mems 的实际 DataFrame 函数分派中新增通用全量断言：

```text
validated = assert_all(predicate, frame);
```

- 固定两个参数；`predicate` 必须求值为与输入等长的 Boolean 表达式，`frame` 是输入 DataFrame。
- 对完整 DataFrame 求值；所有值都为 true 才原样返回输入表，任一 false 或 null 都使执行失败。
- 错误只返回总行数、false 数和 null 数等有界统计，不返回原始行、路径或敏感值。
- 它用于表达通用数据质量前置条件，不在 zjudss 中硬编码 `voltage_base_v`、阈值或其他电气公式。

Agent 只要依赖可空列、正值基准或其他会影响正确性的条件，就必须在计算前生成 `assert_all`。该断言运行在完整输入上，
因此样例之外的异常同样会失败关闭；断言失败属于数据条件，不允许通过删除异常行、填默认值或猜测基准自动修正，
Agent 应说明失败条件并询问用户修正来源或重新仿真。

同时新增多列 List 展开：

```text
exploded = explode(columns, frame);
```

- 固定两个参数；`columns` 接受一个列名、逗号分隔名称或名称数组，至少一列且不能重复。
- 所选列必须存在并是可展开的 List/Array 类型；其他列按 Polars 语义重复。
- 同一原始行中的多个所选列必须是非 null、非空且能锁步展开；null、空列表、长度不一致、错误 dtype 或缺列返回
  明确 RSDss 错误，不能静默截断或省略原始行。
- 函数返回一张 DataFrame，可继续用于 `select/filter/join/with_columns`。

同步更新 `functions.json` 的 `frame_functions`、`rsdss_function_reference.md` 和真实执行测试。新增函数条目不改变 catalog
JSON schema，`functions.json` 仍为 schema version 2；它改变 engine resource snapshot hash，zjudss 更新资源后必须重启。
`tensoreval-parquet-v3` 表示 adapter/wire protocol，不因增加 catalog 函数而升级。

本阶段不把 `v_pu` 硬编码为新的结果列，也不修改 `bus_phase`/`bus_voltage` 的默认选择。以当前数据计算逐相标幺值时，
Agent 可以把 active `bus` 的 `id,terminals,voltage_base_v` 与结果 `bus_voltage` 关联，锁步展开
`terminals,v_mag,v_ang`，再计算 `v_mag / voltage_base_v`。示意 RSS 为：

```text
bus_basis = select(
    alias(col(id), bus),
    col(terminals),
    col(voltage_base_v),
    bus_model
);
voltage_with_basis = join(bus_voltage, bus_basis, bus, bus, left);
validated_voltage = assert_all(gt(col(voltage_base_v), 0), voltage_with_basis);
phase_rows = explode("terminals,v_mag,v_ang", validated_voltage);
phase_voltage_pu = select(
    col(time),
    col(bus),
    alias(col(terminals), phase),
    alias(col(v_mag) / col(voltage_base_v), v_pu),
    phase_rows
);
```

实际脚本必须以当前文档的精确字面量/签名为准，上例只说明数据流。左连接保证结果中的未知母线不会被静默删除；
`assert_all` 把缺失、null 和非正 `voltage_base_v` 在全量数据上失败关闭；`explode` 把 `terminals` 与结果 List 的
null、空值和长度不一致失败关闭。任一条件失败时 Agent 必须询问用户或说明当前数据不足，不能猜测额定电压、相序，
也不能通过过滤异常行生成看似成功但不完整的结果。

## Persistence and lineage

V4 仍以空库 initial migration 为权威，直接扩展 `result_analysis_inputs` 和 `dataset_lineage_snapshots`：

- 增加 active 模型来源所需的 `model_version_id`、`model_dataframe_id` 和 source kind；
- 三类稳定来源 ID 互斥，模型 DataFrame ID 作为冻结审计值保留，即使历史版本删除时物理 model_dataframes 行被清理；
- 每条输入继续保存 alias、ordinal、source name、schema JSON/hash、content hash 和 producer JSON；模型 producer 明确记录
  active version ID/hash、table name 和本次解析时的 dataframe ID；
- 版本删除在分析 `pending|running` 时视为 busy；分析终态后允许沿用既有 tombstone 行为，派生结果依靠冻结快照继续可读；
- 数据集 lineage 公共投影增加 `sourceKind` 和可空模型版本/DataFrame ID，不把模型输入伪装成普通资源或结果数据集。
- 提供只读的 dataset 模型来源解析查询：直接 run、分析输入和派生 lineage 使用同一套最多 64 层的循环/深度保护，
  供 mixed analysis 在任何写入前验证与 active 版本精确一致。

分析运行和输出仍不可修改。相同内容可由 CAS 去重，但不同 tool attempt、run 和 lineage 身份不能合并。

## API and frontend

不新增分析创建 HTTP 路由；创建仍只有受控 Agent 工具。`GET /api/analysis-runs/{id}` 以向后兼容的附加字段返回：

- 实际脚本正文、content hash 和 engine snapshot/contract；
- 有序冻结输入摘要，包含 source kind、稳定 ID、alias、schema/content hash 和来源名称；
- 既有 intent、skill refs、状态、错误和输出 datasets。

脚本最多沿用现有 2 MiB 分析脚本上限，详情按用户主动展开时懒加载；消息列表和 prompt 不常驻全文。
前端在派生分析的结果卡增加“查看分析脚本与输入”，显示只读 RSS 代码块和三类来源摘要；它不提供编辑、重新运行、
路径、artifact 下载或脚本创建按钮。数据集工作台继续从 lineage 显示来源删除状态。

`frontend/src/types/` 同步扩展 `AnalysisRun` 和 lineage 判别联合。跨用户或不存在 analysis/dataset/model 输入仍统一 404。

## Agent bundle and versioning

- Agent bundle 从 `v4.4.0` 升到 `v4.5.0`。
- `result-analysis` 从 `1.2.1` 升到 `1.3.0`；更新触发词、三类来源、直接脚本作者、按需文档和修正规则。
- allowed tools 增加只读 `read_result_dataset`，总数从 23 变为 24；manifest 仍是权限和顺序唯一来源。
- `result-analysis.requiredTools` 明确包含来源发现/读取、引擎文档读取和 `run_result_analysis`；这不授予模型修改、
  发布、研究确认、普通脚本、运行或图表权限。
- manifest、system、skill-index 和 skill 内容 hash 全部重算；运行/分析保存实际 bundle、skill ref 和 engine snapshot。
- engine contract 保持 `tensoreval-parquet-v3`，catalog schema versions 保持当前版本；实际资源字节变化由 snapshot 标识。

## Error policy

新增或明确使用以下稳定错误：

- `analysis_script_invalid`：分析专用结构/catalog 预检失败；
- `analysis_function_forbidden`：脚本请求 IO、DSA 或其他非分析能力；
- `analysis_repair_budget_exhausted`：同一 user request 已达到四次调用；
- `active_model_changed`：指定 model DataFrame 已不属于当前 active 版本；
- `analysis_model_origin_mismatch`：与 active 模型混用的 dataset 无唯一同源模型版本；
- `analysis_input_not_found`：来源不存在、已删除、跨会话或跨用户；
- `analysis_engine_failed` / `analysis_result_contract_invalid`：分别表示可修正执行诊断和输出合同诊断。

公开错误只含稳定 code、可读信息和脱敏结构字段。不可修正基础设施错误继续沿用现有 code，不因为模型可能重试而改名。

## Security and performance

- 脚本只允许当前 snapshot 列出的纯 DataFrame/表达式函数；禁用任何导入、URL、DSA、投影和文件读取入口。
- 所有 repository 查询显式接收 `UserId`，再检查 session；active 模型还校验 workspace active 指针。
- prompt 样例仍受 64 KiB/32 列/100 行等工具预算约束，完整 artifact 只在私有 staging 树流式复制和执行。
- `explode` 可能放大行数，但不设猜测性行数截断；既有结果文件数、单文件 2 GiB、总量 8 GiB、用户配额和路径安全
  监控继续原子拒绝超限输出，绝不发布部分结果。
- 生产分析没有固定执行超时。自动修正只在一次引擎调用实际终止并返回可修正错误后发生。

## Testing and acceptance

### zjudss deterministic tests

1. `AnalysisScriptInspector` 单元测试覆盖分号、直接函数声明、输入覆盖、输出集合、catalog alias/arity、未知函数、
   `import_df_url`/`ds_*` 禁止和多层表达式。
2. 工具 schema/行为测试覆盖三种互斥输入、最多 20 个混合来源、alias 冲突、active-only、active race、草稿/历史/
   跨会话/跨用户拒绝，以及完整冻结 schema/content hash。
3. `read_model_dataframe` 证明 result-analysis 只能读取 active，draft 仍要求 model-intake；`read_result_dataset` 证明
   稳定 ID、session/用户隔离、列投影、cursor 和预算。
4. 分析事务测试覆盖预检失败零写入、实际失败 run 审计、成功原子发布、三类 lineage、版本删除 busy/终态 tombstone、
   来源删除后派生结果仍可读。
5. scripted-agent 测试验证中文分析不加载 `model-intake`，只在需要模型时检查 workspace，直接提交 RSS；唯一来源执行、
   歧义询问、文档按需读取、一次可修正失败后成功和四次预算耗尽均有行为证据。
6. Agent bundle/contract 测试锁定 `v4.5.0`、`result-analysis@1.3.0`、24 个工具和新的 input one-of。
7. 前端 Vitest 覆盖分析详情懒加载、脚本/三类输入展示、失败与跨请求迟到响应，不增加编辑或运行入口。
8. Playwright 经真实 UI/HTTP 验证自然语言生成混合输入分析、结果可打开，以及脚本与 lineage 可恢复查看。

### eig-rs tests

1. 先写失败测试，再实现 `assert_all(predicate, frame)`；覆盖全 true、false、null、非 Boolean、空表和有界失败统计。
2. 先写失败测试，再实现 `explode(columns, frame)`；覆盖单列、多列锁步、其他列重复、null/空 List、缺列、重复列、
   非 List 和长度不一致，并证明异常不产生部分输出。
3. tensoreval CLI 真实执行 RSS，验证全量断言、展开后的输出列、行序和 Snappy Parquet。
4. catalog/reference 测试证明 `functions.json` 与实际注册一致，文档签名可检索。

### Real dependency gate

发布门禁新增一个真实 LLM + 当前 tensoreval 旅程：从实际 active `bus` 和真实 `bus_voltage` 生成逐相派生表，trace 必须
只出现 `result-analysis`、必要的只读发现/文档工具和 `run_result_analysis`，不得出现 `model-intake`、修改或发布工具。
结果逐页验证 `time,bus,phase,v_pu`、输入 lineage 和完整行数；若 fixture 的 `voltage_base_v` 缺失则 fixture 本身失败，
不能改成模型猜测。既有真实结果复制、比较、全案例 projector 和代表门面门禁继续回归。

功能目录、自动化行为测试、`docs/development-status.md`、`docs/api-design.md`、
`docs/agent-prompt-skills-design.md`、`docs/frontend-design.md`、`docs/db-design.md`、
`docs/v4-deployment.md` 与 `docs/integration-testing-design.md` 同步更新。注册表枚举测试不能替代上述行为覆盖。

## Rollout and compatibility

- 这是未上线 V4 的空库基线，数据库只修改 initial migration，不写历史迁移或双读兼容。
- 既有只含 dataset/resource 的 `run_result_analysis` 调用仍合法；model input 是新增 one-of 分支。
- 旧结果保持可读；需要 `explode` 或 active 模型列的旧结果可以直接参加新分析，不要求重跑仿真，只要所需 active
  模型版本 ID 与结果的唯一冻结模型来源精确相等，且全量断言通过。若旧结果无唯一模型来源、来源版本不同、缺少足够
  schema/单位或完整输入不满足断言，必须要求用户重新运行当前 active 模型或修正来源。
- 更新 eig-rs binary 和 `SPARROWZZ_DIR` 后重启 zjudss，启动 preflight、catalog/snapshot 发布和完整发布门禁全部通过后
  才能开放服务；服务不得 checkout、pull、reset、clean 或改写配置的资源树。

## Builder calls

以下是本设计为实现者明确作出的调用，不需要再次向用户选择：

- 使用现有工具分别发现来源，不新增一个把所有模型/文件/结果一次性塞入 prompt 的总 inventory 工具。
- 新增窄的只读 `read_result_dataset`，并让现有模型读取工具按 scope 做 skill 门禁。
- 增加通用 `explode`，不增加固定 `v_pu` 结果列，也不改变仿真默认结果选择。
- 增加通用 `assert_all`，并要求所有影响正确性的值域条件在完整 DataFrame 上失败关闭；有界样例不作为全量证据。
- active 模型与 dataset 混用时要求 dataset 的唯一冻结模型来源版本 ID 精确等于当前 active 版本，不提供跨版本覆盖。
- 结构预检只证明语法外形、函数能力边界、arity、alias 和输出合同；完整列/type 语义仍由 tensoreval 决定。
- 四次分析调用预算包含预检失败；对话总轮次提高到 12。
- 分析详情复用现有 GET 路由返回脚本和输入，不新增脚本编辑、执行或下载路由。
- engine contract 和 catalog schema version 不升级；Agent bundle/skill 版本升级并依赖新的 snapshot hash。

## Left to the builder

实现计划可以在不改变上述产品行为的前提下决定 Rust 模块拆分、catalog 解析缓存结构、前端折叠组件名称、
测试 fixture 组织和错误诊断内部类型。实现时必须遵守两个仓库的现有格式/测试规范，保留用户已有未提交文件，
且 eig-rs 的代码、sparrowzz 资源提交和 zjudss snapshot 验证要按依赖顺序集成。
