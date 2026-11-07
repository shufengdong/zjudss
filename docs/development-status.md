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
| Task 2：动态 DataFrame 研究门面 | complete | 独立 catalog 版本、动态门面真实目录门禁、隔离诊断、传递 schema/hash、current snapshot 事务 |
| Task 3：统一 tensoreval 进程边界 | complete | 双流有界捕获/完整 hash、运行中结果监控、进程组超时与取消、最终严格扫描、无部分结果 |
| Task 4：统一上传、解析设置与用途确认 | complete | upload batch API、24h 可配置 TTL、并发 reservation、CSV/Excel/Parquet/DSS/案例候选、20+para 拆分、动态 catalog 表头推断 |
| Task 5：独立数据资源发布与管理 | complete | model/resource 双子事务、普通 Data/Struct 资源、替换/删除/提升、资源分析、4 个 Agent 工具 |
| Task 6：目录驱动研究配置 | complete | revision 状态机、动态 model/Data/Struct/结果卡、唯一资源自动绑定与真实多资源消歧、server-owned 结果 selector、受控列映射、待确认结果可继续编辑、最终确认准备合同、3 个 Agent 工具 |
| Task 9：受管工作流状态机 | complete | 原子确认、用户参数与结果 selector 私有 overlay、可空 source/effective 冻结输入且不发布 Struct 资源、provisional run、专用 ScriptAuthor、candidate/父 attempt、查询/取消 API、启动恢复 |
| Task 10：脚本生成、执行与自动修正 | complete | RSS 必需语法章节优先注入 generation/repair、结构合同强制多结果方括号解构、冻结输入树、共享调度与会话取消、tensoreval 立即执行、3 次修正/4 次执行上限、原样修复只审计不重跑、64 KiB 脱敏 stdout/stderr、带日志引用的结构化失败分析、精确结果合同、独占原子结果发布与崩溃孤儿回收 |
| Task 11：脚本重跑、失败重试与运行删除 | complete | 最新所需输入冻结、用户/selector overlay 按显式 replacement 血缘跨 retry/rerun 重建、source-less 与旧 snapshot 兼容、开放/精确结果合同、派生 retry workflow、输入差异确认、普通 run 持久失败日志/智能分析、run/result 原子 tombstone、artifact 引用释放 |
| Task 7：durable 消息、历史锚定与会话删除 | complete | 3 种 revisioned 操作卡、`messageId:revision` SSE、连续 around window、影响 token 删除与统一取消 |
| Task 8：V4 Agent bundle 与受控工具 | complete | `v4.4.0`、5 个按需 skills（`study-planning@1.5.0`）、23 个对话工具；研究配置包含模型生成的详细说明和服务端权威页脚；明确自然语言可发布精确草稿并自动激活，也可按当前卡片精确 revision 修改结果；无聊天脚本创建、图表创建、自动确认或旧规划权限 |
| Task 12：统一上传与数据文件工作台 | complete | 旧模型导入前端已移除、durable batch 摘要恢复、用途/隐藏 sheet/双子事务确认；案例完整卡归入模型工作区，普通上传完整卡归入数据文件页；资源分页预览/下载/删除/replacement、模型版本描述 |
| Task 13：研究确认与受管执行前端 | complete | 对话内上传摘要与可按配置记忆收起状态的研究配置卡、只呈现待处理输入、参数模板只读与运行覆盖提示、server-owned selector 隐藏、错误自动展开、目录类型参数、revision stale 刷新、attempt/取消/失败重试、精确脚本定位、run 删除影响确认 |
| Task 14：确定性全链路测试与功能追溯 | complete | 31 条 Playwright 旅程；350/350 功能证据、297/297 公共合同、PR 覆盖 100% |
| Task 15：文档、真实依赖门禁与发布切换 | complete | V4 当前文档与空库部署合同；动态真实引擎门禁；固定用户登录、真实 LLM 规划、受管执行/重跑、分析、比较和删除语义旅程 |
| 用户配置图表工作台 | complete | 六类受控配置、capability/预览/保存/会话发现/删除七条 API、不可变 Snappy Parquet Plot Data、来源删除后恢复、390px 与跨用户浏览器旅程；不授予 LLM 图表权限 |

旧模型导入前端协议和旧确认界面已经移除，不提供兼容层。Playwright 已统一使用 V4 upload batch、
数据资源和受管研究全链路旅程。

## Current release

V4 已直接替换未上线的旧版本。当前产品合同是模型版本、独立 Data/Struct 资源、目录驱动研究配置、确认后受管
生成/执行/修正，以及可再次运行和删除的脚本结果；数据库只支持空库初始化，不存在旧协议兼容层。

