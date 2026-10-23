# V3 部署与发布

## Empty database cutover

V3 不读取前两个未上线版本。部署前停止旧进程，备份后删除旧 SQLite 与 DATA_DIR，从空目录执行唯一 initial
migration。`backend/resources/init_users.csv` 必须为 `ID,NAME`；初始密码等于 ID。种子 SHA-256 变化会让已有库
启动失败，此时必须再次同时重置数据库和 DATA_DIR。

## Required configuration

生产必须配置 `DATABASE_URL`、持久化 `DATA_DIR`、`TENSOREVAL_BIN`、`TENSOREVAL_CWD` 和 DeepSeek secret。
binary 必须支持命名 Parquet input、`--result-dir`、`--output-format parquet`、两种模型 import profile 与脚本
校验；资源目录必须匹配内置 docs/catalog/examples contract。生产没有 stub 回退。

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

受保护环境提供 `DEEPSEEK_API_KEY`、`TENSOREVAL_SHA256` 和固定 engine artifact。artifact 顶层为
`tensoreval` 与 `cwd/`。workflow 校验 SHA-256 后设置 `TENSOREVAL_BIN/TENSOREVAL_CWD` 并运行：

```bash
./scripts/test-release-integration
./scripts/check-functional-coverage --require-release
```

真实旅程从固定用户登录开始，导入直接 DataFrame、确认草稿、显式生成版本、让 Agent 创建脚本、确认没有自动
运行、点击运行两次、执行结果分析和运行比较，再删除一个来源结果并验证运行及物化派生仍可读取。引擎门禁
另外真实执行两种 import profile、命名 input、多个 Snappy Parquet 输出和失败诊断。报告保存测试 ID、Agent
bundle hash、用户 seed hash/数量、engine contract/checks 和 binary hash，不保存用户 ID、密码、Cookie、secret
或物理路径。

## Failure handling

`engine_resource_hash_mismatch`、`engine_contract_mismatch` 或 snapshot mismatch 必须通过恢复匹配 artifact，或正式
更新 contract/bundle/tests 解决。禁止放宽 hash、从相邻工作树取发布事实或退回 CSV/stub。运行中断会在重启后
收敛失败；已发布模型、脚本、结果和分析不回滚。
