# 全局验证、部署与文档收敛实施计划

> **For Codex:** 使用 `superpowers:executing-plans` 连续实施；行为变化先观察失败测试。

**Goal:** 消除 release lane 的 Agent 权限/版本镜像，执行全仓门禁并把当前部署和验证证据写回权威文档。

**Architecture:** manifest 提供 bundle 身份与运行时工具全集；release journey 只传递该事实，semantic assertions 验证轨迹和步骤子集；shell gate 对报告和 manifest 做运行时相等比较。文档区分动态接收规则与有日期的历史证据。

**Tech Stack:** Rust/Axum、React/TypeScript/Vitest、Playwright、POSIX shell、Node.js。

## Task 1: Characterize stale release mirrors

**Files:**
- Modify: `frontend/src/release/semantic-assertions.test.ts`
- Modify: `frontend/tests/release/release-config.test.ts`

1. 写测试证明 runtime policy 参数允许非源码内置工具、缺失工具被拒绝。
2. 写测试要求 release shell 从 manifest 比较报告版本且不存在版本字面量。
3. 运行聚焦测试并观察现状 RED。

## Task 2: Make release semantics manifest-driven

**Files:**
- Modify: `integration-tests/release/semantic-assertions.ts`
- Modify: `integration-tests/release/real-dependencies.spec.ts`
- Modify: `integration-tests/manual-case-journeys/all-cases.spec.ts`
- Modify: `integration-tests/manual-case-journeys/journey.ts`
- Modify: `scripts/test-release-integration`
- Modify: relevant release tests

1. 从 bundle evidence 返回经过非空、唯一、字符串校验的 `allowedTools`。
2. 所有 `expectToolTrace` 调用显式传入本次 manifest policy。
3. shell 最终校验读取同一 manifest 的版本。
4. 运行 semantic/release/manual-case 聚焦测试并确认 GREEN。

## Task 3: Converge deployment and status documentation

**Files:**
- Modify: `docs/v4-deployment.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/development-status.md`

1. 把固定案例/profile 数量改为有日期的证据，不作为接收门槛。
2. 记录 manifest 对 release policy/version 的唯一所有权和真实依赖缺失时的证据边界。
3. 仅从实际命令输出更新最终验证数量与已知外部阻塞。

## Task 4: Run complete repository gates

1. 后端运行 fmt、Clippy、未过滤完整 `e2e-test`；若仍只有同一个相邻 eig-rs 外部断言失败，保存证据并精确跳过该测试复跑其余完整套件。
2. 前端运行全量 Vitest、生产构建和 Playwright；需要回环端口时使用已授权的本机执行。
3. 根目录运行 `./scripts/check-functional-coverage --require-pr` 和 `git diff --check`。
4. 检查真实发布环境变量；缺失时记录未执行，不伪造成功。
5. 自审公共合同 diff，提交实现与文档，然后使用完成前验证流程给出最终状态。

