# Agent、工具、Prompt 与知识资源维护性设计

**日期：** 2026-11-05  
**范围：** 仓库维护路线 8.7  
**状态：** 已批准（用户授权连续完成 8.7–8.9）

## 1. 目标

让 V4 bundle manifest 成为对话 Agent 工具暴露集合的唯一权威来源。Rust 代码只维护工具实现与构造器，运行时必须按已经完成 hash、skill 和 engine contract 校验的 manifest 装配工具。Prompt 中的 skill index 继续由同一份已校验 metadata 生成，不增加第二份权限清单。

本阶段保持 23 个公开工具、5 个按需 skills、稳定参数、错误码、bundle `v4.4.0`、ScriptAuthor/FailureAnalyzer 隔离和运行 provenance 不变。

## 2. 现状与问题

- `backend/resources/agent/v4/manifest.json` 手写 `allowedTools`，`backend/src/tools/mod.rs` 的宏又枚举一次实际注册工具；两者只能靠事后相等测试发现漂移。
- `build_tools` 当前直接使用 Rust 宏顺序，manifest 实际只参与知识预检，没有决定 Agent 真正获得哪些工具。
- 已从 V4 权限面移除的 `create_script` 仍以公开 Rust module 和完整实现留在源码树；虽然没有注册，仍是陈旧能力和误用入口。
- `agent/mod.rs` 保留未被调用的 `build_agent`，直接拼接未校验的 system/skill-index 资源，绕开 `AgentKnowledge` 与 `PromptAssembler` 的当前装配边界。
- 精确工具数在若干测试和文档中重复；双向门禁需要保留，但不应再让重复枚举参与生产注册。

## 3. 方案比较

### 方案 A：继续以 Rust 宏为权威并生成 manifest

优点是 Rust 类型与构造器天然可编译检查。缺点是 manifest 在源代码中不再是权威输入，需要生成并回写带 hash 的资源文件，也与当前启动预检和 AGENTS.md 对 manifest 的定位冲突。不采用。

### 方案 B：`build.rs` 从 manifest 生成完整 Rust 注册代码

优点是权限清单只有一份。缺点是 JSON 工具名无法自行推导 Rust 类型路径，仍需另一份名称到类型映射；同时把普通资源错误变成构建脚本错误并增加生成物维护。不采用。

### 方案 C：manifest 决定暴露集合，Rust catalog 只提供实现

`AgentKnowledge` 先校验 manifest、skill index、hash 和工具名唯一性；工具 catalog 只声明每个可用实现的构造器。启动时双向校验 manifest 与 catalog，`build_tools` 按 manifest 顺序逐项解析构造器。manifest 未列出的实现绝不会注册，缺实现、重复实现或名称不匹配均失败关闭。

该方案保持静态类型构造器，同时让 manifest 真正控制权限，选用此方案。

## 4. 设计

### 4.1 已校验 bundle 视图

`AgentKnowledge` 暴露只读的 bundle version、allowed tool names 和 skill metadata。加载时新增以下约束：

- `allowedTools` 非空、名称非空且唯一；
- skill ID、skill path 和 required tools 继续校验；
- skill required tools 只能引用同一 manifest 的 allowed tools。

PromptAssembler 只从这个 `AgentKnowledge` 生成 skill index，不读取原始 JSON，也不维护工具权限副本。

### 4.2 工具实现 catalog

工具模块维护一个私有实现 catalog。每个条目包含由 `Tool::NAME` 得到的名称和构造器；它不是权限清单，只说明二进制中有哪些可实例化实现。

提供两个操作：

1. `validate_tool_catalog(manifest)`：检查实现名称唯一、manifest 与实现 catalog 双向相等；
2. `build_tools(services)`：按 `services.agent_knowledge.manifest.allowed_tools` 顺序逐项查找构造器并注册。

应用启动和测试应用装配必须在接受 bundle 后运行校验；构造阶段对理论上不可能出现的缺失实现仍失败关闭。

### 4.3 删除陈旧能力

删除 `tools/create_script.rs` 和 module 导出。通用/受管脚本仍由上传、固定 UI 和隔离的 ScriptAuthor 路径创建；对话 Agent 不获得脚本创建权限。

删除 `agent/mod.rs` 中未使用的 `SYSTEM_PROMPT`、`SKILL_INDEX`、`build_agent` 和 `build_agent_with_prompt`。所有对话 Agent 都继续通过 application chat 的 `AgentKnowledge + PromptAssembler + AgentFactory` 装配；ScriptAuthor 与 FailureAnalyzer 只向 factory 传各自专用 prompt，边界不变。

### 4.4 合同与 provenance

- `/api/contracts` 的工具列表直接来自已校验 manifest，不从实现 catalog 另推导。
- run/analysis/comparison 继续保存实际 bundle version、skill refs 和 engine contract；本阶段不改变格式。
- 保留 manifest 与实现 catalog 的双向行为门禁，并增加 manifest 重复工具、缺实现、额外实现的失败用例。
- 23 个工具与现有行为均不变，因此不提升 bundle 版本，不改变功能目录公共合同。

## 5. 非目标

- 不合并语义不同的工具，不扩大任何工具参数。
- 不把 ECharts option、路径、SQL、命令或任意脚本重新开放给 Agent。
- 不让 ScriptAuthor/FailureAnalyzer 使用对话工具。
- 不修改 HTTP/SSE、数据库、前端或 engine adapter 合同。

## 6. 验收

- manifest 是生产注册顺序和暴露集合的唯一来源。
- manifest/catalog 任一方向漂移都在启动或聚焦测试中失败关闭。
- 源码不存在 V4 `create_script` 工具实现和旧的未校验 Agent prompt 装配入口。
- Agent 知识、工具、factory 和 contract manifest 聚焦测试通过；格式、Clippy 和完整后端测试不新增失败。

