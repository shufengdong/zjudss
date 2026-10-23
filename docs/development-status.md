# V3 开发状态

## Current release

第三版已直接替换前两个未上线版本。当前产品合同是 DataFrame 模型、独立 RSS 脚本、用户显式运行和可删除的
单个结果；数据库只支持空库初始化，不存在旧协议兼容层。

| Area | Status | Evidence |
|---|---|---|
| 固定用户与隔离 | complete | Argon2id、HttpOnly Cookie、跨用户 404、SSE 撤销 |
| 模型导入 | complete | DSS/bundle/ZIP/CSV/Excel/Parquet、异步状态、确认/放弃、失败释放 |
| 草稿与版本 | complete | expected revision、整表覆盖、显式发布/激活、影响预览删除 |
| 案例库 | complete | 固定按钮进入普通导入，确认后只修改 draft |
| 脚本 | complete | 上传/Agent 创建、静态校验、预览/全文/复制、无编辑删除 |
| 运行 | complete | 仅按钮创建、active race、冻结快照、私有输入树、多 Parquet 结果 |
| 结果与派生 | complete | 单结果删除、分析、物化视图、比较、tombstone 血缘 |
| 前端 | complete | 三列/窄屏、运行记录、数据表/图表、登录用户菜单 |
| Agent V3 | complete | 16 工具、6 skills、按需引擎知识、无运行/发布/删除权 |
| 发布门禁 | complete | deterministic Rust/Vitest/build/Playwright/coverage；真实依赖另行执行 |

## Runtime contracts

- Agent bundle：`v3.0.0`，目录 `backend/resources/agent/v3`。
- Engine contract：`tensoreval-parquet-v3`；每次启动从当前 `TENSOREVAL_CWD` 动态发现并验证
  docs/catalog/examples，按实际读取字节计算不可变 snapshot 并发布到 DATA_DIR。
- 公共合同：41 条路由、9 类消息、16 个工具、6 个 skills、8 个上传来源、6 类视图。
- 模型表：adapter 固定输出 21 张规范 DataFrame，包含合法的零行表。

## Hard boundaries

所有 tensoreval 调用必须经过 adapter；所有 repository 查询带 `UserId`；所有结果使用 Snappy Parquet；浏览器与
LLM 不接触物理路径。脚本创建不运行，模型修改不发布，案例应用不发布，单条聊天消息不可删除，只能删除整个
会话。运行记录本身不可删除；历史版本删除和单结果删除使用各自明确规则。

启动会功能性探测当前 `TENSOREVAL_BIN` 的命名 Parquet、两种模型导入和脚本校验能力，并在每次启动子进程前
复核启动时的 binary SHA-256；替换 binary 后返回 `engine_binary_changed`，必须重启服务。资源扫描限制为 512 篇
Markdown、单篇 4 MiB、文档合计 64 MiB、2,048 个案例、每案例 10,000 文件/2 GiB、依赖深度 64；变更资源树
后也必须重启。运行时允许没有案例，但候选发布门禁要求至少一个根 RSS 和一个 bundle。

## Reset and operation

`init_users.csv` 是编译输入，密码初始等于 ID。CSV 字节变化后启动会返回
`user_seed_mismatch_reset_required`；必须同时重置 SQLite 与 DATA_DIR。公网可访问配置见
`docs/v3-deployment.md`，当前用户密码策略是已接受的试用风险。

## Verification

后端全量测试覆盖 schema、adapter、导入、版本、脚本、运行、删除、派生、比较、认证、恢复和配额。前端
Vitest 覆盖公共类型、state、workspace、脚本卡、运行与数据工作台。Playwright 覆盖真实浏览器到
Vite/Axum/SQLite/CAS 的身份、导入、案例、显式运行、派生血缘、删除、恢复、恶意 ZIP、配额和无障碍。
最终命令与真实 DeepSeek/tensoreval 候选门禁见部署文档。
