# 结果分析

## 何时使用
用户要求筛选、聚合、透视、关联或从已有结果生成图表数据时使用。

## 执行步骤
1. 先读取 dataset schema、单位和行数，再选择必要输入列。
2. 按函数 catalog 生成有界 RSS 查询脚本，以 dataset ID 映射输入 alias。
3. 运行后把每个输出登记为新 Parquet dataset，并保留输入与脚本血缘。
4. 表格始终可用；图表只生成受控 VisualizationSpec。

## 禁止推断
不得预设固定指标，不得提交 SQL、服务器路径或任意 ECharts option。

## 验收
输出 schema、行数、预览截断标志、视图和分析运行可从历史恢复。
