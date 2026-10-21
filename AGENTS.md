# AGENTS.md

配电网仿真智能体项目：Rust 后端 + React 前端。后端用 rig 框架实现 LLM agent，通过工具（tool）封装仿真内核能力；前端用 React + TypeScript + ECharts 绘制曲线。

## 构建 / 测试 / 运行

```bash
# 后端（Rust）
cd backend
cargo build          # 构建
cargo test           # 测试
cargo run            # 运行

# 前端（React + TS + ECharts）
cd frontend
npm install
npm run dev          # 开发服务器 http://localhost:5173，/api 代理到后端
npm run build        # 构建
```

## 项目结构

- `backend/`：Rust 后端，基于 rig 实现 agent 与工具（`inspect_model`、`run_simulation` 等），未来对外提供 HTTP 服务。
- `frontend/`：React + TypeScript 前端，使用 ECharts 绘制曲线，通过 Vite 把 `/api` 代理到后端。
- `backend/src/adapters/`：仿真内核适配器层，所有对内核 API 的调用都收敛在这里，是内核与外界的唯一边界。

## 仿真内核 API 调用形态

- 仿真内核源码与 API 文档当前**尚未纳入本仓库**；调用契约以适配器层为准，随内核接入逐步固化。
- 调用为**异步**：适配器方法返回 `async`，内部通过 `reqwest` 访问内核服务，由 `tokio` 驱动。
- 返回结构化 JSON（`serde_json`）。工具输出约定：`inspect_model` 返回模型检查结果（当前硬编码 `"model ok"`），`run_simulation` 返回 `task_id`（字符串，当前硬编码）。
- 错误码约定：适配器统一返回 `Result<T, AdapterError>`；`AdapterError` 携带内核错误码（字符串）与可读信息，禁止把内核原始错误直接抛给上层。

## 硬约束

- **仿真内核 API 的调用必须通过 `backend/src/adapters/` 适配器层封装，禁止在 agent 工具实现（tool）里直接调用原始内核 API。** 工具实现只能调用适配器暴露的接口；内核的地址、鉴权、错误码翻译都收敛在适配器内。