| Area | Status | Evidence |
|---|---|---|
| 固定用户与隔离 | complete | Argon2id、HttpOnly Cookie、跨用户 404、SSE 撤销 |
| 统一上传 | complete | DSS/ZIP/CSV/Excel/Parquet/案例的可恢复候选、解析设置/用途确认、model/resource 双发布、部分成功重试与确认幂等 |
| 独立数据资源 | complete | 会话唯一 alias、原文件/规范 Parquet、Struct 覆盖预览、重新上传替换、删除影响、结果提升和分析输入 |
| 草稿与版本 | complete | expected revision、整表覆盖、显式发布/激活、影响预览删除 |
| 案例库 | complete | schema v2 manifest 与 projection profile 严格匹配；2026-11-06 当前候选 150/150 案例通过受控 projector 转成目录声明的 Model/Data/Struct 候选，diagnostic 不进入输入；原生结果允许包含未进入受管有序子集的附加结果 |
| 脚本与运行 | complete | 受管 Agent 脚本已实现候选持久化、直接执行、自动修正和冻结结果合同；普通运行与工作流共用脱敏日志/失败分析器，失败进度卡可恢复展示诊断；成功脚本单次运行读取最新所需输入，失败脚本可比较输入后派生“重新尝试”，手写/上传脚本开放结果合同且不修正；终态 run 支持影响预览和原子删除 |
| 结果与派生 | complete | 单结果删除、分析、用户配置图表、比较、tombstone 血缘；图表冻结来源快照与自己的 Plot Data，来源删除后仍可发现和读取 |
| 前端 | complete | 三列/窄屏、模型详细说明先于研究配置卡、配置卡按浏览器会话记忆收起状态且错误自动展开、Struct 参数显示只读模板身份与本次运行覆盖、运行摘要与失败原因/耗时/全结果选择、普通运行与研究工作流共用失败诊断面板、成功研究的脚本/运行结果成果归组与脚本锚定、pending-only 轮询、结果数据表、用户配置图表工作台、active/draft/历史模型表只读工作台、登录用户菜单 |
| Agent bundle | complete | V4 `v4.4.0`、`study-planning@1.5.0`、23 个受控工具；DeepSeek/百炼 provider；`publish_model_version` 只响应明确发布意图并发布精确草稿 revision；`search_study_facades`/`prepare_study` 要求模型详细说明，并以服务端权威页脚先于配置卡完成准备；`update_study_results` 只修改当前未确认卡片结果，对话 Agent 无通用脚本、图表创建或最终确认权限 |
| 发布门禁 | complete | deterministic Rust/Vitest/build/Playwright/coverage；真实 eig-rs/tensoreval 与所选 LLM provider 候选门禁 |

## Runtime contracts

- 后端 Cargo package 与 library crate 均为 `zjudss`；`backend/` 只是仓库目录名，Rust 调用点使用
  `zjudss::`。
- Agent bundle：`v4.4.0`，目录 `backend/resources/agent/v4`；研究规划 skill 为 `study-planning@1.5.0`。
- LLM provider：缺省为 DeepSeek；也可通过 `LLM_PROVIDER=bailian`、显式模型、同地域 API Key 和 OpenAI-compatible endpoint 使用百炼。两者共用同一 Agent、工具和 ScriptAuthor 合同。
- Engine contract：`tensoreval-parquet-v3`；每次启动从当前 `SPARROWZZ_DIR` 动态发现并验证
  `dss/docs/catalog`（含 case projection schema v2）与 `dss/rssbundle` manifest schema v2，按实际读取字节计算不可变 snapshot 并发布到 DATA_DIR。
- 每次 tensoreval 调用均由统一 adapter 流式排空 stdout/stderr；每流按 `ENGINE_PROCESS_OUTPUT_MAX_BYTES` 保留等额首尾并
  记录完整 hash/总字节数。运行中和退出后都检查结果数量、单文件/总量、普通文件和路径边界；失败、超时或取消会
  终止整个进程组并清理未发布目录。
- 公共合同：81 条唯一 method/path 路由、12 类消息、23 个工具、5 个 skills、8 个上传来源、6 类图表。
- 静态 DSS adapter profile 固定输出 21 张规范 DataFrame；统一上传把其中 20 张网络表作为模型候选，把 `para`
  作为普通 Data resource 候选。其他模型表完全由当前 engine snapshot 的有效 model schema 和实际表头推断。
