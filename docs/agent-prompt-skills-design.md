# V3 Agent Prompt、Skills 与工具

## Bundle

生产 bundle 位于 `backend/resources/agent/v3/`，版本 `v3.2.0`。manifest 固定 system prompt、skill index、
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

只有 `ready` 结果签发请求内有效的 opaque `studyPlanId`。Agent 随后加载 `rss-generation`、按需读取引擎文档，
并使用计划给出的函数、arity 和结果生成脚本；`create_script` 在持久化前核对 plan ID、active 模型 hash、引擎
snapshot 与实际函数。计划或模型变化后旧 ID 失效，且任何研究脚本创建仍不产生运行。

模型修改遵循 `inspect_workspace -> read_model_dataframe -> modify_model`。`read_model_dataframe` 只在加载
`model-intake` skill 后可用，scope 只能是 active 或携带准确 expected draft 的 draft；查询只允许表名、最多
32 个输出列、最多 8 个标量等值条件和 `1..=100` 行。服务端完整扫描得到准确 `matchCount`，序列化结果上限为
64 KiB。Agent 必须确认目标唯一并使用读取到的旧值，不能猜测 row key 或把截断结果当作唯一匹配。

## Stable arguments

工具只接收 session 上下文内的稳定资源 ID、expected draft、受控 patch/spec 和文本。禁止路径、URL、SQL、
shell 命令和任意图表 option。`modify_model` 使用 `tableName + rowKey + changes + expectedDraft`；
`read_model_dataframe` 使用 `scope + tableName + columns + equals + limit + expectedDraft?`；
`create_script` 使用 `name + content + studyPlanId?`；其中受管研究函数必须提供本次 ready 计划的 ID；比较使用两个 run ID。模型范围不存在、查询无效、输出过大、草稿 revision
冲突和表不存在分别使用稳定错误码，工具响应不包含 artifact 路径。

## Engine knowledge

服务启动时从当前 `SPARROWZZ_DIR` 动态发现、验证并发布不可变 snapshot，索引受控文档章节、catalog 和案例。
snapshot 由本次实际读取的字节计算，不比较 eig-rs/sparrowzz commit，也不要求固定案例数量；更新资源后需重启。
Agent 先 `search_engine_docs`，再凭 section ID 读取需要的片段。案例 search/inspect 只读；应用由固定 UI 完成。

## Persistence and audit

每次 tool call 都先写 durable `tool-call` 消息再通过 SSE 发布。脚本消息仅含摘要与有界 preview；全文不进入
prompt。运行保存实际 script artifact/hash、模型 hash、Agent bundle/skill refs 和 engine contract；分析与比较
同样保存来源快照。manifest/注册表双向测试阻止隐藏工具和陈旧工具。
