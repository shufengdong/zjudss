# V4 Agent Prompt、Skills 与工具

## Bundle

生产 bundle 位于 `backend/resources/agent/v4/`，版本 `v4.0.0`。manifest 固定 system prompt、skill index、
engine contract、允许工具和文件 SHA-256；启动时完整验证。Agent prompt 只常驻策略，不常驻整套引擎文档。

## Skills

按需 skill 为 `model-intake`、`study-planning`、`result-analysis`、`comparison` 和
`troubleshooting`。结果分析和比较要求先加载对应 skill；实际引用保存在资源/运行/
分析快照中。

## Tool authority

当前运行时允许工具共 22 个：

```text
inspect_workspace       read_model_dataframe     search_study_facades
prepare_study            search_examples
inspect_example
modify_model            list_result_datasets
run_result_analysis     create_dataset_view      compare_simulations
list_simulation_history search_chat_history      read_chat_context
get_storage_usage       load_agent_skill         search_engine_docs
read_engine_doc_section
list_data_resources     read_data_resource       update_data_resource_alias
promote_result_dataset
```

Agent 可以检查工作区、查询普通数据资源、按用户明确要求修改 alias、把用户明确选定的结果提升为普通资源、修改
draft、读取运行/结果并创建派生分析或比较。规划或分析普通 Data/Struct 输入前必须先列出资源，再按稳定
`resourceId` 读取，不能从历史名称猜测。提升候选不唯一时必须让用户选择 `datasetId`。Agent 不能确认导入、应用案例、
发布/激活/删除版本、删除结果、创建脚本或创建运行。`prepare_study` 成功即表示本轮配置准备完成；后续 provider
收尾失败不会用通用错误覆盖已持久化的配置卡。

研究请求遵循 `load study-planning -> inspect_workspace -> search_study_facades -> prepare_study`。显式函数名优先，目录搜索不会根据网络拓扑改写用户选择；候选不唯一时 Agent 展示 `differences`，禁用门面显示合同诊断。`prepare_study` 创建持久化配置卡，由服务端按门面动态展开实际 model/Data/Struct/结果输入。多个兼容资源不能猜选，同门面历史 Struct 只形成复用建议。配置卡未最终确认前，Agent 不得生成或运行受管脚本。旧 `plan_study` 工具及请求内 plan ID 已从运行时删除。

模型修改遵循 `inspect_workspace -> read_model_dataframe -> modify_model`。`read_model_dataframe` 只在加载
`model-intake` skill 后可用，scope 只能是 active 或携带准确 expected draft 的 draft；查询只允许表名、最多
32 个输出列、最多 8 个标量等值条件和 `1..=100` 行。服务端完整扫描得到准确 `matchCount`，序列化结果上限为
64 KiB。Agent 必须确认目标唯一并使用读取到的旧值，不能猜测 row key 或把截断结果当作唯一匹配。

## Stable arguments

工具只接收 session 上下文内的稳定资源 ID、expected draft、受控 patch/spec 和文本。禁止路径、URL、SQL、
shell 命令和任意图表 option。`modify_model` 使用 `tableName + rowKey + changes + expectedDraft`；
`read_model_dataframe` 使用 `scope + tableName + columns + equals + limit + expectedDraft?`；
`search_study_facades` 只接收查询词和可选显式函数名；`prepare_study` 只接收目录 facade ID、结构化原始意图和可选来源 workflow ID；比较使用两个 run ID。模型范围不存在、查询无效、输出过大、草稿 revision
冲突和表不存在分别使用稳定错误码，工具响应不包含 artifact 路径。

## Engine knowledge

服务启动时从当前 `SPARROWZZ_DIR` 动态发现、验证并发布不可变 snapshot，索引受控文档章节、catalog 和案例。
snapshot 由本次实际读取的字节计算，不比较 eig-rs/sparrowzz commit，也不要求固定案例数量；更新资源后需重启。
Agent 先 `search_engine_docs`，再凭 section ID 读取需要的片段。案例 search/inspect 只读；应用由固定 UI 完成。

V4 动态目录基础已经实现：`functions.json=2`、`study_facades.json=1`、`dataframes.json=3` 和
`dtype_grammar=1` 分别校验，不要求版本号相等。`study_facades.json` 是研究门面的唯一入口；输入种类、参数位置、
结果和选择约束均由该文件及传递 DataFrame schema 决定，`functions.json` 不再补齐研究门面。单个门面或传递 schema
无效时只禁用受影响门面并保留稳定诊断；文件缺失时发布“没有可用研究门面”的全局诊断。Agent 常驻摘要使用
`catalogSchemaVersions` 和门面启用/禁用数量，不注入完整目录。

`StudyPlanningService::search/get` 与 `StudyConfigurationService` 直接查询当前 snapshot 的门面注册表。显式函数选择只返回该函数，不根据网络拓扑改写；候选较多时向用户展示目录中的 `differences`。配置服务按目录合同生成 revisioned 卡片并准备 Task 9 原子确认所需的锁定合同和输入快照。

## Persistence and audit

每次 tool call 都先写 durable `tool-call` 消息再通过 SSE 发布。脚本消息仅含摘要与有界 preview；全文不进入
prompt。运行保存实际 script artifact/hash、模型 hash、Agent bundle/skill refs 和 engine contract；分析与比较
同样保存来源快照。manifest/注册表双向测试阻止隐藏工具和陈旧工具。
