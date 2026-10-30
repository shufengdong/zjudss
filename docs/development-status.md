# V4 开发状态

> V4 规划：负荷预测等辅助 DataFrame 将使用独立数据资源库，不再进入模型版本；V4 从空数据库建立，
> 不迁移或兼容 V3 持久化数据。已确认需求见
> [`2026-10-26-independent-data-resources-design.md`](superpowers/specs/2026-10-26-independent-data-resources-design.md)。
> 详细实施步骤见
> [`2026-10-27-v4-independent-data-resources-managed-studies.md`](superpowers/plans/2026-10-27-v4-independent-data-resources-managed-studies.md)。
> 当前 eig-rs 已提供全部门面的完整传递输入、Struct 参数卡和结果 schema，外部机器合同门禁已经满足。V4 Task 1
> 的空库持久化合同、Task 2 的动态研究门面目录、Task 3 的有界进程执行边界、Task 4 的可恢复统一上传批次和
> Task 5 的独立数据资源发布/管理、Task 6 的目录驱动研究配置、Task 7 的 durable 工作流消息、历史锚定与会话删除合同、Task 8 的 V4 对话 Agent bundle、Task 9 的受管工作流持久状态机、Task 10 的生成/执行/自动修正、Task 11 的脚本重跑、失败重试和运行删除、Task 12 的统一上传与数据文件工作台、Task 13 的研究确认与受管执行前端、Task 14 的确定性全链路验收，以及 Task 15 的真实依赖发布门禁已经实现。
> 当前运行合同已经切换到 V4；历史 plans/spec 仅保留实施记录。

## V4 implementation progress

| Task | Status | Evidence |
|---|---|---|
| Task 1：空库 schema、配置与合同基线 | complete | `6607387`，V4 schema/config/manifest 聚焦测试 |
| Task 2：动态 DataFrame 研究门面 | complete | 独立 catalog 版本、15 门面真实目录门禁、隔离诊断、传递 schema/hash、current snapshot 事务 |
| Task 3：统一 tensoreval 进程边界 | complete | 双流有界捕获/完整 hash、运行中结果监控、进程组超时与取消、最终严格扫描、无部分结果 |
| Task 4：统一上传、解析设置与用途确认 | complete | upload batch API、24h 可配置 TTL、并发 reservation、CSV/Excel/Parquet/DSS/案例候选、20+para 拆分、动态 catalog 表头推断 |
| Task 5：独立数据资源发布与管理 | complete | model/resource 双子事务、普通 Data/Struct 资源、替换/删除/提升、资源分析与物化视图、4 个 Agent 工具 |
| Task 6：目录驱动研究配置 | complete | revision 状态机、动态 model/Data/Struct/结果卡、复用建议、多资源消歧、受控列映射、最终确认准备合同、2 个 Agent 工具 |
| Task 9：受管工作流状态机 | complete | 原子确认、Struct 资源发布、冻结输入、provisional run、专用 ScriptAuthor、candidate/父 attempt、查询/取消 API、启动恢复 |
| Task 10：脚本生成、执行与自动修正 | complete | 结构合同检查、冻结输入树、共享调度与会话取消、tensoreval 立即执行、3 次修正/4 次执行上限、64 KiB 脱敏诊断、精确结果合同、独占原子结果发布与崩溃孤儿回收 |
| Task 11：脚本重跑、失败重试与运行删除 | complete | 最新所需输入冻结、开放/精确结果合同、派生 retry workflow、输入差异确认、run/result 原子 tombstone、artifact 引用释放 |
| Task 7：durable 消息、历史锚定与会话删除 | complete | 3 种 revisioned 操作卡、`messageId:revision` SSE、连续 around window、影响 token 删除与统一取消 |
| Task 8：V4 Agent bundle 与受控工具 | complete | `v4.0.0`、5 个按需 skills、22 个对话工具、无聊天脚本创建或旧规划权限 |
| Task 12：统一上传与数据文件工作台 | complete | 旧模型导入前端已移除、durable batch 摘要恢复、用途/隐藏 sheet/双子事务确认；案例完整卡归入模型工作区，普通上传完整卡归入数据文件页；资源分页预览/下载/删除/replacement、模型版本描述 |
| Task 13：研究确认与受管执行前端 | complete | 对话内上传摘要与完整研究配置卡、目录类型参数、revision stale 刷新、attempt/取消/失败重试、精确脚本定位、run 删除影响确认 |
| Task 14：确定性全链路测试与功能追溯 | complete | 29 条 Playwright 旅程；326/326 功能证据、274/274 公共合同、PR 覆盖 100% |
| Task 15：文档、真实依赖门禁与发布切换 | complete | V4 当前文档与空库部署合同；动态真实引擎门禁；固定用户登录、真实 LLM 规划、受管执行/重跑、分析、比较和删除语义旅程 |

