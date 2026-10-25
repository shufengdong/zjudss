# AGENTS.md

配电网仿真智能体 V3：Rust/Axum/rig 后端 + React/TypeScript/ECharts 前端。Agent 通过受控工具
管理模型工作区、调用仿真、分析 Parquet 结果并恢复历史；领域写操作以自然语言消息为主。

## 开始工作前

1. 先读 `docs/development-status.md`，确认当前实现、硬边界和验证状态。
2. 接口改动读 `docs/api-design.md` 与 `frontend/src/types/`；数据库改动读 `docs/db-design.md`；前端行为
   读 `docs/frontend-design.md`。
3. 引擎资源、Prompt 或工具改动读 `docs/agent-prompt-skills-design.md`；发布和集成测试改动读
   `docs/v3-deployment.md` 与 `docs/integration-testing-design.md`。
4. 除非用户另行要求，直接在 `master` 修改，不自行创建 feature worktree。提交时保留并排除用户已有的
   未提交或已暂存文件。

## 构建、测试与运行

```bash
# 后端
cd backend
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --features e2e-test
cargo run --release

# 前端（接上面的 backend 目录）
cd ../frontend
npm ci
npm test -- --run
npm run build
npm run dev             # http://localhost:5173，/api 代理到后端
npm run test:e2e

# 仓库根目录
cd ..
./scripts/check-functional-coverage --require-pr
```

生产启动必须配置 `TENSOREVAL_BIN`、`SPARROWZZ_DIR` 和 LLM secret；不会自动回退到测试 stub。具体配置
见 `docs/v3-deployment.md`。候选发布的真实依赖门禁也见该文档。

## 项目结构与权威来源

- `backend/src/api/`：REST/SSE 边界。
- `backend/src/application/`：业务编排、调度、工作区、仿真和分析。
- `backend/src/agent/`：DeepSeek agent、Prompt 和按需知识加载。
- `backend/src/tools/`：Agent 工具，只接受稳定资源 ID 和受控参数。
- `backend/src/adapters/`：所有 `tensoreval` 进程调用的唯一边界。
- `backend/src/engine_resources/`：启动 preflight、引擎文档/catalog/案例快照。
- `backend/migrations/00000000000000_initial/`：V3 空库初始化的权威 schema。
- `backend/resources/agent/v3/manifest.json`：Agent bundle、engine contract、资源 hash 和允许工具的权威清单。
- `frontend/src/types/`：前端使用的公共 DTO 类型；HTTP 语义以 `docs/api-design.md` 为准。
- `integration-tests/catalog/features.yaml`：机器可读功能目录和测试追溯入口。

`docs/superpowers/plans/` 是历史实施记录。不要用旧计划中的步骤或中间状态覆盖当前代码与当前设计文档。

## 硬约束

1. **所有 tensoreval 调用必须通过 `backend/src/adapters/`。** tool、handler 和 application service 不得
   直接拼命令或启动子进程。
2. 生产 `ProcessEngine` 显式使用 `--result-dir ... --output-format parquet`；仿真和派生分析结果以
   Snappy Parquet 为事实来源，不恢复 CSV 结果链路或固定 metrics。
3. LLM 和前端只传稳定资源 ID 与受控 spec，不得接收服务器路径、命令、SQL 或任意 ECharts option。
4. artifact、模型版本、任务脚本、结果数据集和分析记录发布后不可修改；更新通过新资源和血缘表达。
5. 每个 repository 查询显式接收 `UserId`；跨用户与不存在资源统一返回 404。
6. `SPARROWZZ_DIR` 必须直接指向当前可读的 `eig-rs/sparrowzz` 资源树，并满足代码定义的 catalog schema、路径和容量约束；
   服务只读取、验证并发布本次启动的不可变 snapshot，不得 checkout、pull、reset、clean 或改写该工作树。
7. Agent Prompt 不常驻整套引擎文档。服务启动时功能性预检当前 binary，并索引和发布当前资源 snapshot；
   更新 eig-rs 后必须重启，agent 按需调用文档和 skill 工具；
   生成脚本的 run 必须保存实际 bundle、skill refs 和 engine contract。
8. 固定按钮只用于上传、会话、案例浏览、历史分页、结果读取和下载等高频无歧义动作。模型修改、激活、
   仿真、分析和比较通过可见的自然语言消息进入 Agent 工具链。
9. 新增或改变公共路由、消息类型、工具、skill、上传格式、视图或稳定错误码时，同步更新功能目录、
   自动化行为测试和对应当前设计文档；注册表枚举测试不能替代行为覆盖。
