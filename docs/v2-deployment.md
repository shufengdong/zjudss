# v2 部署与发布验收

## 1. 数据重置

v2 不兼容未投入使用的原型数据库或 DATA_DIR。首次启动前停止旧进程，备份后删除旧 SQLite 文件和
旧 DATA_DIR，再从空目录启动。仓库只有一份 `00000000000000_initial` migration，不执行数据回填或双读。

## 2. 必需依赖

- SQLite 必须包含 FTS5 trigram。
- `TENSOREVAL_CWD` 指向只读引擎资源根目录。开发时可使用受控的 eig-rs 工作树；候选发布必须使用
  与内置 manifest 内容 hash 完全匹配的资源 artifact。
- `TENSOREVAL_BIN` 对应同一 engine contract，支持 Parquet input、
  `--result-dir` 和 `--output-format parquet`。
- DATA_DIR、SQLite 文件及其父目录位于持久卷。
- LLM provider 的密钥通过 secret 管理注入，不写入镜像或仓库。

启动 preflight 会校验 capability、docs/catalog/examples snapshot、agent bundle hash 和兼容关系。
任一失败时进程不接流量。

## 3. 认证

可信私有单用户部署可使用 `AUTH_MODE=instance` 和 `PUBLIC_DEPLOYMENT=false`。公开多人部署必须使用
`AUTH_MODE=trusted_proxy`、`PUBLIC_DEPLOYMENT=true` 并配置高熵 `AUTH_PROXY_TOKEN`。

代理必须先移除客户端传入的 `X-Zjudss-Subject/X-Zjudss-Tenant/X-Zjudss-Proxy-Token`，认证后重新
注入。后端常量时间比较 proxy token。不得让浏览器自行生成 subject 作为授权边界。

## 4. 默认容量

| 项目 | 默认值 |
|---|---:|
| 实例活动 agent run | 8 |
| 每 owner 活动 run | 2 |
| 等待队列 | 32 |
| 同 session 活动 run | 1 |
| tensoreval 子进程 | 4 |
| 单文件/单批上传 | 50 MiB / 500 MiB |
| artifact 数/逻辑容量 | 20,000 / 20 GiB（每 storage scope） |
| 告警 | 80% |
| 表格页 | 默认 100，最大 500 |
| 图表点/series/category/heatmap cell | 50,000 / 100 / 5,000 / 100,000 |

2026-10-22 在 Apple Silicon 开发机的 debug 测试中，并发门禁 3 项用例自身耗时 0.47 秒；
`/usr/bin/time -l cargo test --test agent_scheduler` 总耗时 1.15 秒，峰值 RSS 224,772,096 字节。
该数字包含 Cargo 测试进程，只用于回归基线。真实 tensoreval 的内存取决于网络规模，因此生产上限
保持 4，需用目标案例压测后才可提高。

## 5. 启动与维护

```bash
cd backend
cp .env.example .env
# 填写真实路径与 secret
cargo run --release
```

```bash
cd frontend
npm ci
npm run build
```

反向代理把 `/api` 和 SSE 转发到后端；SSE 禁用响应缓冲并设置足够长的读取超时。聊天索引可离线
执行 `cargo run --release -- rebuild-search-index` 重建；该命令不启动 LLM 或 tensoreval。

## 6. 发布门禁

当前自动化验收覆盖四类领域输入路径，以及 12 个文件、18 个用例组成的浏览器 L1/L2 全链路：

| 路径 | 验收用例 |
|---|---|
| 单个 `.rss` 与 `rssbundle` | `bundle_ingestion`：安全解包、清单识别、错误文件替换 |
| 多个 CSV/Excel/Parquet DataFrame | `dataframe_ingestion`、`e2e_v2`：按表头推断、分批补传、模型版本冻结 |
| 内置案例导入后修改 | `example_import`：目录快照、可用性校验、复制到用户工作区 |
| 仿真、查询、可视化与对比 | `e2e_v2`：两次仿真、通用 DataFrame 分析、语义视图、任务对比和刷新恢复 |

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

另外检查 `GET /health`、`GET /api/me`、上传、一次真实 Parquet 仿真、结果分页/下载、SSE 重连和
跨 owner 404。候选发布使用受保护的 `candidate-release` 环境，必须提供 `DEEPSEEK_API_KEY`，并从指定
workflow run 下载名为 `tensoreval-release` 的 artifact。artifact 顶层布局固定为：

```text
tensoreval
cwd/                         # TENSOREVAL_CWD，只读资源根
└── sparrowzz/dss/           # docs/catalog、合同内根 RSS、rssbundle、models 等
```

`cwd/` 还必须包含固定案例显式引用的跨目录依赖，例如合同需要的 `psa/tests/bmopf` 或
`sparrowzz/rspower/lib`。具体文件集合由 engine resource manifest 和 73 个案例的依赖闭包决定，不能只按
上面的目录示意手工挑选。artifact 应从获批的来源快照生成；下载后 workflow 校验 binary，后端启动
preflight 再校验 catalog、11 份文档、16 个根 RSS、57 个 rssbundle 及其依赖内容。

受保护环境还必须配置 `TENSOREVAL_SHA256` 变量。workflow 在执行前计算下载 binary 的 SHA-256 并与该
变量精确比较，因此仅有同名 artifact 或任意 run ID 不能替换已批准的仿真内核。发布报告保存测试 ID、
agent bundle 版本与目录 SHA-256、engine contract 和 binary SHA-256。

`.github/workflows/release-integration.yml` 先复用完整 PR 门禁，再执行 `scripts/test-release-integration` 和
`--require-release`。真实旅程验证全部 73 个固定案例、代表 study、DataFrame 潮流、RSS 短路、结果问答、
参数修改重跑和比较，并归档去敏后的 engine/LLM 报告、功能覆盖 JSON/HTML 与 Playwright 失败 trace。
缺少 binary、资源目录、secret、合同 hash 或任一语义断言时发布失败，不能静默跳过。2026-10-23 本地候选
发布验收已通过，engine contract 为 `dss-v2-d2b6d88a`；正式发布以 workflow 的
`release-gate-evidence-*` artifact 为审计记录。

若启动报告 `engine_resource_hash_mismatch`、`example_source_changed` 或 `engine_contract_mismatch`，应恢复
与当前 bundle 匹配的完整资源 artifact；如果变化是计划内升级，则按 Agent/engine 资源变更流程更新
manifest、bundle 和测试后发布新合同。不要删除校验、替换单个文件或让服务回退到其他 catalog。