- 研究配置：`search_study_facades` 按当前 snapshot 返回目录门面、方法差异和禁用诊断，显式函数不会被拓扑改写；
  `prepare_study` 要求 1..=4000 个 Unicode 字符的模型说明，持久化“详细说明 + 服务端实际标题/配置 ID 页脚”后再持久化 revisioned 配置卡。model 输入仅绑定 active 的实际门面表，Struct/Data 绑定普通资源，
  唯一资源自动绑定且只有真实多资源才要求选择，列映射发布带血缘的新 Parquet 资源。Struct 资源只作为参数模板；用户字段和结果 selector 都不修改长期资源，而是在 workflow 私有 Snappy Parquet 中分类冻结，无来源时从目录默认值建立私有输入；
  `ready_to_confirm` 卡仍可从 UI 或 skill-gated `update_study_results` 修改结果，二者都只递增 revision，最终确认仍只由固定按钮完成；
  确认成功会以同一配置 ID 原位发布 `confirmed` 卡片，Agent 历史按用户、会话和数据库权威状态筛选最新可编辑配置，不会把滞后的
  `study-config` 消息重新解释为当前待确认配置。
  `prepare_confirmation` 重新检查 revision、snapshot、门面 hash、active 模型和资源内容，生成 Task 9 原子事务使用的
  locked contract/input snapshot/private Struct overlays；confirm 公共路由原子创建受管 workflow 与 provisional run，并由无工具 ScriptAuthor 生成候选。generation/repair 都优先取得当前 snapshot 的 RSS 顶层和多结果声明，再附加函数章节；多结果必须使用方括号解构。retry/rerun 只跟随最多 64 层显式 replacement 血缘，拒绝同 alias 冒充、循环和超深链；source-less 输入复用冻结 artifact，旧 snapshot 回退到合并 overrides。候选先经过冻结函数、alias 顺序、精确结果名与多结果解构结构合同检查，再通过统一 adapter 调用 tensoreval；与父候选 hash 相同的 repair 仍记录 attempt 和预算，但不重复执行。非零退出或可修正的结果限制/合同错误最多修正 3 次；超时、监控、路径、配额和基础设施错误立即失败。成功仅在全部结果名称、列顺序、逻辑 dtype、实际非空值和可用 unit 元数据满足 Snappy Parquet 结果合同时原子发布。
- 模型表读取：active、准确 draft revision 和历史版本均支持元数据、分页投影与 Parquet 下载；Agent 可通过
  有界等值查询读取实际行后再修改，读取不改变模型生命周期。
- 用户配置图表：固定 API 从仿真/分析 DataFrame 发现标量及最多四层数值 List 分量，服务端校验六类受控配置并编译
  `chart-model-v1`；前端只做确定性 ECharts 映射。预览不写入，保存冻结配置、诊断、来源快照与 Snappy Parquet Plot Data；
  来源 tombstone 后仍可从会话图表页恢复。对话 Agent 没有图表工具，也不产生 ECharts JSON。

## Backend dependency baseline

2026-10-24 完成后端直接依赖刷新：`argon2 0.6.0`、`sha2 0.11.1`、`tokio 1.54.1` 和
`zip 9.1.0`；新增直接使用的 `getrandom 0.4.3`、`hex 0.4.3`，移除未直接使用的 `reqwest`、
`schemars` 与 `rand_core`。ZIP 继续关闭默认 feature，仅启用 `deflate`；Arrow 与 Parquet 保持
`60.0.0` 对齐。升级只包含必要的 API 兼容修改，不改变 REST/SSE、数据库、Agent 工具或引擎边界。

## Maintainability baseline

后端以 `cargo fmt` 和全 target、全 feature 的零警告 Clippy 作为提交门槛，前端以全量 Vitest 和生产构建作为提交门槛。
2026-11-04 的第一阶段全仓维护性整理已经完成，公共实现收敛以下重复逻辑：

- Parquet artifact 元数据由 `ArtifactMetadata::parquet()` 统一创建；
- staged artifact 到 `ArtifactDraft` 由 `ArtifactMetadata::into_draft()` 统一构造；
- `artifact.rs` 保留唯一 CAS commit/配额/link 创建内核，上传原文件提交不再复制该算法；
- `artifact_refs.rs` 统一用户 scope 解析与已有 link 的 retain/release，版本、资源、运行和工作流
  repository 不再直接更新 `ref_count`；
- 仿真、分析与比较事件中的数据集引用由 `ResultOutputRef::from_dataset()` 统一映射；
- 仿真和分析仓储共用内部 `StateTransition` 表达 running/failed 状态字段；
- `AppState` 和 `RunService` 使用具名依赖结构装配，删除按参数排列的构造函数链；
- API 公开错误响应由 `api/error.rs` 统一构造，同时保留各领域错误类型和稳定 code；
- 后端测试应用、临时引擎与 HTTP 请求夹具集中到 `backend/src/testing/` 和 `backend/tests/common/`；
- 前端请求错误解析、下载和取消信号由 `api/client.ts` 统一处理；
- 前端重复 DTO builder 集中到测试专用 `src/test/fixtures.ts`，每次构造的自有默认值相互隔离，传入 override 仍由调用方持有；
- 历史搜索的 SQL、结果映射和索引重建步骤集中定义，仓储异步边界保持可读。
- 第二阶段把 tensoreval 进程职责拆为 `execution`、共享 `result_scan`、`monitor`、`import` 和 RAII `task_file`；退出后只做
  一次独立 final scan，monitor 不再维护另一套扫描规则，取消 import/projector future 会清理临时 RSS/wrapper；
