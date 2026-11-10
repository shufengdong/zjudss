# 配电网仿真智能体 V4 架构

> V4 使用与模型版本分离的数据资源库和受管研究工作流，并以空数据库作为新基线；需求见
> [`2026-10-26-independent-data-resources-design.md`](superpowers/specs/2026-10-26-independent-data-resources-design.md)。

## Scope

V4 是空库直接切换版本。系统由 React/TypeScript 工作台、Rust/Axum 应用、SQLite 元数据与
内容寻址 artifact 存储、可选择 DeepSeek 或百炼的 LLM agent，以及只能经 `backend/src/adapters/` 调用的 tensoreval 组成。
不存在旧模型/任务协议的迁移、双读或兼容路由。

## Runtime flow

```text
固定用户登录 → 会话 → 统一上传批次 → 用途确认 ┬→ 模型表 → 可变草稿 → 生成并激活版本
                                                └→ 普通 Data/Struct 资源
自然语言研究需求 → 配置卡确认 → 受管脚本生成/执行/修正 → Snappy Parquet 结果
已成功/手写/上传 RSS 脚本 → 用户点击运行 → 使用最新所需资源 → 新结果
```

- `.dss`、DSS 目录、`.dssbundle`、`.rssbundle`、ZIP、CSV、Excel 和 Parquet 进入统一上传批次；确认前不发布。
- 每个候选显式选择模型表、普通数据资源或跳过。模型候选进入草稿，Data/Struct 候选成为可替换的独立资源；静态导入的 `para` 属于普通资源。
- `.rss` 是会话脚本资源，不属于模型版本。创建脚本不会运行；只有脚本卡片“运行”按钮能创建运行。
- 内置案例只来自 `SPARROWZZ_DIR/dss/rssbundle`。普通目录形成分类，直接包含
  `manifest.json` 的 `.rssbundle` 目录形成案例；`SPARROWZZ_DIR/dss` 其他位置的 `.rss` 不进入案例库。
- 受管工作流冻结确认时的模型、资源、门面、结果合同和 engine snapshot；脚本再次“运行”时按其输入合同解析最新资源。
- 分析、视图和比较保存输入快照与独立 artifact；来源结果删除后，派生结果通过 tombstone 血缘继续解释。

## Boundaries

1. 所有引擎进程调用都在 adapter 层，参数使用结构化 spec；handler、tool 和 application service 不拼命令。
2. 浏览器和 LLM 只提交稳定 ID、草稿 revision 与受控 spec，不提交服务器路径、SQL、命令或任意图表配置。
3. repository 查询显式接收 `UserId`；跨用户和不存在资源统一 404。
4. 发布后的模型版本、脚本 artifact、运行快照、结果和分析记录不可修改。
5. 生产启动必须通过当前 binary 的功能性 capability 探测、动态 engine resource snapshot、Agent bundle 和用户
   种子校验；binary 探测与资源 snapshot 是两个独立的失败关闭门禁，资源加载不消费 binary feature 集合；服务不修改
   eig-rs 工作树。
6. 结果事实来源仅为 Snappy Parquet；数据库保存索引、schema、行数、hash、状态和血缘。

## Consistency and recovery

统一上传在最终确认前不改草稿或资源库；确认使用相互独立的模型和资源子事务，允许部分成功后只重试失败侧。
草稿写入比较 `{id, revision}`，发布、确认和运行使用 client id 幂等。服务重启恢复 upload batch 和受管工作流，
无法继续的执行收敛为带稳定诊断的终态，已提交消息、版本、资源和结果保持不变。SSE 先持久化消息再广播。

引擎资源在启动时从当前 `SPARROWZZ_DIR` 读取并按字节生成 snapshot ID；发布阶段再次校验源 hash，并原子复制
到 `DATA_DIR/system/engine-resources/<snapshotId>`。案例导入只读取这个不可变发布树。源文件在扫描与发布之间变化、
已发布目录损坏或 catalog 不兼容都会失败关闭。更新文档、catalog 或案例资源后必须重启以发布新 snapshot；
单独替换 `TENSOREVAL_BIN` 无需重启，下一次 adapter 调用直接执行路径上的当前普通文件。

