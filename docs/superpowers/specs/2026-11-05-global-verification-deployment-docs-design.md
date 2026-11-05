# 全局验证、部署与文档收敛设计

**日期：** 2026-11-05  
**范围：** 仓库维护路线 8.9  
**状态：** 已批准（用户授权连续完成 8.7–8.9）

## 1. 目标

让候选发布验证直接消费运行时权威清单，消除 Agent bundle 版本和工具集合的手工镜像；随后执行仓库完整门禁，区分代码回归、受限环境问题、真实依赖缺失和已知外部 eig-rs 资源不兼容。最后把部署与状态文档收敛为当前合同和有日期的证据，不把历史数量写成永久接收条件。

## 2. 审计结论

- `backend/resources/agent/v4/manifest.json` 已在 8.7 成为运行时工具权限权威来源，但 release semantic assertions 仍维护另一份 23 项 allowlist。
- `scripts/test-release-integration` 仍把报告 bundle 版本写死为已经过期的 `v4.2.0`，与当前 manifest `v4.4.0` 不一致；真实发布旅程即使本身成功也会在最终报告校验中失败。
- release config 测试再次写死 bundle 版本和工具数量，只能证明多个副本同时更新，不能证明脚本读取权威 manifest。
- 发布文档把某次 144/144 真实投影证据写成“当前 snapshot”事实，而相邻开发资源树当前已知有 7 个 `case_projection_invalid`。接收规则本身已经是动态发现，不应把历史样本数当作永久门槛。
- `development-status.md` 同时保存当前状态、各维护阶段和多轮重复验证数量；本阶段保留当前状态与最近证据，但不再新增未经运行的全绿声明。

## 3. 方案

### 3.1 Manifest 驱动 release 工具策略

`expectToolTrace` 不再导入或维护全局工具名集合，而是要求调用方显式传入本次 agent bundle manifest 的 `allowedTools`。真实 release journey 和手工全案例 journey 在计算 bundle hash 时一并读取、校验并传递该列表；段落级 `allowedTools` 仍只表达该步骤预期允许成功的子集。

语义断言继续独立维护“有副作用工具”分类，因为它控制单次成功次数，不是权限注册表。稳定 ID、skill 先加载、失败恢复和段落子集检查保持不变。

### 3.2 Manifest 驱动 bundle 身份

发布脚本把 manifest 路径传给最终报告校验，报告版本必须等于该文件的 `agentBundleVersion`。脚本不接受命令行或环境中另一个期望版本，也不复制当前版本字符串。bundle hash 仍由 journey 对整个 bundle 计算并写入报告。

release config 测试验证 manifest 列表非空、唯一并包含安全关键工具，同时验证脚本读取 manifest 且没有硬编码报告版本；`.env.example` 默认版本与 manifest 做相等比较，不再分别断言数字。

### 3.3 动态发布证据

部署文档把案例数、profile 数和门面覆盖表标为带日期的历史真实证据；正式门禁只接受本次报告动态发现的完整性、一致性、零失败和代表运行覆盖。当前相邻资源树的 7 个无效案例继续作为外部阻塞记录，不放宽 loader、projector 或发布断言。

真实 LLM、tensoreval 或 eig-rs 配置缺失时，普通仓库门禁仍完整运行；真实依赖 lane 明确记为未执行，不能记为通过或用 stub 替代。

## 4. 文档所有权

- `docs/v4-deployment.md`：生产配置、发布命令、动态接收规则和最近真实证据。
- `docs/integration-testing-design.md`：各测试 lane、证据边界和功能追溯规则。
- `docs/development-status.md`：当前实现、硬边界、最新验证结果和外部阻塞；历史实施步骤继续留在 `docs/superpowers/plans/`。
- `backend/resources/agent/v4/manifest.json`：bundle 版本与允许工具的唯一机器权威来源。

## 5. 验收

- 测试证明 semantic assertions 接受 manifest 提供的新工具并拒绝 manifest 之外工具，不依赖源码内固定 allowlist。
- 发布脚本报告版本与当前 manifest 比较，源码不出现过期或当前版本字面量。
- Rust fmt、全 target/all feature Clippy、完整 e2e-test（已知外部断言单独记录）、全量 Vitest、生产构建、Playwright 和功能覆盖按当前仓库命令执行。
- 公共路由、DTO、数据库、Agent 工具集合、Prompt 内容、ChartModel/ECharts、Parquet 和 adapter 边界均不改变。

