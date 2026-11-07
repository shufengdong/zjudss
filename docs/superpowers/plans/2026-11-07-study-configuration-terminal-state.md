# Study Configuration Terminal State Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 确认研究配置后同步持久化卡片终态，并阻止旧卡进入后续 Agent 上下文。

**Architecture:** `study_configurations` 保持权威状态。确认 API 在 workflow 创建成功后发布同 ID 的终态卡片；聊天历史从消息候选中提取 ID，再通过按用户和会话限定的配置查询确认其仍可编辑。

**Tech Stack:** Rust、Axum、Diesel/SQLite、rig、现有 scripted agent 与 managed workflow 测试设施

**Spec:** `docs/superpowers/specs/2026-11-07-study-configuration-terminal-state-design.md`

## Global Constraints

- 直接在 `master` 工作，不创建 worktree。
- repository 查询显式携带 `UserId`，跨用户与不存在统一 404。
- 确认仍只能由固定 UI 按钮触发；Agent 不获得确认权限。
- 公共 DTO 和路由不变。

## Review Focus

- 确认成功但终态事件发布失败：workflow 不得被重复创建或回滚。
- 同一会话存在多个配置：只注入权威状态仍可编辑的最新配置。
- 消息 payload 为待确认但数据库已 confirmed/invalidated/cancelled：不得注入。
- 其他用户的配置 ID：不得通过历史校验泄漏状态。
- 卡片更新：必须复用 configuration ID 对应的同一消息。

---

### Task 1: 用权威状态筛选 Agent 配置上下文

**Files:**
- Modify: `backend/src/application/chat.rs`
- Modify: `backend/src/db/repositories/study_configuration.rs`
- Test: `backend/tests/scripted_agent.rs`

**Interfaces:**
- Consumes: 当前用户、session ID、持久化 `study-config` 消息候选。
- Produces: repository 查询当前会话最新可编辑配置 ID；`load_history` 只注入匹配消息。

- [ ] **Step 1: 写确认配置后历史不含旧 `currentStudyConfiguration` 的失败测试**
- [ ] **Step 2: 运行目标测试，确认因旧配置仍被注入而失败**
- [ ] **Step 3: 增加按 `UserId + session_id + mutable statuses` 查询并接入 `load_history`**
- [ ] **Step 4: 运行目标测试，确认通过**

### Task 2: 确认后发布同一配置卡的终态

**Files:**
- Modify: `backend/src/api/study_configurations.rs`
- Test: `backend/tests/managed_workflows.rs`

**Interfaces:**
- Consumes: 确认成功返回的 workflow 与配置 ID。
- Produces: 同 configuration ID 的 `study-config` upsert，state/actionCard 均为 `confirmed`。

- [ ] **Step 1: 写确认接口将原卡更新为 `confirmed` 的失败测试**
- [ ] **Step 2: 运行目标测试，确认卡片仍为 `ready_to_confirm`**
- [ ] **Step 3: 确认成功后读取权威配置并调用现有 `publish`**
- [ ] **Step 4: 运行目标测试，确认消息原位更新且 workflow 仍成功**

### Task 3: 文档和完整验证

**Files:**
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/api-design.md`
- Modify: `docs/development-status.md`

**Interfaces:**
- Consumes: Tasks 1–2 的最终行为。
- Produces: 当前设计与验证记录。

- [ ] **Step 1: 同步说明权威状态过滤和确认终态事件**
- [ ] **Step 2: 运行 `cargo fmt --all -- --check`**
- [ ] **Step 3: 运行目标测试与 `cargo test --features e2e-test`**
- [ ] **Step 4: 运行 `cargo clippy --all-targets --all-features -- -D warnings`**
- [ ] **Step 5: 运行前端测试和构建，确认公共消息合同未回归**
- [ ] **Step 6: 运行 `./scripts/check-functional-coverage --require-pr` 并记录结果**

