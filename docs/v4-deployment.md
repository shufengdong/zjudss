# V4 部署与发布

## Empty database cutover

V4 不迁移或兼容任何旧数据库。切换前停止服务，备份后同时移走旧 SQLite 文件和整个 `DATA_DIR`，再从空目录
执行唯一 initial migration；只删除其中一项会留下无法解释的 artifact 或元数据，禁止启动。确认新版本稳定前保留备份，
但不能把旧库或旧 artifact 混入 V4 目录。`backend/resources/init_users.csv` 必须为 `ID,NAME`；初始密码等于 ID。
种子 SHA-256 变化会让已有库启动失败，此时也必须再次同时重置数据库和 `DATA_DIR`。

## Required configuration

生产必须配置 `DATABASE_URL`、持久化 `DATA_DIR`、`TENSOREVAL_BIN`、`SPARROWZZ_DIR` 和 DeepSeek secret。
监听地址由 `BIND_ADDR` 设置，默认 `127.0.0.1:8080`，DellServer 使用 `127.0.0.1:809`。
binary 必须支持命名 Parquet input、`--result-dir`、`--output-format parquet` 与两种模型 import profile；RSS
语法不由 zjudss 静态校验。资源目录必须包含受支持 schema 的 catalog、可安全读取的文档与案例。启动从当前目录动态计算资源
snapshot，不固定 eig-rs/sparrowzz commit、binary SHA、资源 hash 或案例数量。生产没有 stub 回退，也不会自动
checkout、pull、reset、clean、构建或改写配置的 eig-rs 工作树。

应用配置及默认值如下；`TENSOREVAL_BIN`、`SPARROWZZ_DIR`、LLM secret 没有生产回退值。

| 变量 | 默认值 |
|---|---:|
| `BIND_ADDR` | `127.0.0.1:8080` |
| `DATABASE_URL` | `sqlite://backend.db` |
| `DATA_DIR` | `data` |
| `AGENT_BUNDLE_VERSION` | `v4.0.0` |
| `SESSION_TTL_DAYS` / `SESSION_COOKIE_SECURE` | `30` / `false` |
| `AGENT_MAX_CONCURRENT_RUNS` / `AGENT_MAX_USER_RUNS` / `AGENT_QUEUE_CAPACITY` | `8` / `2` / `32` |
| `TENSOREVAL_TIMEOUT_MS` / `AGENT_MODEL_RETRY_MAX` | `120000` / `2` |
| `SCRIPT_REPAIR_MAX_ATTEMPTS` / `SCRIPT_REPAIR_STDERR_MAX_BYTES` | `3` / `65536` |
| `ENGINE_PROCESS_OUTPUT_MAX_BYTES` | `1048576` |
| `UPLOAD_SINGLE_FILE_LIMIT_BYTES` / `UPLOAD_BATCH_LIMIT_BYTES` / `UPLOAD_STAGING_TTL_HOURS` | `52428800` / `524288000` / `24` |
| `ARCHIVE_MAX_ENTRIES` / `ARCHIVE_MAX_EXPANDED_BYTES` / `ARCHIVE_MAX_COMPRESSION_RATIO` | `1000` / `2147483648` / `100` |
| `USER_ARTIFACT_LIMIT` / `USER_STORAGE_LIMIT_BYTES` / `STORAGE_WARNING_RATIO` | `20000` / `21474836480` / `0.8` |
| `RESULT_MAX_FILES` / `RESULT_MAX_FILE_BYTES` / `RESULT_MAX_TOTAL_BYTES` | `100` / `536870912` / `2147483648` |

`SPARROWZZ_DIR` 直接指向 `eig-rs/sparrowzz`；案例根固定为 `SPARROWZZ_DIR/dss/rssbundle`。候选资源必须让每个案例以直接包含
`manifest.json` 的 `.rssbundle` 目录发布；中间目录只用于分类。manifest 声明的入口和 files 必须是 bundle 内安全
相对路径，文件存在且 SHA-256 一致。单个无效案例会作为不可导入叶节点及脱敏诊断保留，目录逃逸或全局限制错误
仍使启动失败。

