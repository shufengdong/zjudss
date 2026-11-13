# V4 集成测试与功能追溯

## Test lanes

- Rust：数据库约束、repository scope、adapter 命令/输出、application 事务、Axum 状态码和恢复。
- Vitest：公共 DTO、reducer/hook、workspace、脚本卡、运行记录、数据表与图表。
- Playwright：浏览器 → Vite proxy → Axum → SQLite/CAS 的确定性全链路，使用 Scripted LLM 与
  ScenarioEngine，但不绕过 HTTP 或 UI。
- Release：检出并构建 eig-rs 当前默认分支的真实 tensoreval，并按 `LLM_PROVIDER` 使用 DeepSeek 或百炼的真实
  secret/endpoint；缺少所选 provider 的配置、资源或当前接口不兼容时失败，不跳过，也不回退到旧 artifact。
- Manual case journey：仅由用户显式执行的真实 LLM/tensoreval 长测，逐个覆盖当前 snapshot 的可导入案例；不属于
  PR、普通 E2E 或 Release lane，也不作为自动功能追溯的替代证据。

## Test infrastructure ownership

- `backend/src/testing/app.rs` 统一把 minimal/runtime profile、scheduler、engine snapshot 与依赖 override 映射为
  `AppStateOptions`；`backend/src/testing/engine_fixture.rs` 负责可控引擎资源。两者只在 `e2e-test` feature 下编译，
  生产组合根仍由 `AppState::assemble` 负责。
- `backend/tests/common/app.rs` 的 builder 只管理临时目录、数据库迁移、用户 seed 和测试选择，再调用上述共享状态装配；
  `backend/tests/common/` 其余代码按 fixtures 和 HTTP 边界提供共享能力。新增场景组合 builder 选项，不增加
  `spawn_with_*` 或另一套 `AppStateOptions` 构造路径；领域专用数据仍留在对应测试文件。
- `frontend/src/test/fixtures.ts` 只保存重复公共 DTO builder。builder 自有默认值必须为每次调用创建深度隔离的对象，
  只提供顶层覆盖；override 的所有权仍属于调用方，组件交互和特定领域状态仍在测试内显式表达。
- Playwright 只经真实 UI、Vite proxy 和 Axum 路由工作；ScenarioEngine 控制确定性外部行为，但不能替代 HTTP、持久化、
  浏览器交互或公共合同证据。

当前确定性基线为 54 个 Vitest 文件/292 条用例、生产构建和 32 条 Playwright 旅程；Rust 全量套件的真实 eig-rs
验收断言若失败，必须单独报告实际无效案例，不能通过共享夹具自动跳过或改写资源合同。

研究/workflow/run 维护性回归以公共 service/API 和真实 SQLite transaction 为证据：`managed_workflows.rs` 与
`workflow_retries.rs` 覆盖 overlay 第 N 项失败清理、幂等/并发 replay 不消费 staging、source-less/unchanged/replacement/
legacy/same-alias/cycle/超深 lineage 的 retry/rerun 同义矩阵；`script_repair.rs` 用 barrier 覆盖 generation、engine、repair
三处取消和最多 3 次修正/4 次执行；`simulation_runs.rs` 覆盖 manual open contract、Agent frozen contract 终态失败且不 repair、
失败诊断 enrichment 与用户作用域；`result_deletion.rs` 覆盖脚本、输入和全部直接结果引用在墓碑事务中原子释放。
共享执行单元测试只证明输入树、物理 Snappy Parquet 读取和脱敏诊断，不能代替 managed/ordinary 各自的结果策略行为测试。

## Coverage matrix

功能目录 `integration-tests/catalog/features.yaml` 是机器可读追溯源。每项包含 source、layers、lanes、真实
测试符号、断言、fixture 和可选公共合同。每个 manifest route/message/tool/skill/upload/view/error 必须恰好出现
一次；注册表枚举测试不能单独证明行为。PR 路由合同还必须由关联 Playwright 用例产生 2xx/3xx 请求证据。

覆盖范围包括 BOOT、AUTH、SESSION、MESSAGE、SSE、SCHED、UPLOAD、MODEL-IMPORT、WORKSPACE、MODEL-DATAFRAME-READ、VERSION、
EXAMPLE、DATA-RESOURCE、AGENT、SCRIPT、RUN、DATASET、ANALYSIS、VIEW、COMPARE、HISTORY、STORAGE、UI、RECOVERY、ENGINE。

