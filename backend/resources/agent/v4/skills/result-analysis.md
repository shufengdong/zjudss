# 结果分析

## 何时使用
用户明确要求筛选、排序、聚合、统计、透视、关联、越限判断或从已有结果派生新 DataFrame 时使用。只要求画图且不产生派生数据时不运行分析，应引导用户在结果工作台中配置；不得声称工作台支持合同未定义的阈值线或点标记。

## 选择输入
1. `currentResultContext` 是服务端提供的当前会话最近可用成功运行的有界结果元数据，不包含结果行或物理路径。
2. 用户明确给出 dataset ID 时优先使用该 ID，并通过现有工具校验归属和存活状态。否则根据用户原话、结果名和列元数据选择；只有唯一匹配一个 dataset 时才能执行，默认只绑定一个输入。
3. 没有匹配或存在多个合理候选时，列出候选名称和稳定 dataset ID，只询问用户选择，不生成脚本。
4. `datasetsTruncated`、`columnsTruncated` 或字段级 `truncated` 影响候选判断时，先调用 `list_result_datasets` 获取完整权威元数据；不能用被裁剪的摘要排除候选或断言唯一。
5. `currentResultContext` 为 null 只表示当前摘要不可用。先调用 `list_simulation_history`；确认成功运行后再调用 `list_result_datasets`，不得据此声称尚未运行。
6. 默认单结果规则不限制用户明确要求关联多个结果：显式多输入分析继续逐个提交稳定 dataset ID 和不重复 alias。

越限请求若未给出阈值、单位或 List 列的分量，只询问一个必要问题，不预设领域值。阈值、单位、分量和唯一输入均已明确时，在同一轮继续执行分析，不只给操作说明。

## 执行步骤
1. 从未裁剪的 `currentResultContext` 或 `list_result_datasets` 读取 dataset schema、单位和行数，再选择必要输入列。
2. 按函数 catalog 生成有界 RSS 查询脚本，以 dataset ID 映射输入 alias。
   每个声明都以分号结束，右侧必须是返回 DataFrame 的函数调用，不能写 `derived = source` 这类 alias 直赋值；完整复制应按实际 schema 使用 `derived = select(col(...), source);`。
3. 运行后把每个输出登记为新 Parquet dataset，并保留输入与脚本血缘。
4. 分析只发布 Parquet dataset；表格始终可用，图表由结果工作台独立配置和保存。

`run_result_analysis.intent` 是 1 到 1000 个字符的审计说明字符串，不参与执行。必须提交完整 JSON 对象；`script` 只能使用当前 catalog 支持的 RSS，不得使用 Python import。示例：

```json
{
  "intent": "复制当前电压结果的 time 和 v_mag 列",
  "script": "voltage_copy = select(col(time), col(v_mag), bus_voltage);",
  "inputs": [
    {
      "datasetId": 123,
      "alias": "bus_voltage"
    }
  ],
  "expectedOutputs": [
    "voltage_copy"
  ]
}
```

用户在同一条消息中同时要求读取 schema 和执行分析时，读取完成后必须继续调用 `run_result_analysis`，不得只汇报 schema 或等待用户再次要求执行。默认场景的 `inputs` 只包含唯一选中的 dataset 及其 alias，不附带同一次运行的其他结果。

## 禁止推断
不得预设固定指标，不得提交 SQL、服务器路径或任意 ECharts option。

## 验收
输出 schema、行数、预览截断标志和分析运行可从历史恢复。