- 引擎资源职责拆为安全树 `tree`、snapshot `loader` 和不可变 `publisher`；发布器独占 inventory/hash/staging/rename/rebase，
  loader 不再消费虚假的 binary feature 集合，启动仍保持 capability probe → load → publish 的独立失败关闭顺序；
- 删除 `STATIC_MODEL_TABLES` V3 过渡常量和 `EngineExecution` 类型别名，模型 20 表与含 `para` 的 21 个 import 输出分别使用
  `MODEL_TABLE_OUTPUTS`、`STATIC_IMPORT_OUTPUTS`，运行结果结构直接命名为 `EngineRunOutput`。
- 上传确认由 `UploadConfirmationService` 单一编排，`UploadConfirmationRepo` 单一拥有 claim/执行锁、子发布、案例切换和
  batch 收尾；普通上传与案例整体切换只在 `PublicationStrategy` 选择一次，前者保持模型/资源双子事务，后者保持单事务切换。
- 上传生命周期与格式处理已拆分，`UploadProcessor` 独占 DSS/案例/归档/CSV/Excel/Parquet 处理；alias、opaque token 和候选状态
  分别由 `resource_alias.rs`、`opaque_token.rs`、`upload_candidate_policy.rs` 提供唯一应用层策略。
- 数据资源 facade 继续保持不变，但 structured replacement、deletion 和 Struct Parquet codec 已拆为内部子模块；repository 仍只有
  一个 `DataResourceRepo`，替换和删除事务分文件实现。版本发布/激活保持在 `VersionRepo` 主体，版本删除事务独立在
  `version_deletion.rs`，没有与上传模型候选应用或发布 receipt 合并。

领域错误类型仍保留在 adapter、repository 和各 application service 内，避免把不同边界的稳定错误码合并成
一个含糊的全局错误。旧的 application 构造变体、测试 `spawn_with_*` 分支、前端手写下载分支和重复本地 DTO builder
已经移除。上述两阶段整理不改变 REST/SSE、数据库 schema、Agent 工具或引擎合同。
本轮上传/模型版本/数据资源所有权整理同样不改变 HTTP 路由、JSON DTO、稳定错误码、数据库 schema 或前端交互；
`frontend-design.md` 和功能目录因此无需行为性修改。

2026-11-05 的研究配置、受管工作流与普通运行所有权整理已经完成：

- `StudyConfigurationService` 保持 facade，准备、输入编辑、确认快照与纯策略分别由
  `study_configuration_prepare.rs`、`study_configuration_inputs.rs`、`study_confirmation.rs` 和
  `study_configuration_policy.rs` 拥有；
- `WorkflowInputRepo` 是 retry/rerun 解析冻结 Struct 输入、显式 replacement 血缘和 legacy overrides 的唯一仓储边界，
  `StructOverlayService` 只 staging 私有 Parquet，artifact/link 与 workflow/run 输入在原事务内一并提交；
- `ManagedWorkflowRepo` facade 下的确认、retry、attempt/终态和恢复事务分别位于独立子模块，repository 只接收
  `ManagedWorkflowStartCommand`，不依赖 application DTO；
- `execution_input_tree.rs`、`execution_results.rs` 和 `execution_failure.rs` 只共享安全输入树、物理 Parquet 事实和有界脱敏诊断；
  `WorkflowWorker` 仍独占 author/repair、修正预算、managed strict contract 与终态发布；
- `RunService` facade 下的单次执行、open/frozen 结果策略与发布、删除编排分别由 `run_execution.rs`、
  `run_results.rs` 和 `run_deletion.rs` 拥有，普通运行没有 repair 路径；run 删除 token/影响/墓碑事务也已从主仓储文件拆出。

该整理不改变 REST/SSE、JSON DTO、稳定错误码、数据库 schema、Agent bundle/权限或前端流程。功能目录只把两条已合并的
行为测试引用更新为当前测试符号，没有新增或删除公共合同。

2026-11-05 的结果读取、分析、比较与图表维护阶段（8.6）已经完成；设计/计划提交为 `09a0b56`、`a2688c6`，
实施提交序列为 `3020383`、`10ac54b`、`992da49`、`8d59375`、`7d47ddb`、`8bfd27b`、`055e874`：

