# 当前仿真结果自然语言分析设计

## Intent

用户在仿真完成后应能直接用“筛选越限点”“统计各母线最大电压”“从刚才结果派生一张表”等自然语言继续处理结果。
系统向 Agent 提供当前会话最近一次成功运行的有界结果摘要；Agent 根据用户原话唯一匹配一个结果 DataFrame，生成受控 RSS
分析脚本，并继续通过现有 `run_result_analysis` 以稳定 `datasetId + alias` 执行。这样减少无关 Parquet 的复制、扫描和潜在
内存占用，同时保持输入、脚本和输出血缘可审计、可恢复。

## Goals

- 每轮对话向 Agent 提供当前会话最近一次“成功且仍有存活结果”的运行及其结果摘要。
- 摘要只包含稳定 ID、结果名、行数、大小、schema hash 和列的名称/dtype/unit；不包含文件路径或结果正文。
- 用户使用“当前、刚才、上次”等指代并提出结果处理要求时，Agent 从摘要中只选择与自然语言唯一匹配的一个 dataset。
- 唯一匹配后，Agent加载 `result-analysis`，按现有 catalog/doc 生成 RSS，并显式调用
  `run_result_analysis(inputs=[{datasetId, alias}])`。
- “越限、异常点、过滤、排序、聚合、统计、透视、关联、派生”等需要生成新 DataFrame 的要求进入结果分析链路；纯画图仍引导
  到结果工作台。
- 输入选择与分析发布继续冻结 dataset ID、schema/content hash、脚本和输出血缘。
- 上下文读取失败时安全降级：不得声称“没有运行”，而应调用历史/数据集工具确认或说明读取失败。

## Non-goals

- 不让大模型读取完整 Parquet，也不把结果正文放入 Prompt。
- 不允许服务器路径、SQL、任意命令或任意 ECharts option 进入公共边界。
- 不让 `run_result_analysis` 隐式选择 dataset；其执行参数仍必须携带稳定 dataset ID 和显式 alias。
- 不在本次增加 Agent 创建图表的权限，也不增加阈值线、markLine 或越限点高亮图表合同。
- 不改变仿真、工作流或分析的执行超时/取消合同，不增加分析脚本自动修正。
- 不根据固定名称预设电压阈值、单位、相/分量或领域指标。

## Constraints

- 所有 tensoreval 调用继续只经过 `backend/src/adapters/`。
- repository 查询继续显式使用 `UserId`；跨用户和不存在资源保持 404 语义。
- 结果事实来源仍为不可变 Snappy Parquet；浏览器和 LLM 不接触物理路径。
- Prompt 上下文必须有界。仅检查最近 20 次存活运行，并选择第一条成功且至少有一个存活 dataset 的运行；每个运行最多接受现有
  结果上限内的数据集，列摘要沿用持久化 schema，不扫描 Parquet 数据行。上下文序列化预算为 64 KiB，每个 dataset 最多包含
  前 32 个列摘要，dataset/result/column/dtype/unit 文本字段最多保留前 256 个 Unicode 字符；任何裁剪都必须设置显式
  `datasetsTruncated`、`columnsTruncated` 或字段级 `truncated` 标志。
- 显式给出的 run/dataset ID 优先于“当前结果”默认上下文，但仍需通过现有工具校验归属与存活状态。
- 工作区已有 Agent bundle hash 修改，实施时只能在现有内容上重新计算受影响资源 hash，不覆盖其他未提交文件。

## Approaches

### 1. 只修改 Prompt/skill

要求模型每次主动调用运行历史和结果列表。改动最小，但无法避免模型像本次日志一样跳过查询并错误断言“尚未运行”。未采用。

### 2. 注入服务端当前结果上下文，同时保持显式执行输入（选定）

ChatService 在组装单轮 Agent 前，从当前用户和会话解析最近成功运行的结果摘要，并交给 PromptAssembler。模型只选择唯一匹配结果，
实际分析仍显式提交稳定 ID。该方案把“当前结果是什么”变成服务端事实，同时保留现有工具、权限、血缘和错误合同。

### 3. 允许分析工具省略 inputs

由后端自动把最新运行的全部结果注入脚本。虽然调用简单，但会造成多结果歧义、额外大文件复制和隐式输入变化，也扩大公共工具合同。
未采用。

## Design

### Current result context

