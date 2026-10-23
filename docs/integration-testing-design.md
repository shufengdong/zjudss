# V3 集成测试与功能追溯

## Test lanes

- Rust：数据库约束、repository scope、adapter 命令/输出、application 事务、Axum 状态码和恢复。
- Vitest：公共 DTO、reducer/hook、workspace、脚本卡、运行记录、数据表与图表。
- Playwright：浏览器 → Vite proxy → Axum → SQLite/CAS 的确定性全链路，使用 Scripted LLM 与
  ScenarioEngine，但不绕过 HTTP 或 UI。
- Release：固定 SHA-256 的真实 tensoreval artifact + DeepSeek secret；无 secret/资源时失败，不跳过。

## Coverage matrix

功能目录 `integration-tests/catalog/features.yaml` 是机器可读追溯源。每项包含 source、layers、lanes、真实
测试符号、断言、fixture 和可选公共合同。每个 manifest route/message/tool/skill/upload/view/error 必须恰好出现
一次；注册表枚举测试不能单独证明行为。PR 路由合同还必须由关联 Playwright 用例产生 2xx/3xx 请求证据。

覆盖范围包括 BOOT、AUTH、SESSION、MESSAGE、SSE、SCHED、UPLOAD、MODEL-IMPORT、WORKSPACE、VERSION、
EXAMPLE、AGENT、SCRIPT、RUN、DATASET、ANALYSIS、VIEW、COMPARE、HISTORY、STORAGE、UI、RECOVERY、ENGINE。

## Deterministic journeys

浏览器套件验证固定用户登录/退出/重置；会话 CRUD、分页和 SSE；DSS/bundle/ZIP/表格导入；案例确认；draft
发布、active race、历史激活和版本删除；脚本预览/全文/复制与 Agent 只创建；按钮运行、多结果与单结果删除；
分析、视图、比较在来源删除后继续读取；两用户隔离；重启收敛与 FTS 局部失败；配额、恶意归档和响应式无障碍。

ScenarioEngine 场景由控制端按调用顺序排队，不使用 sleep；timeout 使用 barrier。测试证据写入
`target/functional-coverage/e2e-routes.ndjson`，包含通过的测试名及实际 method/path/status。

## Commands

```bash
cd backend
cargo fmt --all -- --check
cargo test --features e2e-test
cd ../frontend
npm ci
npm test -- --run
npm run build
npm run test:e2e
cd ..
./scripts/check-functional-coverage --require-pr
```

候选发布执行 `scripts/test-release-integration`。报告记录测试 ID、Agent bundle hash、用户 seed hash/数量、engine
contract 与 binary hash；日志必须脱敏。真实门禁必须覆盖模型导入/发布、Agent 创建脚本、用户显式运行、结果
分析/比较和只使用稳定 ID。

## Failure policy

不允许 `skip`、`ignore` 或 manual-only 填补覆盖。失败先保留 trace/screenshot/log，按最窄层复现，再运行全量。
相邻引擎工作树 hash 漂移不能通过放宽校验解决；应恢复固定 artifact 或正式更新 contract 和全部证据。
