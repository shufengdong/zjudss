# V4 Agent Prompt、Skills 与工具

## Bundle

生产 bundle 位于 `backend/resources/agent/v4/`，版本 `v4.1.0`。manifest 固定 system prompt、skill index、
engine contract、允许工具和文件 SHA-256；启动时完整验证。Agent prompt 只常驻策略，不常驻整套引擎文档。

## Skills

按需 skill 为 `model-intake`、`study-planning`、`result-analysis`、`comparison` 和
`troubleshooting`。结果分析和比较要求先加载对应 skill；实际引用保存在资源/运行/
分析快照中。

`result-analysis` 明确要求每条 RSS 声明以分号结束，右侧必须是返回 DataFrame 的函数调用；完整复制也要根据实际
schema 使用 `select(col(...), source)`，不能生成 `derived = source` 这类 alias 直赋值。用户在同一条消息中要求先读
schema 再执行分析时，Agent 必须在读取后继续调用 `run_result_analysis`，不能只汇报 schema 后结束本轮。

## Tool authority

当前运行时允许工具共 23 个：

```text
inspect_workspace       read_model_dataframe     search_study_facades
prepare_study            search_examples
inspect_example
modify_model            publish_model_version    list_result_datasets
run_result_analysis     create_dataset_view      compare_simulations
list_simulation_history search_chat_history      read_chat_context
get_storage_usage       load_agent_skill         search_engine_docs
read_engine_doc_section
list_data_resources     read_data_resource       update_data_resource_alias
promote_result_dataset
```

Agent 可以检查工作区、查询普通数据资源、按用户明确要求修改 alias、把用户明确选定的结果提升为普通资源、修改
draft、读取运行/结果并创建派生分析或比较。规划或分析普通 Data/Struct 输入前必须先列出资源，再按稳定
`resourceId` 读取，不能从历史名称猜测。提升候选不唯一时必须让用户选择 `datasetId`。只有用户当前消息明确要求发布或
生成当前模型版本时，Agent 才能在加载 `model-intake` 并调用 `inspect_workspace` 后，以精确 `expectedDraft` 调用
`publish_model_version`；成功会沿用版本服务的去重、描述规则并自动激活。导入完成、模型修改、准备研究以及“生成脚本/结果”
均不构成发布授权，草稿并发变化时必须重新检查，不能猜测新 revision。Agent 仍不能确认导入、应用案例、激活历史版本、
删除版本/结果、创建脚本或创建运行。`prepare_study` 成功即表示本轮配置准备完成；后续 provider
收尾失败不会用通用错误覆盖已持久化的配置卡。

该发布授权同时由工具运行时强制执行：授权只来自当前 request message，且本轮 `inspect_workspace` 记录的 draft ref
必须与 `expectedDraft` 完全一致。缺少明确发布意图或本轮检查分别返回 `publish_authorization_required`、
`workspace_inspection_required`；描述超限返回 `version_description_invalid`，不会发布草稿。运行时只接受消息开头的肯定发布命令；
否定、问句、引用或“如何发布”等解释请求不构成授权，宁可要求用户用明确命令重述，也不能误发布。

研究请求遵循 `load study-planning -> inspect_workspace -> search_study_facades -> prepare_study`。只有用户原文包含完整
`ds_...` 标识时才传 `explicitFunction`；Newton/牛顿法等算法名称保留在自然语言 `query`，目录搜索按核心字段与差异
相关度返回最匹配门面，不会根据网络拓扑改写用户选择。候选不唯一时 Agent 展示 `differences`，禁用门面显示合同诊断。
`prepare_study` 创建持久化配置卡，由服务端按门面动态展开实际 model/Data/Struct/结果输入。多个兼容资源不能猜选，
同门面历史 Struct 只形成复用建议。配置卡未最终确认前，Agent 不得生成或运行受管脚本；运行时不存在绕过配置卡的旧规划或聊天脚本创建权限。

模型修改遵循 `inspect_workspace -> read_model_dataframe -> modify_model`。`read_model_dataframe` 只在加载
`model-intake` skill 后可用，scope 只能是 active 或携带准确 expected draft 的 draft；查询只允许表名、最多
32 个输出列、最多 8 个标量等值条件和 `1..=100` 行。服务端完整扫描得到准确 `matchCount`，序列化结果上限为
64 KiB。Agent 必须确认目标唯一并使用读取到的旧值，不能猜测 row key 或把截断结果当作唯一匹配。