SESSION/MESSAGE/SSE/HISTORY 的 V4 行为测试还覆盖：删除 preview 的完整影响统计、过期或变化 token、删除中写入阻断和最小
tombstone；三种 durable 操作卡以稳定 operation ID 原位更新、终态幂等、revision SSE 回放和公共 payload 脱敏；`aroundId`
严格归属与精确包含；前端 anchored window 与 latest cache 分离、实时新消息提示及返回最新窗口。

DATA-RESOURCE-001 的 Rust 行为测试覆盖：无 schemaRef 的通用资源、静态 `para` 与模型草稿分离且不能改判为模型、
未决用途禁止确认、双子事务部分成功后以新操作重试失败侧、成功侧空间结算与决定冻结、同操作同 payload 幂等、同操作不同 payload 冲突及两个子 receipt、替换不改写旧
artifact 并使未确认研究配置失效、Struct preview/confirm 与被取代 artifact 回收、影响 token 删除和配额释放、跨用户 404、
受管研究覆盖确认幂等、同会话单非终态 workflow、Struct 替换与冻结同事务、provisional run 空脚本合同、candidate 父链、
终态脚本消息锚点、带候选和无候选两类取消/启动恢复、active 模型切换竞争、并发重放 material 引用结算；
Task 10 另以 `backend/tests/script_repair.rs` 覆盖首次生成后立即执行、一次成功、生成和 repair 都收到 RSS 顶层/多结果必需章节、
非零退出修正成功及结构化失败分析、分析模型不可用时仍保留日志、相同脚本 hash 保留审计但不重复调用引擎、初始/repair 结构合同违规的
引擎调用和预算差异、3 次实质变化的修正对应最多 4 次执行、候选配额失败终态化、结果合同拒绝后无部分发布、
64 KiB 脱敏 prompt、审计 stderr 隔离，以及生成阶段取消；前端组件测试覆盖失败诊断直接展示、四段分析、日志下载入口和
默认折叠的自动修正记录。`script_contract.rs` 还直接验证两个及以上结果只有方括号解构才能通过 adapter 前的结构合同。
`artifact_store.rs` 还验证没有数据库记录的崩溃遗留 CAS
对象可由 GC 回收。这些是行为测试，不能由状态枚举注册测试替代。

Artifact 引用边界另有一组事务证据：`artifact_refs` 单元测试覆盖重复 ID 按逻辑次数
retain/release 但不重复计费、批量中间缺失 link 的整体回滚、非法/溢出/下溢计数失败关闭和
用户 scope 404；`artifact_store.rs` 与 `quota_concurrency.rs` 覆盖同 hash 并发提交、配额不超卖、
跨 scope 独立计费、开启中读取与 GC。`data_resources.rs` 覆盖活动模型中重复 artifact 的逐表
clone/release、上传 source 引用转移和提升；`managed_workflows.rs` 覆盖确认时 scope link 损坏返回
`ConfigurationStale`，且 configuration/workflow/run/input snapshot 均不留部分写入。上传 source
指针幂等重放、删除 tombstone 和 GC 继续由各领域集成测试验证，不用仓储内部结构断言替代。
Task 11 由 `simulation_runs.rs`、`workflow_retries.rs` 和 `result_deletion.rs` 覆盖资源型手写脚本无 active 执行、
规范 script message 锚点、按门面读取最新所需输入、开放/冻结结果合同、失败脚本在输入不变时直接派生以及输入变化时
差异确认、完整新修正预算、派生脚本血缘、终态删除 preview/token、`run_busy`/`result_busy`、全部直接结果原子 tombstone、
artifact 配额释放、重复删除和派生/提升血缘保留；同时覆盖普通失败 run 先落脱敏日志/终态进度、后完成结构化分析，以及分析器错误、
失败分析器自身的 30 秒超时和启动恢复的 unavailable 降级。Vitest 覆盖共享面板、旧 `engineOutput`、pending-only 轮询、请求代次隔离与读取重试；Playwright 在不打开运行记录时验证
运行进度卡的多行脱敏 tensoreval 日志，并验证刷新后恢复。
schema 资源的 SaveAsNew alias 分配、过期/跨路径失效 preview 回收、同用户跨会话去重原文件的引用隔离、普通资源
分析输入/冻结血缘、并发 confirm 原文件幂等提交、同一确认声明只能取得一次执行锁、精确重放跨暂存 TTL 仍只读返回、显式 alias 冲突重放保持冻结失败语义、活动确认期间候选冻结与 DELETE 冲突、到期避让以及重启先汇总
已提交子事务、confirm/discard 与 parse/discard 竞争不复活终态、模型替换
原子释放旧草稿引用、活动版本克隆按逻辑表计数去重 artifact 引用、重复候选逐引用结算、结果提升，以及 4 个资源 Agent
工具的真实调用行为。合同测试同时追踪 10 条资源路由、4 个资源 Agent 工具和
稳定错误码；公共 API 的 Playwright 旅程覆盖发布、读取、改名、替换、Struct 覆盖和删除，Task 12 再补真实界面的
数据文件工作区旅程。

