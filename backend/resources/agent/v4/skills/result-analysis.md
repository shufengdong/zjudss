# 结果分析

## 何时使用
用户明确要求筛选、聚合、透视、关联或从已有结果派生新 DataFrame 时使用。只要求画图时不运行分析，应引导用户在结果工作台中配置。

## 执行步骤
1. 先读取 dataset schema、单位和行数，再选择必要输入列。
2. 按函数 catalog 生成有界 RSS 查询脚本，以 dataset ID 映射输入 alias。
   每个声明都以分号结束，右侧必须是返回 DataFrame 的函数调用，不能写 `derived = source` 这类 alias 直赋值；完整复制应按实际 schema 使用 `derived = select(col(...), source);`。
3. 运行后把每个输出登记为新 Parquet dataset，并保留输入与脚本血缘。
4. 分析只发布 Parquet dataset；表格始终可用，图表由结果工作台独立配置和保存。

用户在同一条消息中同时要求读取 schema 和执行分析时，读取完成后必须继续调用 `run_result_analysis`，不得只汇报 schema 或等待用户再次要求执行。

## 禁止推断
不得预设固定指标，不得提交 SQL、服务器路径或任意 ECharts option。

## 验收
输出 schema、行数、预览截断标志和分析运行可从历史恢复。
