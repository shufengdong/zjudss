# ZJU DSS

配电网仿真智能体：前后端分离项目。后端 Rust + axum + rig，实现 LLM agent 并通过工具封装仿真内核能力；前端 React + TypeScript + Vite + Tailwind + ECharts，绘制仿真曲线。

## 目录结构

```
zjudss/
├── backend/               # Rust 后端（axum + rig + Diesel/SQLite）
│   ├── Cargo.toml
│   ├── migrations/        # Diesel 迁移（初始 6 表）
│   └── src/
│       ├── main.rs        # 入口：装配 AppState、迁移、路由
│       ├── api/           # HTTP 层（axum handler / DTO / SSE）
│       ├── application/   # 编排层（AppState / EventBus / ChatService）
│       ├── agent/         # provider 选择 + agent 组装
│       ├── tools/         # 有状态工具（inspect_model / run_simulation）
│       ├── adapters/      # 仿真内核适配器层（内核唯一边界）
│       └── db/            # 连接池 / schema / models / repository
├── frontend/              # React + TypeScript 前端（Vite + Tailwind + ECharts）
│   ├── package.json
│   ├── vite.config.ts
│   ├── index.html
│   └── src/
│       ├── main.tsx
│       ├── App.tsx        # 状态与接线（会话 / 消息 / SSE 流式）
│       ├── types/         # ★ 前后端契约的权威类型定义
│       ├── api/           # 后端 REST + SSE 接入层
│       ├── components/    # 布局 + 6 类消息卡片 + 图表
│       └── mocks/         # 曲线 mock（会话 / 消息 mock 已弃用）
├── docs/                  # 设计文档 + 开发状态速览（新开的 agent 先读）
│   ├── development-status.md   # 开发状态与待办速览
│   ├── architecture.md         # 后端架构
│   ├── api-design.md           # 前后端契约
│   ├── db-design.md            # 数据库设计
│   └── frontend-design.md      # 前端功能设计（后端 agent 对接必读）
├── AGENTS.md              # 项目指令（自动加载）
└── README.md
```

## 启动

### 后端

```bash
cd backend
cargo run          # 监听 http://127.0.0.1:8080
```

可选环境变量（缺省即可跑通）：`DATABASE_URL`（默认 `sqlite://backend.db`）、`LLM_PROVIDER` /
`LLM_MODEL`（默认 DeepSeek）、`TENSOREVAL_BIN`（真实引擎可执行文件路径，未设置时用 stub）、
`TENSOREVAL_TIMEOUT_MS`（单次引擎调用超时）、`DATA_DIR`（数据/结果目录，默认 `data`）。

### 前端

```bash
cd frontend
npm install
npm run dev        # http://localhost:5173，/api 代理到后端 8080
npm run build      # 构建 + TS 类型校验
```

## 文档

- **开发状态与待办速览**：`docs/development-status.md`
- 后端架构：`docs/architecture.md`
- 前后端契约：`docs/api-design.md`
- 数据库设计：`docs/db-design.md`
- 前端功能设计（后端 agent 对接必读）：`docs/frontend-design.md`