## Deterministic journeys

浏览器套件验证登录页 SGOOL 标识与产品标题的同行品牌区，以及固定用户登录/退出/重置；空账户自动建立会话后的发送、上传和案例应用，以及初始化失败重试和删除最后会话后的替代会话；会话 CRUD、分页和 SSE；
DSS/bundle/ZIP/表格统一上传、解析设置、逐候选用途确认、TTL/配额恢复；案例确认；draft
发布、active race、历史激活和版本删除；active/draft/历史模型表的只读元数据、分页投影和 Parquet 下载；草稿
revision 冲突恢复；Agent 对模型行的严格类型等值读取、完整匹配计数和 read-before-modify；脚本预览/全文/复制与 Agent 只创建；按钮运行、多结果与单结果删除；
运行记录默认收起摘要、成功后懒加载全部结果并可选择非首个结果、失败原因/阶段/耗时、加载重试和删除后结果刷新；
运行成功进度终态、Arrow 字符串视图分页和不同宽度数据集详情居中；分析、保存图表、比较在来源删除后继续读取；
两用户隔离（包含六个模型表读取端点）；重启收敛与 FTS 局部失败；配额、恶意归档和模型表窄屏响应式无障碍。

图表专项旅程使用 ScenarioEngine 产生 `Utf8View` 分类轴和三分量 `LargeList<Float64>`，从非首个结果打开真实工作台，
选择 A/B/C 三个分量并自定义系列名，调整图例、方向、调色板、线型和数据点，经过预览后保存；来源仍存活时再复制保存。
随后刷新、删除来源结果，并从会话“图表”页重新发现和打开两个物化图表，验证来源删除提示、390px 无横向溢出和 axe
serious/critical 为零，最后分别删除图表并验证逻辑存储计数/字节下降。Alice 创建的图表由 Bob 对 capability、预览、保存、
详情、Plot Data、删除和会话列表七条路由逐一得到 404。该旅程不向 Agent 发送图表请求，也不注入旧 `initialViews`；
Playwright 必须走真实 UI 选择、预览、保存、刷新、复制和删除操作。

RUN-001 在 Rust 层验证创建、重放、列表和详情都返回绑定脚本的必填 `scriptName` 与精确可空 `scriptMessageId`，并验证
运行输入由后端冻结、失败脚本 retry 分支和终态运行删除；在 Vitest 层验证中文摘要、终态/动态耗时、
失败详情、运行列表与结果加载错误、按运行请求代次隔离和 `datasetRevision` 刷新，在 Playwright 层验证显式运行产生的两个结果
都出现在展开记录中，并能选择非首个 `loss` 打开工作台；真实失败场景验证实际阶段、退出码和多行 tensoreval 报错可见，绝对路径
被隐藏且不请求数据集。手写、粘贴或上传的脚本以及成功脚本的再次运行仍需显式点击“运行”；研究确认后的首次受管脚本
按合同自动执行。运行记录已接入精确消息定位、retry 差异确认和删除影响确认控件。

