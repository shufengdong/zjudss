# V3 Agent Prompt、Skills 与工具

## Bundle

生产 bundle 位于 `backend/resources/agent/v3/`，版本 `v3.3.3`。manifest 固定 system prompt、skill index、
engine contract、允许工具和文件 SHA-256；启动时完整验证。Agent prompt 只常驻策略，不常驻整套引擎文档。

## Skills

按需 skill 为 `model-intake`、`study-planning`、`rss-generation`、`result-analysis`、`comparison` 和
`troubleshooting`。有副作用的脚本生成、结果分析和比较分别要求先加载对应 skill；实际引用保存在资源/运行/
分析快照中。

## Tool authority

允许工具共 18 个：

```text
inspect_workspace       read_model_dataframe     plan_study
search_examples
inspect_example
modify_model            create_script            list_result_datasets
run_result_analysis     create_dataset_view      compare_simulations
list_simulation_history search_chat_history      read_chat_context
get_storage_usage       load_agent_skill         search_engine_docs
read_engine_doc_section
```

Agent 可以检查工作区、修改 draft、创建脚本、读取运行/结果并创建派生分析或比较。Agent 不能确认导入、应用案例、
发布/激活/删除版本、删除结果或创建运行。生成脚本后的回复必须提示用户检查代码并点击脚本卡片“运行”；tool 调用
成功即表示本轮脚本生成任务完成，但不代表仿真发生。`create_script` 成功后如果后续模型收尾失败，服务端发出确定性的检查/运行提示，
不再用通用 Agent 失败消息覆盖已持久化的脚本成功。

潮流、QSTS、短路、谐波和暂态请求统一遵循
`inspect_workspace -> load study-planning -> plan_study`。规划只读取当前用户、当前会话的 active 模型，结合
拓扑、设备、引用关系与本次启动的函数 catalog 返回 `ready`、`needs_clarification`、`missing_model_data` 或
`unsupported`。后两类缺失状态必须在一条回复中覆盖全部缺失项；不允许从案例或历史运行补参数。当前暂态只支持
`ds_motor_dynamics`，EMT、暂态稳定和逆变器控制暂态明确返回不支持。

`plan_study.request` 的字段使用 camelCase，并拒绝未知字段，避免拼写错误静默落入默认值。用户明确指定 Newton
或 Radial 时，Agent 必须分别传 `solverPreference=newton` 或 `radial`；只有未指定求解器时才传 `auto`。
Newton 是通用求解器，也可用于径向网络；显式 Newton 请求不得因拓扑径向而降级或要求用户先修改模型。

只有 `ready` 结果签发请求内有效的 opaque `studyPlanId`。Agent 随后加载 `rss-generation`、按需读取引擎文档，
并使用计划给出的函数、arity 和结果生成脚本。`inspect_workspace.active.tables[].tableName` 是当前 active 版本
实际可用的 DataFrame alias；`plan_study` 将同一清单稳定排序后作为 `inputAliases`，并将其中规范模型输入按
adapter 合同排列为 `modelInputOrder`，写入响应和 ready plan，与 model version、content hash、engine snapshot
一起冻结。规范研究输入缺失时，规划器一次返回所有
`model.tables.<alias>` 缺失项，不签发 plan ID。Prompt 和 skill 不保存固定表名清单。

运行时按表名向脚本注入 active DataFrame。受管脚本直接引用 `inputAliases` 并按 `modelInputOrder` 放置规范模型
参数，可以用新变量派生参数和保存结果。静态潮流不得再查询模型输入顺序；需要确认参数或结果语法时最多执行
一次文档搜索和一次章节读取，避免在保存脚本前耗尽 8 次模型调用预算。DataFrame 模式只接受变量声明；函数
结果绑定中的未消费叶变量就是结果，脚本末尾不得追加裸变量表达式。静态校验以
`script_bare_output_rejected` 在持久化前拒绝该语法。受管脚本
不得重新导入、读取模型文件或覆盖输入 alias。`create_script` 在持久化前核对 plan ID、active 模型 hash、引擎
snapshot、实际函数、静态校验、输入集合和输出绑定；失败返回 `script_error`，不保存 artifact、脚本记录或脚本
消息。计划或模型变化后旧 ID 失效，且任何研究脚本创建仍不产生运行。手工文本和上传脚本维持既有兼容行为。

模型修改遵循 `inspect_workspace -> read_model_dataframe -> modify_model`。`read_model_dataframe` 只在加载
`model-intake` skill 后可用，scope 只能是 active 或携带准确 expected draft 的 draft；查询只允许表名、最多
32 个输出列、最多 8 个标量等值条件和 `1..=100` 行。服务端完整扫描得到准确 `matchCount`，序列化结果上限为
64 KiB。Agent 必须确认目标唯一并使用读取到的旧值，不能猜测 row key 或把截断结果当作唯一匹配。

## Stable arguments

工具只接收 session 上下文内的稳定资源 ID、expected draft、受控 patch/spec 和文本。禁止路径、URL、SQL、
shell 命令和任意图表 option。`modify_model` 使用 `tableName + rowKey + changes + expectedDraft`；
`read_model_dataframe` 使用 `scope + tableName + columns + equals + limit + expectedDraft?`；
`plan_study` 响应使用 `inputAliases` 暴露本次 active 版本实际表名，使用 `modelInputOrder` 暴露其中规范模型输入
的动态调用顺序；`create_script` 使用
`name + content + studyPlanId?`，其中受管研究函数必须提供本次 ready 计划的 ID；比较使用两个 run ID。模型范围不存在、查询无效、输出过大、草稿 revision
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

`StudyPlanningService::search/get` 直接查询当前 snapshot 的门面注册表。显式函数选择只返回该函数，不根据网络拓扑
改写；候选较多时向用户展示目录中的 `differences`。旧 `plan_study` DTO 在 V4 受管研究入口完成前仍作为过渡适配器，
其函数、arity、模型输入顺序与结果集合也只读取同一个门面注册表。

## Persistence and audit

每次 tool call 都先写 durable `tool-call` 消息再通过 SSE 发布。脚本消息仅含摘要与有界 preview；全文不进入
prompt。运行保存实际 script artifact/hash、模型 hash、Agent bundle/skill refs 和 engine contract；分析与比较
同样保存来源快照。manifest/注册表双向测试阻止隐藏工具和陈旧工具。
