# Workflow Failure Diagnosis Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 在研究工作流失败卡片中持久化并展示脱敏引擎日志、结构化大模型分析和默认折叠的自动修正记录。

**Architecture:** 扩展 `ScriptAuthor` 以生成受控的失败分析；worker 在每次引擎失败后先持久化公开日志诊断，再以 best-effort 方式补充 AI 分析。公共 workflow DTO 从现有 `result_diagnostics_json` 解出结构化字段。React 卡片消费该 DTO，提供原格式日志展示、客户端文本下载与折叠的修正历史。

**Tech Stack:** Rust/Axum/async-trait/serde/Diesel, React/TypeScript/Vitest/Testing Library.

**Spec:** `docs/superpowers/specs/2026-11-01-workflow-failure-diagnosis-design.md`

---

### Task 1: 定义并验证失败分析合同

**Files:**
- Modify: `backend/src/agent/script_author.rs`
- Modify: `backend/src/testing/scripted_author.rs`

**Steps:**
1. 先添加单元测试，验证严格 JSON、条目/文本长度限制和非法输出拒绝；运行 `cargo test agent::script_author`，确认失败。
2. 新增 `FailureAnalysisRequest`、`FailureAnalysis`、引用事实/可能原因结构和 `ScriptAuthor::analyze_failure`。
3. 给 `LlmScriptAuthor` 使用独立诊断 prompt，解析并规范化 JSON；给 `ScriptedAuthor` 增加可编排分析响应。
4. 再运行测试，确认通过。

### Task 2: 在工作流 attempt 中持久化公开诊断

**Files:**
- Modify: `backend/src/application/workflow_worker.rs`
- Modify: `backend/src/application/managed_workflows.rs`
- Modify tests near managed workflow integration coverage

**Steps:**
1. 先写工作流测试：引擎失败时公开诊断包含日志元数据和 AI 分析；分析失败时日志仍存在；重复脚本哈希标记无实质修改。运行目标测试并确认失败。
2. worker 在引擎失败后保存基础 diagnostic，然后 best-effort 调用分析接口并更新同一 attempt 的 diagnostic JSON。
3. DTO 暴露结构化 analysis、日志元数据和 `scriptChangedFromParent`；不公开审计 artifact。
4. 运行目标测试，确认通过。

### Task 3: 实现失败诊断卡片和折叠修正历史

**Files:**
- Modify: `frontend/src/types/workflow.ts`
- Modify: `frontend/src/components/messages/WorkflowProgressCard.tsx`
- Modify: `frontend/src/components/messages/WorkflowProgressCard.test.tsx`
- Modify relevant CSS module/global stylesheet if needed

**Steps:**
1. 先添加组件测试，覆盖失败诊断直接显示、`pre` 日志格式、AI 四段信息、分析不可用、下载、修正记录默认折叠和无实质修改提示；运行测试确认失败。
2. 更新 DTO 类型和组件渲染，日志下载只使用已脱敏的 API 文本。
3. 运行组件测试和前端全量测试，确认通过。

### Task 4: 同步设计与功能追溯

**Files:**
- Modify: `docs/api-design.md`
- Modify: `docs/db-design.md`
- Modify: `docs/frontend-design.md`
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/development-status.md`
- Modify: `integration-tests/catalog/features.yaml`

**Steps:**
1. 记录 DTO 字段、JSON 持久化方式、诊断 prompt 边界、前端降级语义和行为覆盖。
2. 运行功能目录校验及相关文档/注册表测试。

### Task 5: 全量验证与提交

**Steps:**
1. 运行 `cargo fmt --all -- --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --features e2e-test`。
2. 运行 `npm test -- --run`、`npm run build` 和 `./scripts/check-functional-coverage --require-pr`。
3. 根据验证结果审查 diff，修复重要问题后重新验证。
4. 提交为一个聚焦提交，不包含用户已有改动。

## Review Focus

- provider 失败或返回畸形 JSON 时不能覆盖或隐藏已有日志。
- 任何返回给浏览器的内容都不能来自未脱敏审计 artifact。
- 旧 workflow 记录和缺少新增字段的 diagnostic 必须兼容。
- 相同哈希的修正尝试不能被误报为有效修改。