资源会原子发布到 `DATA_DIR/system/engine-resources/<snapshotId>`，运行进程内保持不变；更新 eig-rs 后必须重启。
限制为 512 篇 Markdown、单篇 4 MiB、文档总计 64 MiB、2,048 个案例、每案例 10,000 文件/2 GiB、依赖深度
64。超过限制返回 `engine_resource_limit_exceeded`；扫描和发布之间发生变化返回
`engine_resource_changed_during_scan`。启动后可直接替换 binary，后续 adapter 调用使用路径上的当前普通文件；
文件不存在或不再是普通文件时返回 `engine_binary_unavailable`。替换后的 binary 不会重新执行启动能力探测。

公网由 HTTPS 反向代理提供 TLS，转发 `/api` 和 SSE，并设置：

```dotenv
SESSION_COOKIE_SECURE=true
```

Cookie 固定 HttpOnly、SameSite=Lax、Path=/。当前相同 ID/密码约定仅用于已批准的试用环境，不应宣称安全。

旧环境变量 `OWNER_ARTIFACT_LIMIT`、`OWNER_STORAGE_LIMIT_BYTES`、`AGENT_MAX_OWNER_RUNS` 会被启动配置明确拒绝。
V4 只接受 `USER_ARTIFACT_LIMIT`、`USER_STORAGE_LIMIT_BYTES`、`AGENT_MAX_USER_RUNS`。其他默认值和完整模板以
`backend/.env.example` 为准；生产至少显式设置 `DATABASE_URL`、`DATA_DIR`、`TENSOREVAL_BIN`、
`SPARROWZZ_DIR`、`DEEPSEEK_API_KEY` 和 `SESSION_COOKIE_SECURE=true`。

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

## DellServer 部署

访问地址 https://sgool.zju.edu.cn/zjudss/，文件仅放在 /home/sgoolweb 下，systemd unit 除外：

- /home/sgoolweb/zjudss/：zjudss、tensoreval、sparrowzz/ 和运行配置 .env；启动后生成 app.db 与 data/。
- /home/sgoolweb/zjudss/www/：前端 index.html 和 assets/。
- /etc/systemd/system/zjudss.service：安装仓库 deploy/zjudss.service，使用 sgoolweb 账号运行。

后端目录需要 sgoolweb 可读写，.env 权限为 0600，由 systemd 加载；引擎路径必须指向上述实际文件。
不保留 bin/、engine/、current、releases、构建信息、校验摘要、更新配置或更新脚本。
Nginx 由管理员维护：静态根为 /home/sgoolweb/zjudss/www/，/zjudss/api/ 转发到 http://127.0.0.1:809/api/，
SSE 关闭缓冲。前端构建使用 VITE_BASE_PATH=/zjudss/、VITE_API_BASE_PATH=/zjudss/api。

安装 unit 后执行 systemctl daemon-reload 与 systemctl enable --now zjudss.service。
809 属于低位端口，unit 通过 CAP_NET_BIND_SERVICE 允许 sgoolweb 绑定；.env 中的同名设置会覆盖 unit 的 Environment。
启动会通过 adapter 验证 tensoreval 的 Parquet 与两种导入能力；不兼容时应更新引擎。