- `ResultSourceService` 成为 live dataset 用户作用域、会话、schema、artifact 与内容 hash 的唯一应用层来源事实，
  dataset page/download、分析输入和图表读取复用它；`ParquetDataFrameReader` 继续独占物理 Parquet 读取语义；
- 分析的单次输入解析/冻结/staging 与输出验证/提交/失败清理由 `analysis/inputs.rs`、`outputs.rs` 分离，
  比较的纯兼容性策略与受控 RSS 生成由 `comparisons/compatibility.rs`、`script.rs` 分离；两个公开 service facade、
  状态机、事务和错误码未改变；
- `ChartCompiler::compile` 保持入口不变，公共候选/预算/schema 与 line、bar、scatter、heatmap、boxplot、histogram
  六类算法分模块；图表 service 的来源解析、Plot Data 物化和冻结展示/cursor 也分离，已保存图表仍不依赖 live 来源；
- 前端 `ChartBuilder` 收敛为组合层，纯字段映射、六类 config seed/build、capability/preview/save Hook、模板控件和显示控件
  分离；浏览器仍只产生受控 `ChartConfig`，`chart-model-v1 → EChartsOption` 仍只在本地确定性 adapters 中完成。

机械审计和合同 diff 确认本阶段没有新增/修改公共路由、DTO、消息、稳定错误码、migration、图表合同、功能目录或
Agent bundle/权限；没有 adapter 外进程启动，也没有把任意 ECharts option 或物理路径带入公共边界。

2026-11-05 的 Agent、工具、Prompt 与知识资源维护阶段（8.7）已经完成；设计/计划提交为 `ef1dbd2`，实施提交为
`d7d706d`、`e0e34d9`：

- V4 manifest `allowedTools` 现在实际决定每次对话的工具暴露集合与顺序；Rust 私有 catalog 只映射静态实现构造器，
  启动和装配均双向拒绝缺实现、额外实现与重复实现；
- `AgentKnowledge` 在 hash、bundle/engine contract 和 skill requiredTools 之外，继续拒绝空白、空集合和重复工具名；
  Prompt skill index 与 contract manifest 都读取同一个已校验 knowledge 对象；
- 生产 main 预检得到的 knowledge 直接注入 `AppState`，不再在 application 层硬编码重载 bundle；源码删除未注册的
  `create_script` 实现、对应 service 依赖，以及直接拼接未校验 system/skill-index 的旧 Agent 构造入口；
- 公开工具仍为 23 个、skills 仍为 5 个，bundle 保持 `v4.4.0`；HTTP/SSE、数据库、稳定错误码、运行 provenance、
  ScriptAuthor/FailureAnalyzer 隔离和功能目录均未改变。

2026-11-05 的 API、前端状态与界面整合维护阶段（8.8）已经完成；设计/计划提交为 `4c5edc4`，实施提交为
`01058ec`、`fe7b373`：

- dataset、data resource 与 model DataFrame 下载端点通过 API 内部 attachment helper 统一文件读取和响应 header，
  各 handler 仍按用户作用域解析稳定资源 ID 并决定领域文件，路由、媒体类型、文件名和错误码未改变；
- 前端用小型 request-generation gate 统一一次性读取的取消、代次与卸载失效，修复 comparison 切换后的迟到覆盖并提供独立
  loading/error/retry；数据资源工作台、图表读取和 capability 读取同样拒绝旧响应；
- 资源和已保存图表的 cursor 结果只复用按稳定 ID 覆盖且保持首次顺序的纯合并函数，分页、轮询和 mutation 继续由领域 hook
  持有；显式资源刷新会使同会话旧请求与旧分页失效；
- 会话、运行、数据资源和模型版本删除共享可访问的 impact dialog 外壳，领域 preview/token/副作用保持分离；旧 token 被拒绝后
  重新预览并要求再次确认，模型版本删除不再使用 `window.confirm`；历史版本激活放弃草稿仍保留独立明确确认；
- 删除无消费者的 `useAnalysisRun` 和只转发 `useChartView` 的 `useDatasetView`。本阶段没有改变公共路由、DTO、消息、稳定错误码、
  migration、功能目录、Agent 权限、ChartModel/ECharts 或 Parquet 合同。

2026-11-05 的全局验证、部署与文档收敛阶段（8.9）已经完成；设计/计划提交为 `2ffa23d`，实施提交为 `b5779fe`：

- release semantic assertions 不再维护完整工具 allowlist；真实发布和手工全案例 journey 从当前 Agent manifest 读取并传入
  `allowedTools`，段落级工具子集、skill 门禁、稳定 ID 与副作用次数规则保持独立；
- `scripts/test-release-integration` 删除过期的 `v4.2.0` 报告断言，直接比较报告版本与当前 manifest；release config 测试也
  比较 `.env.example` 和 manifest，不再复制工具数量或 bundle 版本；
