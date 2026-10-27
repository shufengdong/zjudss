# V4 开发状态（实施中）

> V4 规划：负荷预测等辅助 DataFrame 将使用独立数据资源库，不再进入模型版本；V4 从空数据库建立，
> 不迁移或兼容 V3 持久化数据。已确认需求见
> [`2026-10-26-independent-data-resources-design.md`](superpowers/specs/2026-10-26-independent-data-resources-design.md)。
> 详细实施步骤见
> [`2026-10-27-v4-independent-data-resources-managed-studies.md`](superpowers/plans/2026-10-27-v4-independent-data-resources-managed-studies.md)。
> 当前 eig-rs 已提供全部门面的完整传递输入、Struct 参数卡和结果 schema，外部机器合同门禁已经满足。V4 Task 1
> 的空库持久化合同、Task 2 的动态研究门面目录、Task 3 的有界进程执行边界和 Task 4 的可恢复统一上传批次已经实现；
> 独立数据资源发布和受管研究工作流尚未实现。
> 本文未明确标为 V4 已完成的其余内容仍描述进入 V4 前已验证的 V3 产品行为。

## V4 implementation progress

| Task | Status | Evidence |
|---|---|---|
| Task 1：空库 schema、配置与合同基线 | complete | `6607387`，V4 schema/config/manifest 聚焦测试 |
| Task 2：动态 DataFrame 研究门面 | complete | 独立 catalog 版本、15 门面真实目录门禁、隔离诊断、传递 schema/hash、current snapshot 事务 |
| Task 3：统一 tensoreval 进程边界 | complete | 双流有界捕获/完整 hash、运行中结果监控、进程组超时与取消、最终严格扫描、无部分结果 |
| Task 4：统一上传、解析设置与用途确认 | complete | upload batch API、24h 可配置 TTL、并发 reservation、CSV/Excel/Parquet/DSS/案例候选、20+para 拆分、动态 catalog 表头推断 |
| Tasks 5–15 | planned | 以 V4 实施计划为准 |

Task 4 的后端聚焦测试和新增 upload batch API Playwright 旅程已通过。完整旧版 Playwright 套件仍依赖已删除的
`/model-imports` 协议和“确认合并”界面，当前失败属于 Task 5 发布事务与 Task 12 前端迁移尚未实施的预期中间状态，
不恢复旧协议作为兼容层。

## Current release

第三版已直接替换前两个未上线版本。当前产品合同是 DataFrame 模型、独立 RSS 脚本、用户显式运行和可删除的
单个结果；数据库只支持空库初始化，不存在旧协议兼容层。

| Area | Status | Evidence |
|---|---|---|
| 固定用户与隔离 | complete | Argon2id、HttpOnly Cookie、跨用户 404、SSE 撤销 |
| 统一上传 | transitioning | Task 4 已实现 DSS/ZIP/CSV/Excel/Parquet/案例的可恢复候选、解析设置/用途确认与放弃；Task 5 接入模型/数据双发布确认 |
| 草稿与版本 | complete | expected revision、整表覆盖、显式发布/激活、影响预览删除 |
| 案例库 | complete | rssbundle 目录树、manifest 详情/搜索、无效叶诊断、严格清单导入、窄屏浏览 |
| 脚本 | transitioning | 上传/Agent 创建已改为保存候选并把语法/运行可行性交给 tensoreval；V4 受管确认与结果合同在 Tasks 9–11 完成 |
| 运行 | complete | 仅按钮创建、active race、含脚本名的冻结快照、私有输入树、可展开失败详情与多 Parquet 结果 |
| 结果与派生 | complete | 单结果删除、分析、物化视图、比较、tombstone 血缘 |
| 前端 | complete | 三列/窄屏、运行摘要与失败原因/耗时/全结果选择、结果数据表/图表、active/draft/历史模型表只读工作台、登录用户菜单 |
| Agent V3 | complete | 18 工具、6 skills、五类研究确定性规划、active 动态输入 alias、模型行读取、按需引擎知识、无运行/发布/删除权 |
| 发布门禁 | complete | deterministic Rust/Vitest/build/Playwright/coverage；真实依赖另行执行 |

