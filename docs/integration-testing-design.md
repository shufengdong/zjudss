# V4 集成测试与功能追溯

## Test lanes

- Rust：数据库约束、repository scope、adapter 命令/输出、application 事务、Axum 状态码和恢复。
- Vitest：公共 DTO、reducer/hook、workspace、脚本卡、运行记录、数据表与图表。
- Playwright：浏览器 → Vite proxy → Axum → SQLite/CAS 的确定性全链路，使用 Scripted LLM 与
  ScenarioEngine，但不绕过 HTTP 或 UI。
- Release：检出并构建 eig-rs 当前默认分支的真实 tensoreval + DeepSeek secret；无 secret/资源或当前接口不兼容时
  失败，不跳过，也不回退到旧 artifact。

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
schema 资源的 SaveAsNew alias 分配、过期/跨路径失效 preview 回收、同用户跨会话去重原文件的引用隔离、普通资源
分析输入/冻结血缘、并发 confirm 原文件幂等提交、同一确认声明只能取得一次执行锁、精确重放跨暂存 TTL 仍只读返回、显式 alias 冲突重放保持冻结失败语义、活动确认期间候选冻结与 DELETE 冲突、到期避让以及重启先汇总
已提交子事务、confirm/discard 与 parse/discard 竞争不复活终态、模型替换
原子释放旧草稿引用、活动版本克隆按逻辑表计数去重 artifact 引用、重复候选逐引用结算、结果提升、普通资源
物化视图，以及 4 个 Agent
工具的真实调用行为。合同测试同时追踪 10 条资源路由、4 个 Agent 工具和
稳定错误码；公共 API 的 Playwright 旅程覆盖发布、读取、改名、替换、Struct 覆盖和删除，Task 12 再补真实界面的
数据文件工作区旅程。

## Deterministic journeys

浏览器套件验证登录页 SGOOL 标识与产品标题的同行品牌区，以及固定用户登录/退出/重置；空账户自动建立会话后的发送、上传和案例应用，以及初始化失败重试和删除最后会话后的替代会话；会话 CRUD、分页和 SSE；
DSS/bundle/ZIP/表格统一上传、解析设置、逐候选用途确认、TTL/配额恢复；案例确认；draft
发布、active race、历史激活和版本删除；active/draft/历史模型表的只读元数据、分页投影和 Parquet 下载；草稿
revision 冲突恢复；Agent 对模型行的严格类型等值读取、完整匹配计数和 read-before-modify；脚本预览/全文/复制与 Agent 只创建；按钮运行、多结果与单结果删除；
运行记录默认收起摘要、成功后懒加载全部结果并可选择非首个结果、失败原因/阶段/耗时、加载重试和删除后结果刷新；
运行成功进度终态、Arrow 字符串视图分页和不同宽度数据集详情居中；分析、视图、比较在来源删除后继续读取；
两用户隔离（包含六个模型表读取端点）；重启收敛与 FTS 局部失败；配额、恶意归档和模型表窄屏响应式无障碍。

RUN-001 在 Rust 层验证创建、重放、列表和详情都返回绑定脚本的必填 `scriptName`，在 Vitest 层验证中文摘要、终态/动态耗时、
失败详情、运行列表与结果加载错误、按运行请求代次隔离和 `datasetRevision` 刷新，在 Playwright 层验证显式运行产生的两个结果
都出现在展开记录中，并能选择非首个 `loss` 打开工作台；真实失败场景验证实际阶段、退出码和多行 tensoreval 报错可见，绝对路径
被隐藏且不请求数据集。运行记录仍不可删除，脚本仍不自动运行。

应用层 scripted-agent 测试额外覆盖配置卡已创建后的 provider 收尾失败不覆盖成功、对话 Agent 无脚本创建权限；
并用“求解牛顿法潮流”等原始短指令验证显式函数选择不被拓扑改写，缺 Data/Struct、资源歧义和禁用门面都只返回
选择或配置卡，不提前生成或运行脚本。

ScenarioEngine 场景由控制端按调用顺序排队，不使用 sleep；timeout/cancel 使用 barrier，并能排队提供二进制 stderr、
结果合同错误和监控错误。进程 adapter 的 Rust 测试使用独立 helper process 验证双流超过 1 MiB 仍持续排空、完整 hash、
非 UTF-8、非零退出码、进程组超时/取消、运行中三类结果限额、符号链接/目录、监控失败和退出后最终扫描；所有失败
都断言未发布目录被清理。测试证据写入
`target/functional-coverage/e2e-routes.ndjson`，包含通过的测试名及实际 method/path/status。

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

候选发布执行 `scripts/test-release-integration`。报告记录测试 ID、Agent bundle hash、用户 seed hash/数量、engine
contract、当前 binary hash、动态资源 snapshot 与案例分类数量；hash 是证据而非固定接收值，日志必须脱敏。
真实门禁必须覆盖模型导入/发布、目录驱动的研究门面检索、配置卡准备与最终确认、Agent 创建受管研究脚本、自动执行、结果分析/比较和只使用稳定 ID，并动态导入所有
可导入案例、报告个别不可导入案例、执行按来源类型与 study 选择的代表案例。

## Failure policy

不允许 `skip`、`ignore` 或 manual-only 填补覆盖。失败先保留 trace/screenshot/log，按最窄层复现，再运行全量。
当前引擎不兼容不能通过放宽校验解决；应修复当前 eig-rs 或正式更新应用 contract 和全部证据。服务不会自动
改写引擎工作树；资源或 binary 更新后必须重启并重新生成运行时证据。