- 部署与集成测试文档把 144/144、profile 和代表运行数明确标为 2026-10-30 的历史候选证据；新候选必须从本次动态
  snapshot 重新产生零失败报告，不把旧数量当作接收门槛。

## Hard boundaries

所有 tensoreval 调用必须经过 adapter；所有 repository 查询带 `UserId`；所有模型和结果正文使用 Snappy Parquet；浏览器与
LLM 不接触物理路径。脚本创建不运行，模型修改、案例应用和研究请求不隐式发布；只有用户本轮明确要求发布时，Agent 才能以
刚检查到的精确草稿 ID/revision 生成并自动激活版本。单条聊天消息不可删除，只能通过影响预览和短期
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
manifest 只有逐字段匹配启用的 snapshot projection profile 才可导入；2026-11-06 已接受的候选快照中 150 个案例全部满足。候选发布门禁要求
不可导入案例为零，对全部 150 个案例逐一真实运行 projector 并验证目录声明的 Model/Data/Struct/diagnostic 输出；
该已接受证据为 150/150 成功、0 失败。之后仅对每个被案例引用的 projection profile 选择一个成功代表，运行只接受
DataFrame 命名输入的 `managed.rss`，当前为 6 个代表。当前 profile 和案例分布见 `docs/v4-deployment.md`。

## Reset and operation

`init_users.csv` 是编译输入，密码初始等于 ID。CSV 字节变化或从 v4.4.0 之前的开发 schema 切换后启动会返回或要求
空库重建；必须同时重置 SQLite 与 `DATA_DIR`，不能复用其中任一侧。公网可访问配置见
`docs/v4-deployment.md`，当前用户密码策略是已接受的试用风险。

## Verification

后端全量测试覆盖 schema、adapter、导入、版本、模型表读取、脚本、运行、删除、派生、比较、认证、恢复和配额。前端
Vitest 覆盖公共类型、state、workspace、模型表、脚本卡、运行与数据工作台。Playwright 覆盖真实浏览器到
Vite/Axum/SQLite/CAS 的身份、导入、分层案例目录与窄屏详情、显式运行、可展开运行详情与多结果选择、派生血缘、删除、恢复、恶意 ZIP、配额和无障碍。
最终命令与真实 LLM provider/tensoreval 候选门禁见部署文档。

研究配置合同由 `backend/tests/study_configurations.rs` 覆盖：显式 Newton、目录动态模型输入、保护模型范围、独立
`para`、多资源不猜选、Struct 私有用户/selector 覆盖、source-less 默认值、revision/用户隔离、无提前 confirm 路由及受控列映射发布。

当前代码确定性基线（不含下述真实相邻 eig-rs 资源树断言）通过 `cargo fmt --all -- --check`、
`cargo clippy --all-targets --all-features -- -D warnings`、后端全量测试、54 个文件/285 条 Vitest、生产构建、31 条 Playwright 旅程及
350/350 功能证据、297/297 公共合同、100% PR 功能追溯；候选门禁另以真实
所选真实 LLM provider、当前 eig-rs/tensoreval、动态门面与案例快照完成规划、执行、分析、比较和删除全旅程。
候选门禁中的 tensoreval 固定以 `--no-default-features --features tensoreval-cli,with_dsa_mumps` 构建，不使用
eig-rs 默认 features。

2026-11-03 的运行私有 Struct 参数变更已通过后端聚焦测试（study configuration、managed workflow、retry/rerun、run lifecycle）
和前端 `StudyConfigurationCard` 20/20 聚焦用例。因同一工作区存在并行修改，本轮全量 Rust/Vitest/build/Playwright/coverage
验证按用户要求暂缓，以上“当前确定性基线”不作为这批未完成全量验证的新增证据。

2026-11-04 的用户配置图表工作台已通过 `cargo fmt`、全 feature 零警告 Clippy、49 文件/258 条 Vitest、生产构建、
31 条 Playwright 旅程和 350/350 PR 功能覆盖。后端全量测试中的代码与临时 e2e-server 测试通过；真实相邻 eig-rs
资源树门禁当前单独报告 7 个 `case_projection_invalid` 案例（四个 DistanceRelays、GFL IEEE123、InductionMachine 和
59NRelayDemo），因此在外部资源合同修复并重跑前，不能把本次变更声明为候选发布全绿，也不能通过修改 zjudss 放宽该门禁。

2026-11-04 的维护性第一阶段在当前代码上重新通过 50 文件/265 条 Vitest、生产构建、31/31 Playwright、后端格式与
全 feature 零警告 Clippy；跳过且仅跳过上述一个真实资源树验收断言后，后端其余完整 `e2e-test` 套件通过。精确的
`cargo test --features e2e-test` 仍只在
`current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count` 报告以下 7 个外部
`case_projection_invalid`，所以候选发布状态没有被误标为全绿：