## Runtime contracts

- 后端 Cargo package 与 library crate 均为 `zjudss`；`backend/` 只是仓库目录名，Rust 调用点使用
  `zjudss::`。
- Agent bundle：`v3.3.3`，目录 `backend/resources/agent/v3`。
- Engine contract：`tensoreval-parquet-v3`；每次启动从当前 `SPARROWZZ_DIR` 动态发现并验证
  `dss/docs/catalog` 与 `dss/rssbundle` manifest，按实际读取字节计算不可变 snapshot 并发布到 DATA_DIR。
- 每次 tensoreval 调用均由统一 adapter 流式排空 stdout/stderr；每流按 `ENGINE_PROCESS_OUTPUT_MAX_BYTES` 保留等额首尾并
  记录完整 hash/总字节数。运行中和退出后都检查结果数量、单文件/总量、普通文件和路径边界；失败、超时或取消会
  终止整个进程组并清理未发布目录。
- 公共合同：50 条路由、9 类消息、18 个工具、6 个 skills、8 个上传来源、6 类视图。
- 静态 DSS adapter profile 固定输出 21 张规范 DataFrame；统一上传把其中 20 张网络表作为模型候选，把 `para`
  作为普通 Data resource 候选。其他模型表完全由当前 engine snapshot 的有效 model schema 和实际表头推断。
- 研究规划：潮流、QSTS、短路、谐波和暂态只基于 active 模型的实际表清单与当前 engine snapshot；
  `inputAliases` 与从 active 表动态排列的 `modelInputOrder` 随 ready plan 冻结。规范输入缺失时一次返回全部
  `model.tables.<alias>`，不签发 plan ID。静态潮流生成不再查询模型输入顺序，参数/结果语法最多进行一次文档
  搜索和一次章节读取，确保在 8 次模型调用预算内创建脚本；轮次耗尽时返回明确的未创建脚本提示并写后台日志。
  研究请求字段按 camelCase 严格反序列化；显式 `newton` 在径向网络上仍选择 `ds_newton_raphson_pf`，只有
  `auto` 才按模型事实在 Radial 与 Newton 间自动选择，未知或拼错字段不会静默回退默认值。
  Agent 脚本直接使用运行时注入的 alias，且 DataFrame 脚本只包含变量声明；函数结果绑定中的叶变量直接成为
  结果，不追加裸变量表达式。受管脚本若导入/读取文件、引用计划外输入、覆盖输入或包含裸输出表达式，在持久化前
  以 `script_error` 拒绝。当前暂态仅支持电机—网络动态。
- 模型表读取：active、准确 draft revision 和历史版本均支持元数据、分页投影与 Parquet 下载；Agent 可通过
  有界等值查询读取实际行后再修改，读取不改变模型生命周期。

## Backend dependency baseline

2026-10-24 完成后端直接依赖刷新：`argon2 0.6.0`、`sha2 0.11.1`、`tokio 1.54.1` 和
`zip 9.1.0`；新增直接使用的 `getrandom 0.4.3`、`hex 0.4.3`，移除未直接使用的 `reqwest`、
`schemars` 与 `rand_core`。ZIP 继续关闭默认 feature，仅启用 `deflate`；Arrow 与 Parquet 保持
`60.0.0` 对齐。升级只包含必要的 API 兼容修改，不改变 REST/SSE、数据库、Agent 工具或引擎边界。

## Backend maintainability baseline

后端以 `cargo fmt` 和全 target、全 feature 的零警告 Clippy 作为提交门槛。当前公共实现已收敛以下重复逻辑：

