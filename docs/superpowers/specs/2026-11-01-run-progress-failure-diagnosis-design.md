# 运行进度与研究工作流共享失败诊断设计

**日期：** 2026-11-01

## 背景

研究工作流卡片已经能在执行失败后直接展示脱敏引擎日志和大模型分析，但普通脚本运行的“运行进度”卡片仍只显示阶段失败。用户必须再打开运行记录，才能看到失败原因和 tensoreval 输出；两处展示与后端诊断逻辑也容易逐渐分叉。

## 目标

- 普通运行失败后，在原“运行进度”卡片内直接展示与研究工作流一致的失败诊断。
- 两类执行共用后端日志整理与大模型分析合同，共用前端失败诊断面板。
- 失败诊断持久化，刷新页面或恢复历史消息后仍可读取。
- 日志优先：大模型不可用、超时或输出非法时，不能影响运行终态，也不能遮蔽已保存的脱敏日志。

## 非目标

- 不给成功运行生成诊断。
- 不回填历史运行的大模型分析；旧记录继续展示已有 `engineOutput`，并提示智能分析不可用。
- 不公开原始审计 artifact、服务器路径、命令、SQL、密钥或模型思维链。
- 不给普通运行增加自动修正记录；该部分仍只属于研究工作流。

## 方案选择

采用“共享后端失败分析管线 + 共享前端展示组件”。不在 `GET /api/runs/{id}` 时临时调用大模型，也不在两类执行路径中复制提示词、解析和展示代码。

原因：失败发生时生成一次并持久化，刷新后的内容稳定；共享合同可以统一长度限制、日志证据校验、脱敏和降级行为；前端则能保持相同的信息结构与交互。

## 后端设计

### 共享失败分析器

从当前 `ScriptAuthor` 中抽出独立的失败分析能力，形成共享 `FailureAnalyzer` 边界。研究工作流和普通运行只负责构造受控请求，不各自维护提示词和 JSON 校验。

共享请求包含：

- 已脱敏、已限长的 stdout/stderr；
- 稳定错误码、失败阶段、退出码和受控 adapter diagnostic；
- 当前脚本文本；
- 可选的冻结研究合同和输入摘要。

普通运行没有研究合同也可以分析；研究工作流继续提供其冻结合同和输入快照。分析器输出沿用当前受控结构：

```json
{
  "summary": "一句话结论",
  "facts": [{ "statement": "已确认事实", "evidence": "stderr: ..." }],
  "possibleCauses": [{ "statement": "可能原因", "evidence": "stderr: ..." }],
  "unknowns": ["仍需确认的信息"],
  "recommendations": ["下一步动作"]
}
```

日志证据必须逐字存在于已脱敏日志中；字段长度、条目数量和总输出继续受限。provider 报错和非法输出只转换为 unavailable 状态，不把 provider 原文返回给浏览器。

### 共享诊断组装

新增应用层共享组装函数，把 adapter 错误转换为同一份公开诊断：

- `code`、`message`、`stage`、`exitCode`、`durationMs`；
- `engineLogs`：按 stdout/stderr 保存文本、总字节、发送字节和截断标记；
- `analysis`：可选结构化分析；
- `analysisStatus`：`pending`、`complete` 或 `unavailable`。

两条业务链继续自行决定终态和修正策略；共享层只负责诊断，不接管 workflow 状态机或普通 run 状态机。

### 普通运行持久化顺序

普通运行失败时按以下顺序处理：

1. 使用现有 `application::diagnostics` 对 adapter 捕获流进行脱敏和限长。
2. 先把基础诊断写入 `simulation_runs.error_json`，将 run 原子置为 `failed`；此时日志已经可恢复，`analysisStatus` 为 `pending`。
3. 立即写入失败的 durable progress 消息，使卡片不再停留在运行中。
4. 在有界时间内调用共享分析器；无论成功或失败，都通过受限的 repository 操作把 `analysisStatus` 从 `pending` 单向更新为 `complete` 或 `unavailable`，基础日志和终态不可改写。

该 enrichment 更新只允许作用于当前用户、未删除且已经失败的 run，并且只能完成一次 pending 分析，不开放通用终态修改接口。服务在步骤 2 后中断时，恢复逻辑把遗留 `pending` 归一为 `unavailable`，不会让界面永久显示“分析中”。

### 公共 API

继续复用 `GET /api/runs/{id}`，不新增路由。失败 run 的 `error` 增加可选字段：

```json
{
  "code": "run_failed",
  "message": "仿真引擎执行失败",
  "stage": "run",
  "engineLogs": [
    {
      "stream": "stderr",
      "text": "...",
      "totalBytes": 137,
      "sentBytes": 137,
      "truncated": false
    }
  ],
  "analysisStatus": "complete",
  "analysis": {
    "summary": "...",
    "facts": [],
    "possibleCauses": [],
    "unknowns": [],
    "recommendations": []
  }
}
```