2026-11-04 的维护性第二阶段完成 adapter process 与 engine resource tree/loader/publisher 边界拆分后，再次通过后端格式、
全 target/all feature 零警告 Clippy、跳过且仅跳过同一外部资源断言的完整 `e2e-test`、50 文件/265 条 Vitest、生产构建、
31/31 Playwright，以及 350/350 功能证据、297/297 公共合同和 100% PR 追溯。未跳过的后端命令仍只报告下列同一 7 个
相邻资源树 `case_projection_invalid`，没有新增代码失败，也未放宽资源合同。

- `Examples/DistanceRelays/DistanceRelayTest.rssbundle`
- `Examples/DistanceRelays/ReverseDistanceRelayTest.rssbundle`
- `Examples/DistanceRelays/ReverseTD21RelayTest.rssbundle`
- `Examples/DistanceRelays/TD21RelayTest.rssbundle`
- `Examples/IBRDynamics_Cases/GFL_IEEE123/Run_IEEE123Bus_GFLDaily.rssbundle`
- `Examples/InductionMachine/Run.rssbundle`
- `Examples/Scripts/59NRelayDemo.rssbundle`

2026-11-04 的 Artifact/数据库仓储维护阶段已将 CAS commit 收敛为单一内核，并将已有
scope link 的 retain/release、重复逻辑引用聚合、checked arithmetic 和可信用户 scope 解析集中到
`artifact_refs.rs`。上传原文件幂等重放、模型 clone、数据资源转移、普通运行和受管工作流的事务/
tombstone 边界保持不变。本阶段通过后端格式与全 feature 零警告 Clippy、50/50 个文件共
265/265 条 Vitest、生产构建、31/31 Playwright、350/350 功能证据、297/297 公共合同与 100% PR
追溯。精确后端全测仍只报告上述同一个真实 eig-rs 断言和同一 7 个案例；仅跳过该断言后，
其余完整 `e2e-test` 套件通过，因此候选发布状态仍未被标记为全绿。

独立的 `./scripts/test-manual-case-journeys` 可由测试人员显式选择运行当前 snapshot 的全部可导入案例，或以
`CASE_JOURNEY_EXAMPLE_ID`、`CASE_JOURNEY_FACADE`、`CASE_JOURNEY_RESUME_REPORT` 聚焦/续跑。它逐案例验证导入确认、
自然语言发布、手册原文提示词、研究确认、脚本/修正、运行和结果，并增量写入 `target/manual-case-journeys/report.json`；
该长测不属于 PR、日常回归或候选发布门禁。已接受快照的 150/150 只证明 projector 发布门禁，只有一次显式完整长测的报告全部通过，
才能声明 150 个案例的自然语言全链路通过。

案例库确认已切换为当前会话案例整体替换：单事务发布/激活模型版本与配套资源，普通上传仍保持双子事务；确认卡展示影响并保护准备后的并发变更。旧协议的部分发布案例批次需重新准备，不自动补齐为案例切换。验证以本次 case_import、data_resources、workspace_versions 和浏览器案例旅程为准。

2026-11-04 的上传、模型版本与数据资源所有权维护阶段已通过后端格式、全 target/all feature 零警告 Clippy，以及
`upload_batches`、`case_import`、`model_imports`、`data_resources`、`study_configurations`、`managed_workflows`、
`version_deletion`、`model_versions` 和 `workspace_versions` 定向行为测试。新增回归锁定重新解析失败后的 artifact 清理、
不受支持解析设置不破坏旧候选、structured replacement 源不可读时不提交新 artifact、版本 busy/无效或过期 token，
以及版本删除与 artifact 引用释放的事务原子性。最终全仓验证已执行：前端 50 个文件 265 项测试与生产构建通过，
功能覆盖 350/350、公共契约 297/297；后端跳过已知外部资源断言后的 `e2e-test` 全套通过。未跳过运行仍只受上文
同一真实 eig-rs 资源树断言及 7 个外部 `case_projection_invalid` 阻塞，本阶段未放宽该合同，也不将候选发布标记为全绿。

2026-11-05 的研究/workflow/run 所有权整理以 `67e2d34..317bf6b` 八个独立提交完成。机械审计确认旧
`study_struct_overlays`/`materialize_*` 路径、repository 对 `PreparedStudyConfirmation` 的依赖、retry 私有随机/hash 实现、
retry/rerun 对底层 lineage helper 的直接调用，以及 adapter 外的进程启动均不存在。后端格式和全 target/all feature
零警告 Clippy 通过；精确跳过唯一外部资源断言后完整 `e2e-test` 通过；前端 50/50 文件、265/265 Vitest、生产构建、
31/31 Playwright、350/350 功能证据、297/297 公共合同和 100% PR 追溯通过。未过滤后端测试仍只报告上文同一断言和
同一 7 个 `case_projection_invalid`，因此候选发布状态仍未标记为全绿。

