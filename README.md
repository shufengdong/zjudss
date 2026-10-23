# ZJU DSS

配电网仿真智能体 V3。用户可以上传单个 `.rss`、ZIP/目录形式的 `.rssbundle`，也可以分批上传
CSV、Excel 或 Parquet DataFrame；随后通过自然语言完成模型确认、参数修改、潮流/短路等仿真、结果
查询和修改前后对比。聊天、模型版本、仿真任务、结果数据集和分析血缘都持久化，可在刷新后恢复。

后端使用 Rust、Axum、rig、Diesel/SQLite，通过受控工具调用 `tensoreval`；前端使用 React、
TypeScript、Vite 和 ECharts。仿真及派生分析结果统一保存为 Snappy Parquet，数据库只保存元数据、
血缘和 artifact 引用。

## 已实现能力

- 单 RSS、rssbundle、CSV、Excel、Parquet 和分批追加上传；DataFrame 类型按列名推断。
- active/draft 模型工作区、不可变版本、错误文件重传、替换、移除、激活和回退。
- 内置案例搜索与导入；案例导入后可继续用自然语言修改参数并重跑。
- 22 个 Agent 工具，以及按需加载的领域技能和引擎文档检索。
- 原生 Parquet 仿真结果、自然语言结果分析、分页表格、六类受控 ECharts 视图和任务比较。
- 固定用户登录、侧栏用户菜单、会话历史、全文搜索、可恢复 SSE、身份隔离、存储配额和并发调度。
- 功能目录驱动的 Rust、Vitest、Playwright 和真实 DeepSeek/tensoreval 发布验收。

## 目录结构

```text
zjudss/
├── backend/
│   ├── migrations/        # 单一 V3 initial schema
│   ├── resources/agent/   # 内置 prompt、skill 索引和资源合同
│   └── src/
│       ├── api/           # REST/SSE handler 与 DTO
│       ├── application/   # 会话、调度、工作区、分析等业务编排
│       ├── agent/         # DeepSeek agent 与按需知识加载
│       ├── tools/         # 22 个受控 Agent 工具
│       ├── adapters/      # tensoreval 唯一调用边界
│       └── db/            # SQLite repository
├── frontend/              # React 工作台、历史、上传、表格和图表
├── integration-tests/     # 功能目录、Playwright 和发布语义断言
├── scripts/               # 覆盖率与真实依赖发布门禁
├── docs/                  # 当前设计、部署、测试和状态文档
└── AGENTS.md              # 仓库开发约束
```

## 启动

后端生产路径不会回退到 stub。请先准备与固定资源合同匹配的 `tensoreval` 可执行文件和只读引擎资源
目录，再填写配置：

```bash
cd backend
cp .env.example .env
# 必填：DEEPSEEK_API_KEY、TENSOREVAL_BIN、TENSOREVAL_CWD
cargo run --release
```

`TENSOREVAL_CWD` 是发布 artifact 中经过校验的资源根目录，其中必须包含合同固定的文档、catalog、
案例及其依赖。开发时可以指向 `eig-rs` 的受控工作树；合同范围内的内容 hash 必须与内置 manifest
完全一致，否则启动 preflight 失败。当前相邻仓库不能自动视为发布资源。

启动前端：

```bash
cd frontend
npm ci
npm run dev       # http://localhost:5173，/api 代理到 127.0.0.1:8080
```

固定用户初始密码与 ID 相同，只适用于当前明确接受风险的第三版。公网 HTTPS 必须设置
`SESSION_COOKIE_SECURE=true`。完整变量、空库重置、artifact 布局和发布门禁见
[V3 部署与发布验收](docs/v3-deployment.md)。

## 验证

```bash
cd backend
cargo fmt --all -- --check
cargo test --features e2e-test

cd ../frontend
npm test -- --run
npm run build
npm run test:e2e

cd ..
./scripts/check-functional-coverage --require-pr
git diff --check
```

普通 PR 使用 Scripted LLM 和 ScenarioEngine 走生产前后端代码路径；候选发布另以真实 DeepSeek 和运行时构建的
eig-rs 当前默认分支验证外部依赖。测试分层、覆盖率定义和当前数量见
[自动化集成测试设计](docs/integration-testing-design.md)。

## 文档入口

- [开发状态与关键决策](docs/development-status.md)：当前实现、硬边界和已验证结果。
- [系统架构](docs/architecture.md)：分层、身份、调度、存储和引擎资源机制。
- [HTTP/SSE 契约](docs/api-design.md)：前后端公共接口的权威说明。
- [数据库设计](docs/db-design.md)：V3 schema、固定用户、不可变记录和血缘。
- [前端设计](docs/frontend-design.md)：自然语言交互、状态恢复和可视化。
- [模型工作区设计](docs/model-workspace-design.md)：摄取、版本、分析、比较和历史。
- [Agent Prompt 与领域技能](docs/agent-prompt-skills-design.md)：知识快照、技能门控和资源版本。
- [部署与发布验收](docs/v3-deployment.md)：必需配置、固定用户 seed、动态引擎资源和发布门禁。
- [自动化集成测试设计](docs/integration-testing-design.md)：功能目录和各测试层职责。

`docs/superpowers/plans/` 保存已经执行的历史实施计划，不作为当前运行契约；发生差异时以上述当前文档
和代码为准。