- Parquet artifact 元数据由 `ArtifactMetadata::parquet()` 统一创建；
- 仿真、分析与比较事件中的数据集引用由 `ResultOutputRef::from_dataset()` 统一映射；
- 仿真和分析仓储共用内部 `StateTransition` 表达 running/failed 状态字段；
- `RunService` 使用具名依赖结构装配，避免依赖顺序错误；
- 历史搜索的 SQL、结果映射和索引重建步骤集中定义，仓储异步边界保持可读。

领域错误类型仍保留在 adapter、repository 和各 application service 内，避免把不同边界的稳定错误码合并成
一个含糊的全局错误。上述整理不改变 REST/SSE、数据库 schema、Agent 工具或引擎合同。

## Hard boundaries

所有 tensoreval 调用必须经过 adapter；所有 repository 查询带 `UserId`；所有模型和结果正文使用 Snappy Parquet；浏览器与
LLM 不接触物理路径。脚本创建不运行，模型修改不发布，案例应用不发布，单条聊天消息不可删除，只能删除整个
会话。运行记录本身不可删除；历史版本删除和单结果删除使用各自明确规则。

运行创建、列表和详情统一返回不可变脚本名。前端运行记录默认收起，按冻结审计信息显示研究类型；成功记录展开后按需列出全部
存活数据集，失败记录显示实际阶段、服务端清洗后的公共原因、退出码、起止时间和耗时；tensoreval 非零退出时还显示有界多行报错，
其中服务器绝对路径和控制字符在持久化与出网两层清理。删除单个数据集会失效同会话运行结果缓存，
不会删除运行或自动启动新运行。

启动会功能性探测当前 `TENSOREVAL_BIN` 的命名 Parquet 和两种模型导入能力；启动后替换 binary
无需重启，下一次调用直接使用路径上的当前普通文件，只有文件不存在或不再是普通文件时才拒绝。资源扫描限制为 512 篇
Markdown、单篇 4 MiB、文档合计 64 MiB、2,048 个案例、每案例 10,000 文件/2 GiB、依赖深度 64；变更资源树
后也必须重启。案例只发现 `SPARROWZZ_DIR/dss/rssbundle` 下直接含 manifest 的 `.rssbundle` 目录；运行时允许没有案例，
候选发布门禁要求至少一个 rssbundle 和一个可导入案例，并对当前存在的受支持 study 类型各运行一个代表案例。

## Reset and operation

`init_users.csv` 是编译输入，密码初始等于 ID。CSV 字节变化后启动会返回
`user_seed_mismatch_reset_required`；必须同时重置 SQLite 与 DATA_DIR。公网可访问配置见
`docs/v3-deployment.md`，当前用户密码策略是已接受的试用风险。

## Verification

后端全量测试覆盖 schema、adapter、导入、版本、模型表读取、脚本、运行、删除、派生、比较、认证、恢复和配额。前端
Vitest 覆盖公共类型、state、workspace、模型表、脚本卡、运行与数据工作台。Playwright 覆盖真实浏览器到
Vite/Axum/SQLite/CAS 的身份、导入、分层案例目录与窄屏详情、显式运行、可展开运行详情与多结果选择、派生血缘、删除、恢复、恶意 ZIP、配额和无障碍。
最终命令与真实 DeepSeek/tensoreval 候选门禁见部署文档。

动态脚本输入合同由 `planning_lists_every_missing_canonical_table_from_the_active_version`、
`plan_study_preserves_explicit_newton_and_issues_request_local_ready_ids`、
`planned_agent_script_cannot_reimport_the_active_model` 和
`planned_agent_script_cannot_overwrite_an_active_input_alias` 覆盖；轮次耗尽诊断由
`turn_limit_failure_explains_that_no_script_was_created` 覆盖；DataFrame 声明语法由
`static_validation_rejects_a_trailing_bare_output_expression` 覆盖。

当前后端基线通过 `cargo fmt --all -- --check`、
`cargo clippy --all-targets --all-features -- -D warnings` 及 `cargo test --features e2e-test`。