2026-11-05 的 8.6 阶段在提交 `055e874` 后完成最终验证：后端 `cargo fmt --all -- --check` 与全 target/all feature
零警告 Clippy 通过；未过滤 `cargo test --features e2e-test` 只在
`current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count` 报告上文完全相同的 7 个
`case_projection_invalid`，其余 engine resource 测试为 32/32；跳过且仅跳过该精确外部断言后，完整 `e2e-test`
套件通过。前端 52/52 文件、275/275 Vitest 和生产构建通过；PR 功能覆盖为 350/350、公共合同 297/297、30/30
目录条目，追溯率 100%。本机没有配置候选发布要求的 `TENSOREVAL_BIN`、`SPARROWZZ_DIR`、真实 LLM secret、
`INIT_USERS_CSV` 或 `target/eig-rs`，因此没有把未运行的真实 binary 构建和 LLM 发布全旅程记为通过；相邻资源树七案例
状态仍是外部兼容性阻塞，候选发布仍未标记为全绿。

2026-11-05 的 8.7 阶段通过后端格式、全 target/all feature 零警告 Clippy，以及 Agent knowledge、factory、tool
manifest、contract manifest 聚焦测试。完整 `cargo test --features e2e-test` 仍只在同一
`current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count` 外部资源断言报告相同 7 个
`case_projection_invalid`；精确跳过且仅跳过该断言后，其余完整 Rust 套件通过。8.7 没有修改前端或公共合同，前端与
功能覆盖将在 8.8/8.9 全局门禁统一复验。

2026-11-05 的 8.8 阶段通过 attachment 三类端点的后端聚焦测试、54/54 文件共 284/284 项前端 Vitest 和生产构建。
全量 Vitest 首次在受限环境中仅因开发服务器无法监听 `127.0.0.1` 失败；允许回环监听后原命令全绿。Playwright、功能覆盖、
后端全量格式/Clippy/test 与发布配置将在 8.9 全局门禁统一复验。

2026-11-05 的 8.9 最终门禁通过后端格式和全 target/all feature 零警告 Clippy。受限环境首次让 `e2e_server` 的回环监听
返回 `Operation not permitted`；允许本机回环后，未过滤完整 Rust 套件只在
`current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count` 精确报告上文同样 7 个外部
`case_projection_invalid`，`engine_resources` 其余 32/32 通过；跳过且仅跳过该断言后其余完整 `e2e-test` 通过。
前端 54/54 文件、285/285 Vitest、生产构建和 31/31 Playwright 通过；PR 功能门禁为 350/350 功能证据、
297/297 公共合同、30/30 目录条目和 100% 追溯。本机未配置 `TENSOREVAL_BIN`、`SPARROWZZ_DIR`、真实 LLM secret、
`INIT_USERS_CSV` 或 `target/eig-rs`，因此真实候选发布 lane 未执行；当时候选发布仍受相邻资源树七案例阻塞，未标记为全绿。

2026-11-06 eig-rs `4f191f05c` 与 sparrowzz `86c5b92` 完成 Dynamic V13 Monitor 合同后，zjudss 将
`resultSelector.valueSource` 切换为 `profile.resultMap(manifest.projection.results)`，并把受管结果校验为原生
`study.results` 的非空、唯一、有序子集，同时继续精确校验 profile 映射、target 和 binding。新增行为测试证明
16 个原生结果/15 个受管结果可导入，而未知、重复、重排和错误映射仍失败关闭；release wrapper 也会在 Cargo 启动前
删除旧报告，失败候选不再复用历史证据。

本轮通过后端格式、全 target/all feature 零警告 Clippy，以及允许本机回环后的未过滤完整
`cargo test --features e2e-test`；`engine_resources` 为 35/35，`process_engine_v4` 普通套件为 18/18。
真实 `scripts/test-release-engine` 使用 binary SHA-256
`c7dce676f954d090780a6f9ff7b4f81382e4d16e047d8292169c74cb0742a369` 和 snapshot
`ers_7d30a9cba13a8bf2ba4741e4fa2010df`，通过 19/19 ProcessEngine、150/150 案例 projector、0 失败，
并通过 6/6 被引用 profile 的代表运行。先前 7 个 `case_projection_invalid` 外部阻塞已经关闭；这不等同于尚未运行的
150 案例自然语言全链路长测，也不替代下一次 binary/snapshot 的新报告。
