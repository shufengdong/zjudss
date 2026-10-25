# V3 Agent Prompt、Skills 与工具

## Bundle

生产 bundle 位于 `backend/resources/agent/v3/`，版本 `v3.0.0`。manifest 固定 system prompt、skill index、
engine contract、允许工具和文件 SHA-256；启动时完整验证。Agent prompt 只常驻策略，不常驻整套引擎文档。

## Skills

按需 skill 为 `model-intake`、`study-planning`、`rss-generation`、`result-analysis`、`comparison` 和
`troubleshooting`。有副作用的脚本生成、结果分析和比较分别要求先加载对应 skill；实际引用保存在资源/运行/
分析快照中。

## Tool authority

允许工具共 16 个：

```text
inspect_workspace       search_examples          inspect_example
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

## Stable arguments

工具只接收 session 上下文内的稳定资源 ID、expected draft、受控 patch/spec 和文本。禁止路径、URL、SQL、
shell 命令和任意图表 option。`modify_model` 使用 `tableName + rowKey + changes + expectedDraft`；
`create_script` 使用 `name + content`；比较使用两个 run ID。

## Engine knowledge

服务启动时从当前 `SPARROWZZ_DIR` 动态发现、验证并发布不可变 snapshot，索引受控文档章节、catalog 和案例。
snapshot 由本次实际读取的字节计算，不比较 eig-rs/sparrowzz commit，也不要求固定案例数量；更新资源后需重启。
Agent 先 `search_engine_docs`，再凭 section ID 读取需要的片段。案例 search/inspect 只读；应用由固定 UI 完成。

## Persistence and audit

每次 tool call 都先写 durable `tool-call` 消息再通过 SSE 发布。脚本消息仅含摘要与有界 preview；全文不进入
prompt。运行保存实际 script artifact/hash、模型 hash、Agent bundle/skill refs 和 engine contract；分析与比较
同样保存来源快照。manifest/注册表双向测试阻止隐藏工具和陈旧工具。