应用层 scripted-agent 测试额外覆盖模型生成说明及服务端权威页脚先于配置卡持久化、配置卡已创建后的 provider 收尾失败不覆盖成功、对话 Agent 无脚本创建权限；
并用“求解牛顿法潮流”等原始短指令验证显式函数选择不被拓扑改写，缺 Data/Struct、资源歧义和禁用门面都只返回
选择或配置卡，不提前生成或运行脚本。
当前结果分析回归先通过真实普通运行发布 Parquet dataset，再发起自然语言请求：唯一 `bus_voltage` 场景断言
`currentResultContext` 只提供有界稳定元数据、Agent 加载 `result-analysis`、实际调用 `run_result_analysis`，且数据库只冻结
选中的一个 dataset ID；两个名称/列均可能匹配的结果则只要求选择，不产生分析记录。另一个场景证明较新的 running、failed
和 succeeded-but-empty 运行不会遮蔽最近可用成功结果。上下文构造单元测试覆盖 64 KiB 总预算、32 列、256 字符与显式裁剪标志；
Agent bundle/tool 合同测试锁定 `result-analysis@1.2.1`、23 个工具、不变的显式多输入参数，以及 1 到 1000 字符的
审计 `intent` 字符串；对象、空白和超长 intent 在分析写入前失败关闭。
唯一结果 scripted 场景请求复制指定列，不声称模拟引擎验证了阈值筛选语义；真实依赖旅程通过 trace 与输出血缘确认恰好冻结当前候选运行的
`bus_voltage`，并逐页比较派生副本的全部行及 schema，删除同一来源后再验证 tombstone 血缘与派生数据可读。
补充 scripted 场景覆盖不同用户/会话的较新结果不可见、最近 20 条的精确边界、全部来源删除后的 `null`，以及损坏持久化 schema
触发 lookup error 后仍注入 `null` 并完成 Agent 回答。预算测试包含最终裁剪标志从 `true` 转为 `false` 的 65,536 字节边界。
研究结果选择同时覆盖两个入口：Vitest 验证 `ready_to_confirm` 状态下结果编辑器与最终确认按钮共存并使用当前 revision；
Rust 直接工具测试验证 skill 门禁、十进制字符串 ID、当前会话隔离、乐观锁、终态和非法集合；scripted-agent 与 Playwright
验证自然语言 `bus_voltage，summary` 原位更新 durable 卡，且脚本、workflow 和 run 仍为空。组件测试覆盖配置卡首次展开、按配置跨 revision 的浏览器会话记忆、mutation 错误自动展开和终态紧凑显示。受管研究浏览器旅程断言详细说明的 DOM 节点先于配置卡、provider 尾声不存在，并验证收起后隐藏输入/最终确认、刷新和切换会话后保持收起；还通过卡片
选择 `summary`、提交“确认结果”并检查 revision/摘要，再恢复原结果继续验证既有执行修正链路。
模型发布行为还覆盖明确发布请求使用精确 draft/revision 并自动激活、消息级幂等、内容去重、stale/跨用户拒绝，以及
普通修改、研究和“生成脚本/结果”文字不触发隐式发布。

ScenarioEngine 场景由控制端按调用顺序排队，不使用 sleep；长时间执行/cancel 使用 barrier，并能排队提供二进制 stderr、
结果合同错误和监控错误。进程 adapter 的 Rust 测试使用独立 helper process 验证双流超过 1 MiB 仍持续排空、完整 hash、
非 UTF-8、非零退出码、生产执行不因等待时长自行结束、future drop/用户取消终止进程组、运行中三类结果限额、符号链接/目录、监控失败和退出后最终扫描；所有失败
都断言未发布目录被清理。退出后的 final scan 只执行一次并与 monitor 共享扫描实现，同时仍独立复核退出瞬间的目录；
import/projector 的临时 wrapper 与 RSS 通过 future cancellation 测试证明会被清理。engine resource 测试精确断言
snapshot identity、catalog 版本、generated + manifest-declared source inventory、逐文件 hash、既有目录损坏拒绝和原子发布后的
path rebase；binary probe 先于且独立于 resource load。测试证据写入
`target/functional-coverage/e2e-routes.ndjson`，包含通过的测试名及实际 method/path/status。

普通运行专项测试覆盖排队取消、运行中进程组取消、重复取消幂等、完成/取消竞态只保留首个终态、跨用户 404，以及受管
run 返回 `managed_run_cancel_via_workflow` 并要求走 workflow 端点。Vitest 覆盖普通/受管端点分流、失败恢复按钮和
`cancelled_by_user` 的中性终态。测试中的 `tokio::time::timeout`、Vitest/Playwright timeout 只用于发现挂死，不传入生产
engine spec。导入、案例投影和派生分析不测试用户取消，因为产品没有这些入口；它们只验证完成、实际错误、结果安全拒绝和服务停止清理。

