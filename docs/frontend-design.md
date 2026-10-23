# 配电网仿真智能体 v2 前端设计

> 状态：已实现。HTTP 契约以 [api-design.md](api-design.md) 为准，TypeScript 出网类型以
> `frontend/src/types/` 为准。

## 1. 交互原则

模型导入后的修改、激活、仿真、结果查询和比较默认通过自然语言完成。固定按钮只用于高频且无歧义的
动作：会话管理、上传、只读案例浏览、历史分页、结果标签切换和文件下载。领域快捷按钮会把完整文本
作为可见 user 消息发送，用户和历史回放都能看到实际请求。

## 2. 页面结构

```text
历史侧栏 | 对话与 Composer | 模型/任务工作台
                         └─ DataFrame/比较详情层
```

- 历史侧栏参考常见对话产品：最近会话分组、服务端全文搜索、重命名和删除。
- 对话区渲染八类持久消息；顶部可打开案例库和右侧工作台。
- 右侧在“模型工作区 / 仿真历史”间切换，桌面可调整宽度，窄屏为抽屉。
- DataFrame 使用独立详情层，避免大表进入聊天消息状态。

## 3. 状态和恢复

`App` 只管理布局和选中 ID。领域状态由 hooks 管理：

- `useSessionFeed`：按 session ID 键控消息；先打开 SSE，再读最近快照；按稳定键归并；支持 older 页。
- `useWorkspace`：合并 REST 快照和 model-status active/draft 事件。
- `useIngestion`：本地 queued/uploading 与服务端 batch/file 状态。
- `useExamples`：只读案例搜索。
- `useSimulationHistory/useTaskDatasets/useDataset/useDatasetView/useComparison`：结果工作台。
- `useStorageUsage`：80% 告警。

启动先读取 `/api/me`。导航偏好键为 `zjudss:last-session:<scopeKey>`；若该 session 已不可访问则
打开当前列表第一项。localStorage 不保存聊天、模型或授权信息。

## 4. 消息归并

- 普通消息按 message ID 去重排序。
- 乐观 user 消息收到真实 user SSE 后替换。
- progress 按 `operation.kind + operation.id` 替换。
- tool-call 按 callId 替换。
- attachment 按 batch/file 替换。
- model-status 按 workspace/scope 替换；`snapshot:null` 清空 draft。
- result 按 task/analysis producer 更新 outputs。
- comparison 按 comparisonId 恢复。

旧 session 的异步响应写入其自己的 keyed state，不会覆盖当前会话。

## 5. 上传和案例

文件、多文件和目录使用一个 multipart 请求。每次重新选择生成新的 batch ID 和 file ID；目录读取
`webkitRelativePath` 并发送规范相对路径。硬配额或上传失败时保留队列和输入内容，允许用户修正后
重新上传。

案例库显示标题、source kind、study type、文件数、验证状态和 required features。“使用此案例”会
发送含稳定 example ID 的可见自然语言消息；导入与修改均由 agent 工具创建不可变版本。

## 6. 模型工作区

active 和 draft 同时展示：

- 版本 ID、状态、source/origin；
- 输入 ID、文件/sheet、table kind、confidence 和 resolution；
- 匹配/缺失列、dtype 冲突、ambiguity 和诊断；
- 活动 ingestion。

激活候选、替换或移除输入的建议按钮只填充并发送自然语言。已完成任务继续绑定旧版本。

## 7. DataFrame 和图表

每张数据集必须有分页表格。列顺序来自 metadata schema，null 显示为“—”，cursor 页按顺序追加，
合法重复行不会被删除。下载始终保留 Parquet。

保存的 `DatasetView` 由本地注册表编译：

| kind | 必需字段 |
|---|---|
| line/bar/scatter | x、至少一个 y，可选 series |
| heatmap | x、category、value |
| boxplot | category、value |
| histogram | value、bins |

builder 只输出固定 ECharts 属性，单位进入坐标轴，直方图保留 bin 边界，禁止 formatter 和远程资源。
422 规模错误展示 estimated/limit，并建议用户在对话中要求筛选或聚合；局部失败不清空聊天或工作区。

## 8. 任务和比较

任务历史显示 task ID、study type、状态、绑定 model version 和时间。选中两个任务后，“在对话中比较”
发送可见消息，由 agent 校验结果名、稳定键、dtype、单位和 QSTS 时间键。comparison 消息只保存稳定
ID；刷新后从比较 API 恢复兼容性和差值 DataFrame。

## 9. 验证

Vitest 覆盖 session keyed 状态、workspace null 快照、SSE 控制事件、schema 列序、null、合法重复行、
六类图表和直方图边界。基础门禁运行 `npm test -- --run` 与 `npm run build`。

完整前端验收由 12 个 Playwright 文件中的 18 个浏览器用例从浏览器访问 Vite 和真实 Axum e2e server，
覆盖上传、自然语言工具调用、SSE、工作区、任务、表格、六类图表、比较、历史恢复、多人隔离及
桌面/窄屏布局。测试不直接写 React state，也不以 mock HTTP 响应代替关键用户旅程；测试控制 API 只用于
编排 LLM/engine 场景、故障、并发 barrier 和容量边界。实现状态和 100% 功能覆盖口径见
[integration-testing-design.md](integration-testing-design.md)。