## Stable arguments

工具只接收 session 上下文内的稳定资源 ID、expected draft、受控 patch/spec 和文本。禁止路径、URL、SQL、
shell 命令和任意图表 option。`modify_model` 使用 `tableName + rowKey + changes + expectedDraft`；
`read_model_dataframe` 使用 `scope + tableName + columns + equals + limit + expectedDraft?`；
`publish_model_version` 使用 `expectedDraft + description?`，服务端从当前消息 ID 生成幂等键，不接受 session/version/path；
`search_study_facades` 只接收查询词和可选显式函数名；`prepare_study` 只接收目录 facade ID、结构化原始意图和可选来源 workflow ID；比较使用两个 run ID。模型范围不存在、查询无效、输出过大、草稿 revision
冲突和表不存在分别使用稳定错误码，工具响应不包含 artifact 路径。

配置确认后的脚本由独立无工具 ScriptAuthor 处理，不把脚本生成能力重新开放给对话 Agent。generation 请求只含冻结合同、
冻结输入快照和按函数检索的有界引擎文档片段；repair 请求固定包含当前脚本全文、脱敏且最多 64 KiB 的 stderr、结构/结果
诊断与冻结合同，并明确要求“只输出修改后的完整脚本，不要解释”。原始 stderr、服务器路径和 secret 不进入 LLM。

## Engine knowledge

服务启动时从当前 `SPARROWZZ_DIR` 动态发现、验证并发布不可变 snapshot，索引受控文档章节、catalog 和案例。
snapshot 由本次实际读取的字节计算，不比较 eig-rs/sparrowzz commit，也不要求固定案例数量；更新资源后需重启。
Agent 先 `search_engine_docs`，再凭 section ID 读取需要的片段。案例 search/inspect 只读；应用由固定 UI 完成。

V4 动态目录基础已经实现：`functions.json=2`、`study_facades.json=1`、`dataframes.json=3` 和
`dtype_grammar=1` 分别校验，不要求版本号相等。`study_facades.json` 是研究门面的唯一入口；输入种类、参数位置、
结果和选择约束均由该文件及传递 DataFrame schema 决定，`functions.json` 不再补齐研究门面。单个门面或传递 schema
无效时只禁用受影响门面并保留稳定诊断；文件缺失时发布“没有可用研究门面”的全局诊断。Agent 常驻摘要使用
`catalogSchemaVersions` 和门面启用/禁用数量，不注入完整目录。

`StudyPlanningService::search/get` 与 `StudyConfigurationService` 直接查询当前 snapshot 的门面注册表。显式函数选择只返回该函数，不根据网络拓扑改写；候选较多时向用户展示目录中的 `differences`。配置服务按目录合同生成 revisioned 卡片；用户确认后由 `ManagedWorkflowService` 原子冻结合同、输入和 provisional run。

脚本生成使用独立 `ScriptAuthor`，不注册聊天工具，也不复用对话 Agent 的授权面。generation/repair 请求只包含冻结合同、
冻结输入、允许的引擎文档和明确指令；外部模型传输失败按 `AGENT_MODEL_RETRY_MAX` 独立重试，不创建 attempt。每份成功
返回的完整脚本先作为不可变 candidate artifact 持久化并记录父 attempt，执行与修正循环由后续 worker 使用。

引擎失败分析由独立 `FailureAnalyzer` 边界提供，研究工作流和普通运行使用同一 prompt、有界 JSON 解析和日志证据校验，不与脚本生成/
repair 接口绑定。请求只接收当前脚本、可选冻结合同/输入、受控 diagnostic 和已脱敏 stdout/stderr；普通运行的合同/输入为空。
响应必须包含 summary、带 `stderr:` 引用的 facts/possibleCauses、unknowns 和 recommendations；禁止输出思维链、恢复脱敏内容或把推断写成事实。
两条执行路径都使用 30 秒上限的 best-effort 调用；超时、解析或 provider 失败只标记 unavailable，不覆盖已持久化日志、不改变 run 终态，也不阻断 repair。

## Persistence and audit

每次 tool call 都先写 durable `tool-call` 消息再通过 SSE 发布。脚本消息仅含摘要与有界 preview；全文不进入
prompt。运行保存实际 script artifact/hash、模型 hash、Agent bundle/skill refs 和 engine contract；分析与比较
同样保存来源快照。manifest/注册表双向测试阻止隐藏工具和陈旧工具。