## Result, analysis, comparison, and chart ownership

结果读取先经过 `backend/src/application/result_sources.rs` 的 `ResultSourceService`：它以
`CurrentUser + dataset ID` 解析 live dataset、所属会话、用户 storage scope 内的 artifact、已解析 schema、artifact
内容 hash 和 `DataFrameReadSource`。结果表格分页/下载、dataset 分析输入和图表 capability/编译共享这一个来源事实；
数据资源与模型表因 ID 和生命周期不同，继续使用各自的受控来源解析。`ParquetDataFrameReader` 仍是应用层分页、投影、
Arrow→JSON、下载和批次扫描的唯一物理 DataFrame 读取内核。

分析、比较和图表只共享来源事实与无副作用读取，不共享状态机：

- `analysis/inputs.rs` 在一次解析中同时冻结输入快照和执行 artifact，`analysis/outputs.rs` 负责验证、提交和清理输出；
  `AnalysisService` 保留阶段迁移、引擎调度、原子发布和消息投影。
- `comparisons/compatibility.rs` 只从运行/结果事实推导稳定键、同名数值列和 dtype/unit 兼容性，
  `comparisons/script.rs` 只从该受控模型生成 RSS；比较仍通过既有分析门面执行并独立发布比较快照。
- `charting/compiler.rs` 保留 `ChartCompiler::compile` 门面；`compiler/common.rs` 与 `schema.rs` 保存共享不变量，
  line/bar/scatter/heatmap/boxplot/histogram 各自拥有算法。`charting/service/source.rs` 解析/读取来源，
  `materialization.rs` 构造不可变 Snappy Parquet Plot Data 和发布 draft，`presentation.rs` 只恢复冻结 DTO、模型和 cursor。

已保存图表的读取不重新访问来源 DataFrame，只消费冻结配置、schema、诊断和自己的 Plot Data artifact；来源 tombstone
后仍可恢复，并以 `sourceDeleted` 反映血缘状态。发布前来源被删除仍由原子 repository 边界拒绝，不产生 live view 或
artifact 引用。

前端 `ChartBuilder` 只组合 `chart_builder/` 下的受控层次：`fieldMapping.ts` 和 `state.ts` 做纯字段/config 转换，
`useChartBuilder.ts` 管理 capability seed、预览 fingerprint 和保存 operation ID，模板/显示控件保持受控。
浏览器继续只提交 `ChartConfig`，并由按图表类型分离的 adapters 将服务端 `chart-model-v1` 确定性映射为本地
`EChartsOption`；后端、DTO 和持久化都不接收任意 ECharts option。

案例扫描显式解析 manifest schema v2 的 schema/artifact、入口、study、source、projection、files 和 inputs，并把
projection 逐字段绑定到同一 snapshot 的 `case_projections.json` schema v2。单个无效 manifest 仍形成
带脱敏诊断的不可导入叶节点，不阻断其他案例。应用案例时先验证不可变快照中 `manifest.json` 和
`manifest.files` 全部哈希，再由 adapter 在私有树内执行已发布 projector；有序输出按目录角色成为 Model/Data/Struct
候选，diagnostic 只进入批次诊断，然后进入统一上传用途确认。bundle 内未声明文件不会被读取或暂存。HTTP、前端与
Agent 工具共享同一规范目录和 manifest 解释结果，并且都不返回物理路径。

## Backend code organization

`api` 只处理 HTTP/SSE、认证提取与 DTO；`application` 编排用例；`db/repositories` 封装带用户作用域的
持久化；`adapters` 是 tensoreval 唯一边界。跨用例重复出现的数据形状在所属层集中构造：Parquet artifact
元数据属于 application artifact 服务，结果事件引用属于事件模型，运行/分析状态迁移属于 repository 内部。
服务依赖通过具名结构在 `AppState` 组合根装配，避免构造参数顺序承载隐含语义。