后续在当前 Linux x86_64 开发机运行 `./scripts/deploy`，默认连接 SSH 别名 DellServer；
也可用 `./scripts/deploy 用户@主机` 指定目标。脚本本地执行 Cargo release、npm ci 和前端构建，
只经 SSH 上传 zjudss 与 www/，沿用远端 .env、tensoreval、sparrowzz/、数据库和 data/。
上传完成后通过 SSH 终端执行 sudo 停服、替换和启动；健康检查失败恢复旧程序与前端，持久化数据不回退。
临时上传目录会在正常完成或成功回退后清理。无需在目标机保存更新脚本、配置或构建工具。
编译前准备真实 backend/resources/init_users.csv；本机与目标机需兼容 CPU、glibc 和 SQLite 运行库。
可通过 DEPLOY_DIR、SERVICE、HEALTH_URL、HEALTH_TIMEOUT、VITE_BASE_PATH 和 VITE_API_BASE_PATH 覆盖默认值。
普通 CI 继续使用测试 CSV；真实依赖发布门禁使用 INIT_USERS_CSV Secret。

## Deterministic gate

V4 Task 3 起，候选发布还必须通过 `process_engine_v4`：stdout/stderr 有界流式捕获、运行中结果目录监控、超时/取消
进程组终止和退出后严格扫描均属于 adapter 发布门禁。RSS 脚本不再依赖 zjudss 静态语法校验能力。

```bash
cd backend
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
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

受保护环境提供 `DEEPSEEK_API_KEY` 与仓库级 `INIT_USERS_CSV`。workflow 每次检出 eig-rs 默认分支（含 submodule），构建当前
`tensoreval`，设置 `TENSOREVAL_BIN/SPARROWZZ_DIR` 并运行：

```bash
./scripts/test-release-integration
./scripts/check-functional-coverage --require-release
```

真实旅程从固定用户登录开始，上传 21 个静态导入 DataFrame 并确认拆分为 20 张模型表和一个 `para` 普通资源，
显式生成版本，让对话 Agent 通过动态目录建立研究配置，用户确认后由 ScriptAuthor 生成、执行并在需要时修正脚本；
成功后再次点击脚本“运行”，对 list 型 `bus_voltage` 做真实 `select` 结果分析，并用带稳定键和标量数值列的
`load_state` 验证运行比较，再删除一个来源结果并验证运行及物化派生仍可读取。引擎门禁
另外真实执行两种 import profile、命名 input、多个 Snappy Parquet 输出和失败诊断。报告保存测试 ID、Agent
bundle hash、用户 seed hash/数量、engine contract/checks 和 binary hash，不保存用户 ID、密码、Cookie、secret
或物理路径；同时记录动态 `engineResourceSnapshot`、rssbundle 发现/可导入/不可导入数量、发现的 study 类型与代表
运行。binary hash 和 snapshot 是本次证据，不是接收门槛。真实门禁要求至少一个 rssbundle 和一个可导入案例，
要求动态门面 `enabled_count == catalog_entry_count` 且零诊断，并为当前存在的 study family 各运行一个代表合同；
不写死门面数或案例数。个别坏案例可以在案例报告中保留脱敏诊断；当前真实门面目录的任何诊断都会让候选发布失败。

结果验收按逻辑 Arrow 类型族处理字符串和 list/matrix，非空合同通过实际值扫描证明；缺失 unit 元数据允许通过，但
存在且冲突的 unit 必须拒绝。该规则用于适配真实 tensoreval 的 `Utf8View`、`LargeList` 和物理 nullable 表示，不会
放宽列名、顺序、元素类型或实际 null。

服务重启会恢复未完成上传和受管 workflow；可安全继续的工作进入队列，无法继续的执行收敛为稳定终态。每次启动只读取
当前 `SPARROWZZ_DIR` 并发布新的不可变 snapshot；运行中的历史继续引用旧 snapshot。资源树变化后必须重启，单独替换
`TENSOREVAL_BIN` 则由下一次 adapter 调用直接读取当前普通文件。

## Failure handling

`engine_contract_mismatch` 表示当前 binary/catalog 不满足应用协议；`engine_resource_hash_mismatch` 表示 bundle
自校验或已发布 snapshot 损坏。应修复当前 eig-rs/配置或正式更新应用协议与测试，不得放宽校验或退回
CSV/stub。运行中断会在重启后收敛失败；已发布模型、脚本、结果和分析不回滚。