`exitCode` 和时间字段继续使用 run 顶层现有字段。API 出网层再次执行字段白名单、限长和脱敏。旧 `engineOutput` 记录保持兼容，并映射为单条 stderr 展示；不会误称为完整原始日志。

durable progress 消息仍只保存稳定 `operation.kind=run` 和 run ID，不复制日志或分析内容。这样旧消息可以直接受益于新 run detail，也避免在消息表中保存第二份诊断。

## 前端设计

### 共享组件与类型

把当前 `WorkflowProgressCard` 内的失败区域提取为 `FailureDiagnosisPanel`，并把 `FailureAnalysis`、`EngineLog`、诊断状态等类型移到通用失败诊断类型文件。

共享面板负责：

- 错误标题、错误码、阶段、退出码和耗时；
- stdout/stderr 原格式展示；
- 当前已保留脱敏日志的下载和截断提示；
- 大模型分析的“已确认事实、可能原因、仍未知、建议动作”；
- `pending`、`unavailable` 和无日志时的明确降级提示。

`WorkflowProgressCard` 继续负责工作流计数、取消、重新尝试和自动修正记录；`ProgressCard` 继续负责普通阶段列表。两者只向共享面板传入各自整理后的诊断数据。

### 运行进度卡行为

当 `operation.kind === 'run'` 且阶段进入 failed 时，`ProgressCard` 使用现有 run ID 调用 `GET /api/runs/{id}`：

- 首次失败以及刷新恢复历史时读取一次；
- `analysisStatus=pending` 时显示“正在生成智能分析”，只在 pending 期间短暂轮询 run detail，完成或不可用后立即停止；
- 读取失败时保留阶段列表并显示可重试的详情加载提示，不把网络错误误报为引擎错误；
- ingestion 和 analysis 类型的进度卡不请求 run detail。

失败诊断直接展开显示，与研究工作流一致，不要求用户再到运行记录展开。运行记录本身继续保留，且复用相同的错误解析类型，避免两处对 API 字段理解不同。

## 错误与恢复语义

- 无引擎输出：显示“本次失败没有可用的引擎输出”。
- 分析不可用：日志照常显示，并显示“智能分析暂不可用”。
- 分析尚未完成：显示“正在生成智能分析”，不提前宣称不可用。
- 旧记录：展示旧 `engineOutput`；分析显示不可用。
- 服务重启：已经持久化的基础日志仍可见；遗留 pending 被恢复为 unavailable。
- 脱敏或截断：下载内容与 API 展示内容一致，并明确它是当前保留的脱敏片段。

## 测试与文档

### 后端

- 普通 adapter 失败持久化 stdout/stderr、元数据、退出码和阶段。
- 分析成功时持久化结构化结果；引用无法在日志中找到时拒绝该分析。
- 分析器失败、超时或返回非法 JSON 时 run 仍失败且日志可见。
- pending enrichment 只能单向完成一次，不能改写终态或基础日志。
- 服务恢复把遗留 pending 变为 unavailable。
- API 对新旧错误格式均兼容，并验证路径、控制字符、密钥和超长文本不会出网。
- 研究工作流继续通过同一分析器并保持原修正行为。

### 前端

- `FailureDiagnosisPanel` 覆盖日志、下载、截断、四类分析内容及三种分析状态。
- failed run progress 自动读取详情并内联展示诊断。
- pending 分析完成或不可用后停止详情轮询。
- 刷新恢复的 failed progress 消息仍展示诊断。
- 非 run 进度不调用 run API；详情读取失败时阶段列表仍可用。
- `WorkflowProgressCard` 改用共享面板后原有失败诊断和自动修正记录行为不变。

同步更新 `docs/api-design.md`、`docs/frontend-design.md`、`docs/agent-prompt-skills-design.md`、`docs/integration-testing-design.md`、`docs/development-status.md` 和功能目录；公共路由数量不变，只扩展失败 run 的可选 DTO 字段与行为覆盖。

## 验收标准

1. 普通运行失败后，“运行进度”卡片不跳转即可看到具体脱敏 tensoreval 日志和结构化大模型分析。
2. 刷新页面后诊断仍存在；旧记录可降级展示。
3. 大模型故障不影响 run 失败落库、失败消息或日志展示。
4. 研究工作流与普通运行使用同一个后端分析合同和同一个前端诊断面板，没有复制提示词、解析器或整块 UI。
5. 两类卡片保留各自业务差异：工作流有修正记录，普通运行没有。
6. 后端、前端、行为测试、功能目录和当前设计文档同步通过。