新增纯 DTO `CurrentResultContext` 和 `CurrentResultDatasetContext`。应用层解析流程为：

1. 按当前用户、当前会话读取最近 20 条未删除运行。
2. 按新到旧顺序只考虑 `status=succeeded` 的运行。
3. 为候选运行读取存活 datasets；第一条非空候选成为 `currentResultContext`。
4. 结果摘要按持久化顺序包含 `datasetId`、`name`、`rowCount`、`sizeBytes`、`schemaHash` 和列摘要。
5. 没有成功结果时上下文为 `null`；读取错误时也为 `null`，但系统规则要求 Agent 使用查询工具确认，不能据此断言没有运行。

该上下文只进入当次 system prompt，不写入消息，不改变历史快照，也不把完整结果卡重复注入历史。

### Matching and alias rules

- 用户明确提供 dataset ID 时使用该 ID，当前上下文只作辅助证据。
- 否则 Agent依据用户原话与 dataset 名称、列名称匹配；只有一个候选符合时才执行。
- 无候选或多个候选同样符合时，Agent列出候选名称和稳定 dataset ID，请用户选择。
- 不因为只有一个 dataset 就把与其无关的自然语言强行解释为结果分析。
- 分析 alias 优先使用合法的 dataset 名；名称不是合法标识符时使用 `source`。后端仍验证 alias，且不接受重复 alias。
- 只绑定选中的 dataset，不注入同一运行的其他结果。
- 只要唯一匹配依赖被截断的 dataset、列名、dtype 或 unit，就必须调用 `list_result_datasets` 取得完整权威元数据；不完整摘要不能
  用于排除候选或断言唯一。
- “默认结果”只选择一个 dataset；用户明确要求关联多个结果时继续使用现有最多 20 个显式输入的分析能力，逐个稳定 ID 绑定，
  不受默认单结果规则限制。

### Natural-language routing

更新系统规则和 `result-analysis` skill：

- “画图/查看图表”且没有派生要求：不运行分析，指向结果工作台。
- “标出越限点”包含派生标记或筛选要求：属于结果分析；阈值、单位或 List 分量不明确时先询问一个必要问题。
- 用户已经给出完整阈值和唯一结果时，必须在同一轮完成 `load_agent_skill -> 必要文档查询 -> run_result_analysis`，不得只给操作说明。
- 不得声称图表工作台支持未在 `ChartConfig` 中定义的阈值线或点标记。

该语义变化把 `result-analysis` skill 从 `1.1.0` 提升到 `1.2.0`；bundle 仍为 `v4.4.0`，内容身份继续由 manifest、
skill-index 和 skill SHA-256 共同校验并写入分析 provenance。

### Failure behavior

- 当前结果上下文为空：调用 `list_simulation_history` 确认；有成功运行后再调用 `list_result_datasets`。
- 上下文中的 dataset 在调用前被删除：现有 `analysis_input_not_found` 返回后说明结果已不存在，不回退到其他 dataset。
- schema 不足以确定阈值、单位或 List 分量：询问用户，不生成猜测脚本。
- 分析执行失败：沿用现有分析进度、稳定错误和无部分发布语义。

## Testing

- Rust 单元测试验证上下文选择跳过失败、运行中、已删除结果和成功但无存活结果的运行，只返回最近可用成功运行。
- Rust Prompt 测试验证有界 JSON 包含稳定结果摘要、不含路径或结果正文，并验证 `null` 降级。
- Scripted Agent 行为测试复现本次场景：仿真成功后用户要求对唯一电压结果进行明确阈值派生，工具顺序必须包含
  `load_agent_skill` 和 `run_result_analysis`，且只传一个 dataset ID。
- Scripted Agent 行为测试覆盖两个名称/列均可能匹配的结果时只询问选择，不调用分析。
- 资源合同测试验证更新后的 system/skill/index/manifest hash 一致，工具数量和 `run_result_analysis` 参数合同不变。
- 功能目录为该行为增加真实测试证据；当前路由、消息、工具、skill 数量不变。

## Documentation

同步更新：

- `docs/agent-prompt-skills-design.md`：当前结果上下文、唯一匹配和降级规则。
- `docs/development-status.md`：自然语言结果分析默认上下文能力与验证状态。
- `docs/integration-testing-design.md`：确定性行为覆盖。
- `integration-tests/catalog/features.yaml`：ANALYSIS/AGENT 对应行为证据。