错误类型按边界保留：adapter 错误携带稳定 code、category、用户可读 message 和可选诊断；repository 错误
表达存储与并发语义；application service 再转换为业务错误；API 最后映射为公开状态码和错误体。可选的大型诊断
在 adapter 错误内使用间接存储，使常见 `Result<T, AdapterError>` 保持紧凑，同时保持 JSON 合同不变。

repository 的异步请求路径把 Diesel 同步 SQLite 操作封装在 `spawn_blocking` 内；启动迁移、恢复与用户 seed
在开始监听 HTTP 前同步完成。历史搜索的 LIKE/FTS 查询和索引重建语句集中定义，运行与分析仓储共用状态迁移
值对象，但各自保留独立事务与表级 SQL。

维护性公共代码遵循以下唯一归属，业务模块只保留领域映射和调用点：

| 关注点 | 权威实现 | 边界 |
|---|---|---|
| application 组合根 | `backend/src/application/mod.rs` 的 `AppStateDependencies`、`AppStateOptions` 与 `AppState::assemble` | 生产和测试都使用具名依赖/选项；不再增加按参数排列的构造函数变体 |
| HTTP 错误响应 | `backend/src/api/error.rs` | 各领域错误类型保持独立；这里只集中公开 status/code/message/details 的映射和响应构造 |
| HTTP 查询列解析 | `backend/src/api/query.rs` | dataset、data resource 和 model DataFrame 共用同一逗号分隔解析；保持原始顺序、重复项和空项过滤语义，不承担字段授权 |
| durable 事件 | `backend/src/application/events/mod.rs`、`writer.rs`、`bus.rs` | `mod` 拥有 wire/card 投影，`writer` 保证先持久化再广播并处理 operation revision，`bus` 只拥有 session broadcast 生命周期 |
| 托管结果合同 | `backend/src/application/workflow_result_contract.rs` | 集中逻辑 dtype/unit/schema 与非空值校验；`WorkflowWorker` 继续拥有 author/repair、执行顺序和终态发布 |
| Artifact CAS 提交与 GC | `backend/src/db/repositories/artifact.rs` | 唯一实现 hash 去重、配额、新 artifact/link 和 usage 结算；上传原文件只在外层增加状态校验、幂等重放和指针写入 |
| Artifact 已有引用计数 | `backend/src/db/repositories/artifact_refs.rs` | 唯一实现可信用户 scope 解析、重复 ID 聚合和已有 link 的 retain/release；唯一 artifact 的 usage 只在 link 创建/删除时变化 |
| Artifact 领域事务 | 各 `backend/src/db/repositories/` 领域模块 | 决定引用集合、事务时机、stale/404 映射和 tombstone；不自行更新已有 `ref_count` |
| SQLite insert ID | `backend/src/db/repositories/mod.rs` 的 `last_insert_rowid` | 必须在产生行的同一连接上紧跟 insert 读取；领域 repository 不再复制 SQL 形式 |
| tensoreval 进程执行 | `backend/src/adapters/process/control.rs`、`execution.rs`、`result_scan.rs`、`monitor.rs`、`task_file.rs` | `control` 负责 spawn、双流捕获、超时/取消和进程组终止；`execution` 负责编排、退出映射与 final result；monitor 与退出后的 final scan 共用同一结果扫描语义；临时 RSS/wrapper 由 RAII 文件对象负责取消清理 |
| 引擎导入与投影 | `backend/src/adapters/process/import.rs` | 只编排受控 import/projector spec、临时任务文件和规范输出，不自行启动子进程或扫描结果目录 |
| 引擎资源读取 | `backend/src/engine_resources/tree.rs`、`loader.rs` | `tree` 唯一实现安全路径、遍历及有界读取/hash；`loader` 解析 catalog、文档与案例并按实际字节生成 snapshot identity，不依赖 binary capability |
| 引擎资源发布 | `backend/src/engine_resources/publisher.rs` | 唯一实现 generated/source inventory、复制时 hash 复核、staging 清理、原子 rename、既有 snapshot 校验和路径 rebase |
| 后端 E2E 状态装配 | `backend/src/testing/app.rs`、`backend/src/testing/engine_fixture.rs` | 统一 minimal/runtime 限制、scheduler、override 与 snapshot 到 `AppStateOptions` 的映射；仅在 `e2e-test` feature 下编译 |
| 后端集成测试生命周期 | `backend/tests/common/app.rs` | 只负责临时目录、数据库迁移、用户 seed 和调用共享 E2E 状态装配，不复制 `AppStateOptions` 构造 |
| E2E 事件提交故障 | `backend/src/testing/faults.rs` | 控制路由保留全局一次性故障；进程内并行测试按 app scope + session 注入，避免其他临时数据库的同号会话抢先消费计数 |
| 前端 HTTP 传输 | `frontend/src/api/client.ts` 的 `checkedFetch` 与共享下载路径 | 集中凭据、错误体解析和取消信号；领域 API 模块仍负责 URL、DTO 和业务参数 |
| 前端 DTO 测试数据 | `frontend/src/test/fixtures.ts` | 仅供测试使用；builder 自有的默认值每次深度隔离并只接受顶层覆盖，传入 override 的所有权仍属于调用方 |
| live 结果来源事实 | `backend/src/application/result_sources.rs` | 唯一组合 dataset 用户/live 校验、会话、artifact scope、schema 与 artifact content hash；不拥有删除或发布事务 |
| 物理 DataFrame 读取 | `backend/src/application/dataframe_reader.rs` | 唯一实现结果/资源/模型表的受控 Parquet 分页、投影、批次读取、cursor 内容绑定和 Arrow 值转换 |
| 分析输入与输出 | `backend/src/application/analysis/inputs.rs`、`outputs.rs` | 输入一次解析后同时用于快照与 staging；输出模块验证/提交/失败清理，服务门面继续拥有状态机和发布 |
| 比较兼容性与脚本 | `backend/src/application/comparisons/compatibility.rs`、`script.rs` | 前者是纯兼容性策略，后者只消费受控模型生成 RSS；不读取文件、不访问数据库、不成为通用脚本入口 |
| 图表编译算法 | `backend/src/application/charting/compiler/` | facade 保持不变；公共候选/预算/schema 与六类独立算法分层，不合并不同统计语义 |
| 图表来源与物化视图 | `backend/src/application/charting/service/` | 来源解析、Plot Data draft、冻结展示/cursor 分离；`ChartViewRepo` 继续独占幂等发布、列表和删除事务 |
| 前端受控图表构建 | `frontend/src/components/datasets/chart_builder/` | 纯字段/状态转换、Hook 副作用、模板控件和显示控件分离；只产生 `ChartConfig`，不产生或接收服务器 ECharts option |

部分文件因拥有一个不可再分割的合同或编排而有意保持较大：`catalog/schema.rs` 统一 dtype grammar、schema 引用与可执行规则；
`application/events/mod.rs` 保持同一消息 wire/card 投影；图表 compiler 的 facade/公共合同继续与按图表类型拆出的算法协作；
`workflow_worker.rs` 保留单个托管工作流的 author/repair/执行/终态顺序。维护时只抽取具有独立不变量和测试所有权的职责，
不为缩短文件而复制合同或改变公开 facade。

这些归属不允许把服务器路径、任意 ECharts option、SQL 或命令带入公共边界，也不改变各层原有的稳定错误码。

## Capacity and trust

Agent 调度默认实例 8、每用户 2、每会话 1、等待 32；引擎进程默认 4。artifact 配额按固定用户的可信
storage scope 计费。当前固定用户密码与 ID 相同是已明确接受的试用约束，不代表安全身份方案。
