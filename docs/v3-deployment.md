# V3 部署与发布验收

## 1. 数据重置与固定用户

V3 不兼容前两个未上线版本的数据库或 DATA_DIR。首次启动前停止旧进程，备份后删除旧 SQLite 文件和
旧 DATA_DIR，再从空目录启动；仓库只有一份 `00000000000000_initial` migration，不执行回填或双读。

后端将 `backend/resources/init_users.csv` 作为编译输入。空库启动时完整校验 `ID,NAME` 表头与每行
内容，再在一个事务中创建固定用户和各自 storage scope。初始密码与 ID 相同，数据库只保存带独立随机
salt 的 Argon2id 散列。这个密码约定便于未上线阶段试用，但任何知道 ID 的人都能登录，**不适合作为
安全身份方案**；当前第三版已明确接受这项风险，正式安全化前不得据此宣称账号安全。

数据库在 `system_metadata.init_users_sha256` 保存种子字节 SHA-256。相同字节可幂等重启；CSV 改动而
数据库已有用户时，启动以 `user_seed_mismatch_reset_required` 失败。此时必须停止服务并同时重置数据库
和 DATA_DIR，不能在旧业务数据上替换用户集合。候选发布证据记录该 SHA-256 和用户数量，但绝不列出 ID。

## 2. 必需依赖

- SQLite 必须包含 FTS5 trigram。
- `TENSOREVAL_CWD` 指向只读引擎资源根。候选发布必须使用与内置 manifest 内容 hash 完全匹配的 artifact。
- `TENSOREVAL_BIN` 对应同一 engine contract，支持 Parquet input、`--result-dir` 和
  `--output-format parquet`。
- DATA_DIR、SQLite 文件及其父目录位于持久卷。
- LLM provider 密钥通过 secret 注入，不写入镜像或仓库。

启动 preflight 会校验 capability、docs/catalog/examples snapshot、agent bundle hash 和兼容关系；任一
失败时不接流量。

## 3. 登录与公网配置

`POST /api/auth/login` 使用固定用户 ID 和密码，成功后设置服务端可撤销的不透明
`zjudss_session` Cookie。默认有效期 30 天，可用 `SESSION_TTL_DAYS` 调整；一个用户可在多个浏览器
分别登录，退出只撤销当前浏览器的会话。

公网必须由 HTTPS 反向代理提供 TLS，并设置：

```dotenv
SESSION_COOKIE_SECURE=true
```

否则浏览器可能经明文连接发送登录 Cookie。Cookie 已固定为 HttpOnly、SameSite=Lax、Path=/；代理不得
记录 Cookie 或登录请求体。V3 不再接受 trusted-proxy 身份头，也没有 instance 模式。

## 4. 默认容量

| 项目 | 默认值 |
|---|---:|
| 实例活动 agent run | 8 |
| 每用户活动 run | 2 |
| 等待队列 | 32 |
| 同 session 活动 run | 1 |
| tensoreval 子进程 | 4 |
| 单文件/单批上传 | 50 MiB / 500 MiB |
| artifact 数/逻辑容量 | 20,000 / 20 GiB（每用户） |
| 告警 | 80% |
| 表格页 | 默认 100，最大 500 |
| 图表点/series/category/heatmap cell | 50,000 / 100 / 5,000 / 100,000 |

## 5. 启动与维护

```bash
cd backend
cp .env.example .env
# 填写真实路径和 secret；公网额外设置 SESSION_COOKIE_SECURE=true
cargo run --release
```

```bash
cd frontend
npm ci
npm run build
```

反向代理把 `/api` 和 SSE 转发到后端；SSE 禁用响应缓冲并设置足够长的读取超时。聊天索引可离线执行
`cargo run --release -- rebuild-search-index` 重建；该命令不启动 LLM 或 tensoreval。

## 6. 发布门禁

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

除完整自动门禁外，候选发布还检查 `GET /health`、登录、`GET /api/me`、退出、SSE 撤销、跨用户 404、
上传、一次真实 Parquet 仿真以及结果分页/下载。发布报告必须保存测试 ID、种子 SHA-256、agent bundle、
engine contract 和 binary SHA-256，不保存固定用户 ID、密码、Cookie 或完整请求体。

候选发布使用受保护的 `candidate-release` 环境，必须提供 `DEEPSEEK_API_KEY`，并下载经
`TENSOREVAL_SHA256` 精确校验的 `tensoreval-release` artifact。artifact 顶层布局为：

```text
tensoreval
cwd/
└── sparrowzz/dss/
```

`cwd/` 必须包含固定案例显式引用的全部跨目录依赖。后端启动 preflight 会再次校验 catalog、文档、
案例及依赖；缺少 binary、资源目录、secret、合同 hash 或任一语义断言时发布失败，不能静默跳过。

若报告 `engine_resource_hash_mismatch`、`example_source_changed` 或 `engine_contract_mismatch`，应恢复
与当前 bundle 匹配的完整资源 artifact；计划内升级必须同步更新 manifest、bundle 和测试。不得放宽
校验、替换单个文件或回退到其他 catalog。
