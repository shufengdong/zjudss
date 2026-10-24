# V3 部署与发布

## Empty database cutover

V3 不读取前两个未上线版本。部署前停止旧进程，备份后删除旧 SQLite 与 DATA_DIR，从空目录执行唯一 initial
migration。`backend/resources/init_users.csv` 必须为 `ID,NAME`；初始密码等于 ID。种子 SHA-256 变化会让已有库
启动失败，此时必须再次同时重置数据库和 DATA_DIR。

## Required configuration

生产必须配置 `DATABASE_URL`、持久化 `DATA_DIR`、`TENSOREVAL_BIN`、`TENSOREVAL_CWD` 和 DeepSeek secret。
binary 必须支持命名 Parquet input、`--result-dir`、`--output-format parquet`、两种模型 import profile 与脚本
校验；资源目录必须包含受支持 schema 的 catalog、可安全读取的文档与案例。启动从当前目录动态计算资源
snapshot，不固定 eig-rs/sparrowzz commit、binary SHA、资源 hash 或案例数量。生产没有 stub 回退，也不会自动
checkout、pull、reset、clean、构建或改写配置的 eig-rs 工作树。

资源会原子发布到 `DATA_DIR/system/engine-resources/<snapshotId>`，运行进程内保持不变；更新 eig-rs 后必须重启。
限制为 512 篇 Markdown、单篇 4 MiB、文档总计 64 MiB、2,048 个案例、每案例 10,000 文件/2 GiB、依赖深度
64。超过限制返回 `engine_resource_limit_exceeded`；扫描和发布之间发生变化返回
`engine_resource_changed_during_scan`。启动后可直接替换 binary，后续 adapter 调用使用路径上的当前普通文件；
文件不存在或不再是普通文件时返回 `engine_binary_unavailable`。替换后的 binary 不会重新执行启动能力探测。

公网由 HTTPS 反向代理提供 TLS，转发 `/api` 和 SSE，并设置：

```dotenv
SESSION_COOKIE_SECURE=true
```

Cookie 固定 HttpOnly、SameSite=Lax、Path=/。当前相同 ID/密码约定仅用于已批准的第三版试用，不应宣称安全。

## Build and start

```bash
cd backend
cargo run --release
```

```bash
cd frontend
npm ci
npm run build
# 由静态服务托管 dist，并把 /api 代理到后端
```

默认限制：Agent 实例/用户/session 为 8/2/1，等待 32，引擎进程 4；单文件/批次 50/500 MiB；每用户
20,000 artifacts / 20 GiB，80% 告警；数据页最大 500。

## Deterministic gate

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
git diff --check
```

## Candidate real-dependency gate

受保护环境只提供 `DEEPSEEK_API_KEY`。workflow 每次检出 eig-rs 默认分支（含 submodule），构建当前
`tensoreval`，设置 `TENSOREVAL_BIN/TENSOREVAL_CWD` 并运行：

```bash
./scripts/test-release-integration
./scripts/check-functional-coverage --require-release
```

真实旅程从固定用户登录开始，导入直接 DataFrame、确认草稿、显式生成版本、让 Agent 创建脚本、确认没有自动
运行、点击运行两次、执行结果分析和运行比较，再删除一个来源结果并验证运行及物化派生仍可读取。引擎门禁
另外真实执行两种 import profile、命名 input、多个 Snappy Parquet 输出和失败诊断。报告保存测试 ID、Agent
bundle hash、用户 seed hash/数量、engine contract/checks 和 binary hash，不保存用户 ID、密码、Cookie、secret
或物理路径；同时记录动态 `engineResourceSnapshot` 与发现、可导入、不可导入、根 RSS、bundle 数量。binary hash
和 snapshot 是本次证据，不是接收门槛。真实门禁要求至少一个根 RSS、一个 bundle 和一个可导入案例，不断言
精确案例数；个别坏案例以脱敏诊断报告，不阻断其他案例。

## Failure handling

`engine_contract_mismatch` 表示当前 binary/catalog 不满足应用协议；`engine_resource_hash_mismatch` 表示 bundle
自校验或已发布 snapshot 损坏。应修复当前 eig-rs/配置或正式更新应用协议与测试，不得放宽校验或退回
CSV/stub。运行中断会在重启后收敛失败；已发布模型、脚本、结果和分析不回滚。