旧模型导入前端协议和旧确认界面已经移除，不提供兼容层。Playwright 已统一使用 V4 upload batch、
数据资源和受管研究全链路旅程。

## Current release

V4 已直接替换未上线的旧版本。当前产品合同是模型版本、独立 Data/Struct 资源、目录驱动研究配置、确认后受管
生成/执行/修正，以及可再次运行和删除的脚本结果；数据库只支持空库初始化，不存在旧协议兼容层。

| Area | Status | Evidence |
|---|---|---|
| 固定用户与隔离 | complete | Argon2id、HttpOnly Cookie、跨用户 404、SSE 撤销 |
| 统一上传 | complete | DSS/ZIP/CSV/Excel/Parquet/案例的可恢复候选、解析设置/用途确认、model/resource 双发布、部分成功重试与确认幂等 |
| 独立数据资源 | complete | 会话唯一 alias、原文件/规范 Parquet、Struct 覆盖预览、重新上传替换、删除影响、结果提升、分析/物化视图输入 |
| 草稿与版本 | complete | expected revision、整表覆盖、显式发布/激活、影响预览删除 |
| 案例库 | complete | schema v2 manifest 与 projection profile 严格匹配；当前 144/144 案例通过受控 projector 转成目录声明的 Model/Data/Struct 候选，diagnostic 不进入输入；目录树、详情/搜索、无效叶诊断和窄屏浏览完整 |
| 脚本与运行 | complete | 受管 Agent 脚本已实现候选持久化、直接执行、自动修正和冻结结果合同；成功脚本单次运行读取最新所需输入，失败脚本可比较输入后派生“重新尝试”，手写/上传脚本开放结果合同且不修正；终态 run 支持影响预览和原子删除 |
| 结果与派生 | complete | 单结果删除、分析、物化视图、比较、tombstone 血缘 |
| 前端 | complete | 三列/窄屏、运行摘要与失败原因/耗时/全结果选择、结果数据表/图表、active/draft/历史模型表只读工作台、登录用户菜单 |
| Agent bundle | complete | V4 `v4.0.0`、22 个受控工具；DeepSeek/百炼 provider；`search_study_facades`/`prepare_study` 准备配置卡，对话 Agent 无通用脚本创建权限 |
| 发布门禁 | complete | deterministic Rust/Vitest/build/Playwright/coverage；真实 eig-rs/tensoreval 与所选 LLM provider 候选门禁 |

## Runtime contracts

- 后端 Cargo package 与 library crate 均为 `zjudss`；`backend/` 只是仓库目录名，Rust 调用点使用
  `zjudss::`。
- Agent bundle：`v4.0.0`，目录 `backend/resources/agent/v4`。
- LLM provider：缺省为 DeepSeek；也可通过 `LLM_PROVIDER=bailian`、显式模型、同地域 API Key 和 OpenAI-compatible endpoint 使用百炼。两者共用同一 Agent、工具和 ScriptAuthor 合同。
- Engine contract：`tensoreval-parquet-v3`；每次启动从当前 `SPARROWZZ_DIR` 动态发现并验证
  `dss/docs/catalog`（含 case projection schema v2）与 `dss/rssbundle` manifest schema v2，按实际读取字节计算不可变 snapshot 并发布到 DATA_DIR。
- 每次 tensoreval 调用均由统一 adapter 流式排空 stdout/stderr；每流按 `ENGINE_PROCESS_OUTPUT_MAX_BYTES` 保留等额首尾并
  记录完整 hash/总字节数。运行中和退出后都检查结果数量、单文件/总量、普通文件和路径边界；失败、超时或取消会
  终止整个进程组并清理未发布目录。
- 公共合同：75 条路由、12 类消息、22 个工具、5 个 skills、8 个上传来源、6 类视图。
- 静态 DSS adapter profile 固定输出 21 张规范 DataFrame；统一上传把其中 20 张网络表作为模型候选，把 `para`
  作为普通 Data resource 候选。其他模型表完全由当前 engine snapshot 的有效 model schema 和实际表头推断。