## Commands

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
```

需要人工选择全案例语义长测时，从仓库根显式运行 `./scripts/test-manual-case-journeys`。可设置
`CASE_JOURNEY_EXAMPLE_ID` 或 `CASE_JOURNEY_FACADE` 聚焦，或用 `CASE_JOURNEY_RESUME_REPORT` 从先前报告只选择失败和未运行
案例。套件使用独立临时 SQLite/`DATA_DIR`、单 worker 和多小时 timeout，逐案例原子写入脱敏的
`target/manual-case-journeys/report.json`；浏览器痕迹也只写入本次临时目录，结束后一并清理。续跑前校验 engine snapshot、
Agent bundle 和发现/选择的案例集合；输入与输出为同一路径时先保留原报告副本。失败提示词由人审阅修改映射与测试手册，再筛选重跑；
测试不会让 LLM 自动改写提示词。完整执行前不得根据 `--list`、聚焦案例或仅有 projector 报告声称全案例语义链路通过。

候选发布执行 `scripts/test-release-integration`。脚本缺省使用 DeepSeek，也可通过 `LLM_PROVIDER=bailian`、
`LLM_MODEL`、`BAILIAN_API_KEY` 和 `BAILIAN_BASE_URL` 切换到百炼；endpoint 与 API Key 必须同地域。报告记录测试 ID、Agent bundle hash、用户 seed hash/数量、engine
contract、当前 binary hash、动态资源 snapshot 与案例分类数量；hash 是证据而非固定接收值，日志必须脱敏。
Agent bundle 版本和完整工具权限只从 `backend/resources/agent/v4/manifest.json` 读取：真实与手工旅程把该次
`allowedTools` 传给语义轨迹断言，发布 shell 直接比较报告版本与同一 manifest，不维护第二份工具全集或版本常量。
真实门禁必须覆盖模型导入/发布、目录驱动的研究门面检索、配置卡准备与最终确认、Agent 创建受管研究脚本、自动执行、结果分析/比较和只使用稳定 ID。门禁扫描全部
manifest schema v2 案例，要求不可导入案例为零，并对每个案例真实执行其 projector，逐项验证有序 Snappy Parquet
输出及 Model/Data/Struct/diagnostic 分类。2026-10-30 历史候选证据为 144/144 成功；2026-11-06 当前候选以
snapshot `ers_7d30a9cba13a8bf2ba4741e4fa2010df` 重新证明 150/150 成功、0 失败。每次候选仍必须以本次
动态 snapshot 重新生成报告，并以 `projectedExamples`、
`projectionFailures` 和逐案例 `caseProjections` 固化该门禁，任一案例失败都使候选发布失败。
manifest `study.results` 是原生结果全集，`projection.results` 是受管结果的有序子集；selector 只映射后者，
但 unknown、duplicate、reorder、target 或 binding 漂移仍使案例不可导入。随后对每个被案例引用的 profile 选择一个
投影成功的代表，以其命名 Parquet 输入执行 `managed.rss`；当前证据为 6 个代表，
同时覆盖全部 study family。projector 输出缺失、重复、多余、顺序合同不一致或 manifest/profile 不一致均原子失败。
没有案例引用的 disabled profile 可以保留稳定诊断，但不能被 manifest 使用。历史证据分布见 `docs/v4-deployment.md`；
数量不是未来 snapshot 的固定接收条件。

## Failure policy

不允许 `skip`、`ignore` 或 manual-only 填补覆盖。失败先保留 trace/screenshot/log，按最窄层复现，再运行全量。
当前引擎不兼容不能通过放宽校验解决；应修复当前 eig-rs 或正式更新应用 contract 和全部证据。服务不会自动
改写引擎工作树；资源或 binary 更新后必须重启并重新生成运行时证据。

2026-11-05 全仓门禁再次确认：后端格式与全 target/all feature Clippy 通过，跳过且仅跳过
`current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count` 后完整 `e2e-test` 通过；未过滤运行仍只报告既有
四个 DistanceRelays、GFL IEEE123、InductionMachine 和 59NRelayDemo 共 7 个 `case_projection_invalid`。前端 50/50 个
Vitest 文件、265/265 用例、生产构建、31/31 Playwright，以及 350/350 功能证据、297/297 公共合同和 100% PR 追溯通过。

2026-11-06 eig-rs/sparrowzz 发布 Dynamic V13 完整合同后，zjudss 未过滤 `cargo test --features e2e-test`
在允许本机回环监听的环境中通过；真实 release-engine 同时通过 19/19 ProcessEngine 测试、150/150 案例 projector
和 6/6 profile 代表运行。上述 7 个 `case_projection_invalid` 已由资源合同修复并关闭，不再是当前阻塞。

2026-11-10 无固定引擎超时与用户取消阶段重新通过未过滤 Rust 全套、54/54 文件共 292/292 项 Vitest、生产构建和
32/32 Playwright；功能追溯为 351/351，公共合同为 298/298，30/30 条目均有效且 `gatePassed=true`。新增浏览器旅程从
运行记录取消一条真实运行中的普通仿真，并为 `POST /api/runs/{id}/cancel` 产生 200 路由证据。

案例库应用以整体切换验收：准备阶段不发布，确认后 active 为新建或复用的模型版本、draft 为空、上一案例配套资源退出 live 集合，历史资源引用保留。`case_import` 行为测试覆盖重复案例去重、资源切换、资源发布失败回滚、用户资源 alias 冲突保护和过期确认基线；浏览器案例旅程覆盖“确认切换案例”与自动激活。独立全案例人工旅程在确认后验证 active，不再额外请求 Agent 发布同一案例。
