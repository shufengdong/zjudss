# ZJU DSS

前后端分离项目：Rust 后端 + React（TypeScript）前端，使用 ECharts 绘制曲线。

## 目录结构

```
zjudss/
├── backend/    # Rust 后端（axum）
│   ├── Cargo.toml
│   └── src/main.rs
├── frontend/   # React + TypeScript 前端（Vite + ECharts）
│   ├── package.json
│   ├── vite.config.ts
│   ├── index.html
│   └── src/
│       ├── main.tsx
│       ├── App.tsx
│       └── components/CurveChart.tsx
└── README.md
```

## 启动

### 后端

```bash
cd backend
cargo run          # 监听 http://127.0.0.1:8080
```

### 前端

```bash
cd frontend
npm install
npm run dev        # 监听 http://localhost:5173，/api 已代理到后端 8080
```