- 研究配置：`search_study_facades` 按当前 snapshot 返回目录门面、方法差异和禁用诊断，显式函数不会被拓扑改写；
  `prepare_study` 持久化 revisioned 配置卡。model 输入仅绑定 active 的实际门面表，Struct/Data 绑定普通资源，
  多资源必须选择，列映射发布带血缘的新 Parquet 资源。结果集合按 fixed/minimum/conditional 目录约束校验。
  `prepare_confirmation` 重新检查 revision、snapshot、门面 hash、active 模型和资源内容，生成 Task 9 原子事务使用的
  locked contract/input snapshot/replacement commands；confirm 公共路由原子创建受管 workflow 与 provisional run，并由无工具 ScriptAuthor 生成候选。候选先经过只证明冻结函数、alias 顺序和结果绑定未变化的结构合同检查，再通过统一 adapter 调用 tensoreval。非零退出或可修正的结果限制/合同错误最多修正 3 次；超时、监控、路径、配额和基础设施错误立即失败。成功仅在全部结果名称、列顺序、逻辑 dtype、实际非空值和可用 unit 元数据满足 Snappy Parquet 结果合同时原子发布。
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
LLM 不接触物理路径。脚本创建不运行，模型修改不发布，案例应用不发布，单条聊天消息不可删除，只能通过影响预览和短期
token 删除整个会话。删除确认后先隐藏会话并取消其 Agent/engine 工作，再清理内容并保留最小 tombstone。终态运行通过影响预览和
短期 token 删除，运行及其全部直接结果原子 tombstone；历史版本删除和单结果删除使用各自明确规则。

运行创建、列表和详情统一返回不可变脚本名。前端运行记录默认收起，按冻结审计信息显示研究类型；成功记录展开后按需列出全部
存活数据集，失败记录显示实际阶段、服务端清洗后的公共原因、退出码、起止时间和耗时；tensoreval 非零退出时还显示有界多行报错，
其中服务器绝对路径和控制字符在持久化与出网两层清理。删除单个数据集会失效同会话运行结果缓存，
不会删除运行或自动启动新运行。

启动会功能性探测当前 `TENSOREVAL_BIN` 的命名 Parquet 和两种模型导入能力；启动后替换 binary
无需重启，下一次调用直接使用路径上的当前普通文件，只有文件不存在或不再是普通文件时才拒绝。资源扫描限制为 512 篇
Markdown、单篇 4 MiB、文档合计 64 MiB、2,048 个案例、每案例 10,000 文件/2 GiB、依赖深度 64；变更资源树
后也必须重启。案例只发现 `SPARROWZZ_DIR/dss/rssbundle` 下直接含 manifest 的 `.rssbundle` 目录；运行时允许没有案例。
manifest 只有逐字段匹配启用的 snapshot projection profile 才可导入；当前 144 个案例全部满足。候选发布门禁要求
不可导入案例为零，对全部 144 个案例逐一真实运行 projector 并验证目录声明的 Model/Data/Struct/diagnostic 输出；
当前证据为 144/144 成功、0 失败。之后仅对每个被案例引用的 projection profile 选择一个成功代表，运行只接受
DataFrame 命名输入的 `managed.rss`，当前为 6 个代表。当前 profile 和案例分布见 `docs/v4-deployment.md`。

## Reset and operation

`init_users.csv` 是编译输入，密码初始等于 ID。CSV 字节变化后启动会返回
`user_seed_mismatch_reset_required`；必须同时重置 SQLite 与 DATA_DIR。公网可访问配置见
`docs/v4-deployment.md`，当前用户密码策略是已接受的试用风险。

## Verification

后端全量测试覆盖 schema、adapter、导入、版本、模型表读取、脚本、运行、删除、派生、比较、认证、恢复和配额。前端
Vitest 覆盖公共类型、state、workspace、模型表、脚本卡、运行与数据工作台。Playwright 覆盖真实浏览器到
Vite/Axum/SQLite/CAS 的身份、导入、分层案例目录与窄屏详情、显式运行、可展开运行详情与多结果选择、派生血缘、删除、恢复、恶意 ZIP、配额和无障碍。
最终命令与真实 LLM provider/tensoreval 候选门禁见部署文档。

研究配置合同由 `backend/tests/study_configurations.rs` 覆盖：显式 Newton、目录动态模型输入、保护模型范围、独立
`para`、多资源不猜选、Struct replacement intent、revision/用户隔离、无提前 confirm 路由及受控列映射发布。

当前确定性基线通过 `cargo fmt --all -- --check`、`cargo clippy --all-targets --all-features -- -D warnings`、
`cargo test --features e2e-test`、Vitest、生产构建、29 条 Playwright 旅程及 100% PR 功能追溯；候选门禁另以真实
所选真实 LLM provider、当前 eig-rs/tensoreval、动态门面与案例快照完成规划、执行、分析、比较和删除全旅程。
候选门禁中的 tensoreval 固定以 `--no-default-features --features tensoreval-cli,with_dsa_mumps` 构建，不使用
eig-rs 默认 features。
