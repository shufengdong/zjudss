# Agent、工具、Prompt 与知识资源维护性实施计划

> **For Codex:** 使用 `superpowers:executing-plans` 逐项实施；每个行为改动先写失败测试并观察 RED，再写最小实现。

**Goal:** 使已校验 V4 manifest 真正决定对话 Agent 工具注册，删除陈旧脚本工具与旧 prompt 装配入口，同时保持所有公共行为不变。

**Architecture:** `AgentKnowledge` 校验和暴露 bundle 权限；`tools` 私有 catalog 仅映射静态实现，启动双向核对后按 manifest 装配；Prompt 和 contract manifest 只读取已校验知识对象。

**Tech Stack:** Rust、Axum、rig、Serde、现有 V4 bundle JSON/Markdown。

## Global Constraints

- 直接在 `master` 修改，不创建 worktree。
- 保持 23 个工具、5 个 skills、bundle `v4.4.0`、HTTP/SSE、数据库和稳定错误码不变。
- 工具仍只接受稳定 ID 与受控参数；ScriptAuthor/FailureAnalyzer 无对话工具。
- 所有编辑用 `apply_patch`；提交时不包含用户无关改动。
- 每项先跑聚焦测试；阶段末跑 fmt、Clippy 和后端完整套件，并如实保留已知外部 eig-rs 阻塞。

## Review Focus

- manifest 是否真正驱动生产注册，而非只在测试中比较。
- catalog 是否只是实现映射，没有形成第二份权限来源。
- bundle 校验是否拒绝重复工具和 registry 双向漂移。
- 删除陈旧源码后是否仍有 V4 生产引用。
- contract manifest、Prompt 和 provenance 是否都读取同一已校验 bundle。

### Task 1: Characterize authority and stale-entry failures

**Files:**
- Modify: `backend/tests/agent_knowledge.rs`
- Modify: `backend/tests/tool_manifest.rs`

1. 添加 manifest 重复 `allowedTools` 必须返回稳定知识错误的测试。
2. 添加工具 catalog 对缺实现、额外实现和重复实现失败关闭的测试。
3. 添加 `build_tools` 的名称顺序与 manifest 顺序一致的测试。
4. 运行聚焦测试并确认因缺少新校验/API 而 RED。

### Task 2: Make validated manifest drive tool registration

**Files:**
- Modify: `backend/src/agent/knowledge.rs`
- Modify: `backend/src/tools/mod.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/main.rs`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/common/app.rs`

1. 在知识加载中校验 allowed tool 名称非空且唯一，提供只读访问器。
2. 把工具宏改为私有实现 catalog 与构造器；实现双向校验错误和按 manifest 的装配。
3. 在生产和测试 AppState 创建前校验 catalog；失败时阻止启动。
4. contract manifest 直接使用已校验 manifest allowed tools。
5. 运行 Task 1 测试确认 GREEN，再运行 Agent factory 与 contract manifest 聚焦测试。

### Task 3: Remove stale conversational capabilities

**Files:**
- Delete: `backend/src/tools/create_script.rs`
- Modify: `backend/src/tools/mod.rs`
- Modify: `backend/src/agent/mod.rs`

1. 删除未注册的 `create_script` 实现和 module 导出。
2. 删除未调用、直接拼原始资源的旧 Agent 构造函数与常量。
3. `rg` 确认当前 V4 生产代码只保留关于禁止该能力的测试/文档陈述。
4. 运行 `cargo check --all-targets --all-features` 与聚焦测试。

### Task 4: Documentation and verification

**Files:**
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/development-status.md`

1. 记录 manifest 权威、实现 catalog、启动失败关闭和已删除陈旧入口。
2. 运行后端格式、Clippy、完整 `e2e-test`；若只有既知外部资源断言失败，精确跳过该断言复跑其余套件。
3. 复核公共合同 diff，确认无需更新功能目录。
4. 提交 8.7 实现和文档，执行自我代码审查后进入 8.8。

