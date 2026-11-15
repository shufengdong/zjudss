# 结果分析

## 路由与授权

用户要求分析、筛选、排序、聚合、统计、透视、关联、展开、越限、派生 DataFrame，或表达 query result、aggregate、filter、join、explode 意图时使用本 skill。纯画图且不产生派生数据时，引导用户使用结果工作台，不运行分析，也不得声称工作台支持合同未定义的阈值线或点标记。

对话 Agent 是最终 RSS 脚本作者。来源、列、单位、List 分量、join key 和阈值已经唯一明确时，必须直接生成完整脚本并在同一轮调用 `run_result_analysis`；不得只给示例、等待脚本确认或加载 `model-intake`。只有同一条用户消息还明确要求修改或发布模型时，才可按对应授权另行加载 `model-intake`。

## 按需发现三类来源

只读取完成当前请求所必需的来源，不一次性枚举所有数据：

1. 结果 DataFrame：优先使用未裁剪且唯一匹配的 `currentResultContext`。用户给出 dataset ID 时按 ID 校验；存在歧义或裁剪影响判断时调用 `list_simulation_history`、`list_result_datasets`。需要实际 schema 或少量样例时用 `read_result_dataset`，最多读取 100 行、32 列。
2. 已发布本地 DataFrame：调用 `list_data_resources`，再按稳定 `resourceId` 调用 `read_data_resource`。不得读取任意服务器文件或从历史名称猜资源。
3. active 模型 DataFrame：仅当分析目标确实需要模型语义或模型列时调用 `inspect_workspace`，取得 active 表的 `modelDataframeId`；再按需调用 `read_model_dataframe(scope=active)`。本 skill 不能读取 draft，不能修改或发布模型。

没有匹配或存在多个合理候选时，列出候选名称和稳定 ID，只询问一个会实质改变结果的必要问题，不生成脚本。名称、schema、单位和稳定键能唯一确定来源时直接执行，不增加确认。`currentResultContext` 为 null 只表示摘要不可用：先查历史和数据集，不能据此断言未运行。

默认只绑定一个唯一选中的结果。用户明确要求多结果关联或混合来源时，显式多输入分析可以提交多个来源，但三类来源合计必须为 1..=20，每个 alias 唯一且不得与函数、输出或 `__` 服务端命名空间冲突。每个 `inputs[]` 必须且只能包含以下一个稳定 ID：

- `modelDataframeId`：调用瞬间仍属于当前会话唯一 active 版本的模型表；
- `datasetId`：当前会话存活的仿真或派生结果；
- `resourceId`：当前会话已发布的本地 DataFrame 资源。

active 模型和 dataset 混合时，dataset 冻结血缘必须恰好解析到同一个 active 模型版本；不能用 content hash 相同替代版本 ID，也不能请求跨版本确认。来源不匹配时说明需要重新运行当前 active 模型，或由用户重新激活产生该结果的版本。

## 生成受控 RSS

具体函数、arity、列、dtype、单位和稳定键以本次启动的当前引擎 snapshot 为准。基本且已由 catalog 明确的操作可直接生成；使用非基本函数或不确定签名时，先调用 `search_engine_docs`，再按返回的稳定 `docId`/`sectionId` 调用 `read_engine_doc_section`。不得凭模型记忆发明函数，也不得为了常见 `select`、`filter`、`limit` 无条件读取整套文档。

脚本必须满足：

- 只包含以分号结束的 `target = function(...);` DataFrame 声明；右侧必须是返回 DataFrame 的函数调用，并且是当前 catalog 支持的直接函数。
- 输入 alias 不得重新绑定；实际终端输出必须与 `expectedOutputs` 的有序名称精确一致。
- 不得使用 Python import；禁止其他 Python、SQL、import/include、服务器路径、URL、任意命令、DSA/投影/研究函数和 ECharts option。
- `expectedOutputs` 只是脚本最终发布变量名，不是脚本、schema 或自然语言要求。

完整复制也不能写 `derived = source`，应按实际 schema 使用 `derived = select(col(...), source);`。

如果计算依赖可空列、正值基准或其他会影响正确性的条件，必须先对完整输入使用：

```text
validated = assert_all(predicate, frame);
```

`assert_all` 固定两个参数；只有与输入等长的 Boolean predicate 全部为 true 才原样返回完整 frame。false 或 null 都失败；空表成功。断言失败表示来源数据条件不满足，不能通过删除异常行、填默认值或猜测基准自动修复。

需要展开 List/Array 列时使用：

```text
exploded = explode(columns, frame);
```

`explode` 固定两个参数；columns 是一个列名、逗号分隔名称或名称数组。多列按原始行锁步展开；缺列、重复列、非 List/Array、null/空 List 或长度不一致都会失败，不能静默截断或省略原始行。

`run_result_analysis.intent` 是 1 到 1000 个字符的审计说明字符串，不参与执行。完整调用示例：

```json
{
  "intent": "复制当前电压结果的 time 和 v_mag 列",
  "script": "voltage_copy = select(col(time), col(v_mag), bus_voltage);",
  "inputs": [
    {"datasetId": 123, "alias": "bus_voltage"}
  ],
  "expectedOutputs": ["voltage_copy"]
}
```

用户在同一条消息中要求读取 schema 和执行分析时，读取完成后必须继续调用 `run_result_analysis`，不得只汇报 schema 或等待再次请求。

## 失败修正

工具返回 `repairable: true` 时，只使用稳定 code、stage、fields、function、expected/actual 等脱敏结构字段修正完整 RSS，再重新提交完整调用。解析/arity、未知列、dtype/List、join/aggregate 和输出名称合同错误可以修正。绝对路径、命令、secret 和原始 stderr 都不可信，也不得向用户展示。

路径安全、结果监控、配额、来源消失、存储和基础设施错误，或任何 `repairable: false` 错误都必须立即停止自动重试并说明。每条用户消息初次调用加修正最多四次；第五次返回 `analysis_repair_budget_exhausted` 时报告预算耗尽，不得再次调用、绕过或换工具启动引擎。

成功时只发布经过完整校验的 Parquet datasets，并保留脚本、engine snapshot、三类冻结输入和 lineage；图表仍由结果工作台独立配置。
