# V4 HTTP 与 SSE API

> 本文跟随已实现的 V4 公共接口更新。Tasks 1–12 的统一上传、独立资源、研究配置、受管执行、重试和删除接口已实现；
> 对话内完整配置卡和运行到脚本消息定位界面已经接入。已确认的行为边界见
> [`2026-10-26-independent-data-resources-design.md`](superpowers/specs/2026-10-26-independent-data-resources-design.md)；
> 当前接口表只记录已经实现并可验证的能力。

## Conventions

除 `/health`、登录与登出外，所有路由要求 HttpOnly 登录 Cookie。ID 出网为字符串，时间为 ISO 8601，字段为
camelCase。跨用户与不存在资源统一 404。错误响应为
`{"error":"…","code":"stable_code","details":…}`。上传使用 multipart；upload batch 创建返回 202，运行创建返回
202（幂等重放为 200）。

## Source navigation

公共 method/path 的权威来源是 [`backend/src/api/mod.rs`](../backend/src/api/mod.rs) 中同时构造生产 Router 和
`route_manifest()` 的注册表；当前合同保持 81 条唯一 method/path。各 handler 上的 Rustdoc 解释本地 extractor、用户作用域、
application 调用与响应语义，但不替代路由注册表。公共错误响应集中在
[`backend/src/api/error.rs`](../backend/src/api/error.rs)，分页列选择解析和下载附件构造分别集中在
[`backend/src/api/query.rs`](../backend/src/api/query.rs) 与
[`backend/src/api/attachment.rs`](../backend/src/api/attachment.rs)。

路由族按职责位于 `backend/src/api/` 的 `auth`、`sessions`/`events`、`uploads`、`data_resources`、
`study_configurations`/`workflows`/`retries`、`scripts`/`runs`、`datasets`/`analysis`/`comparisons`/`chart_views`、
`workspaces`/`model_dataframes` 和 `examples` 模块。浏览器 DTO 以
[`frontend/src/types/`](../frontend/src/types/) 为源码入口；领域状态机和事务语义继续由
[`backend/src/application/`](../backend/src/application/) 的对应 service facade 拥有。

## Routes

| Method | Path | Semantics |
|---|---|---|
| GET | `/health` | 存活检查 |
| POST | `/api/auth/login` | 固定用户登录 |
| POST | `/api/auth/logout` | 撤销当前浏览器会话 |
| GET | `/api/me` | 当前用户 |
| GET | `/api/storage-usage` | 当前用户配额 |
| GET | `/api/examples` | rssbundle 案例目录树与搜索 |
| GET | `/api/examples/{id}` | 规范 manifest 案例详情 |
| POST | `/api/sessions/{id}/draft/example` | 案例进入统一上传候选批次 |
| GET/POST | `/api/sessions` | 列表/新建会话 |
| PATCH/DELETE | `/api/sessions/{id}` | 改名/携带会话影响 token 幂等删除整个会话 |
| POST | `/api/sessions/{id}/deletion-preview` | 返回删除影响统计和短期 impact token |
| GET/POST | `/api/sessions/{id}/messages` | latest/before/around 连续窗口或发送消息；POST 返回权威 `sessionTitle` |
| GET | `/api/sessions/{id}/events` | 以 `messageId:revision` 为游标的 SSE replay + live |
| GET | `/api/history/search` | 当前用户存活会话的历史检索，消息命中返回稳定 message ID |
| POST | `/api/sessions/{id}/upload-batches` | 上传 DSS/ZIP/CSV/Excel/Parquet 并建立可恢复候选批次 |
| GET/DELETE | `/api/upload-batches/{id}` | 读取/放弃上传批次 |
| PUT | `/api/upload-batches/{id}/parse-settings` | 提交受控解析设置并重新解析 |
| PUT | `/api/upload-batches/{id}/candidates` | 确认每个候选的模型表/数据资源/跳过用途 |
| POST | `/api/upload-batches/{id}/confirm` | 按两个独立原子子事务发布模型候选和普通数据资源 |
| POST | `/api/data-resources/{id}/replacement-batches` | 为已有资源建立替换候选批次 |
| GET | `/api/sessions/{id}/data-resources` | cursor 分页列出当前普通数据资源 |
| GET/PATCH | `/api/data-resources/{id}` | 读取元数据或修改 display name/alias |
| GET | `/api/data-resources/{id}/rows` | 受控列投影和 cursor 分页 |
| GET | `/api/data-resources/{id}/download` | 下载 `original` 或规范 Parquet |
| POST | `/api/data-resources/{id}/structured-replacement-previews` | 校验 Struct 参数覆盖并生成短期预览 token |
| POST | `/api/data-resource-replacement-previews/{id}/confirm` | 以 Replace/SaveAsNew/Cancel 确认参数覆盖 |
| POST | `/api/data-resources/{id}/deletion-preview` | 生成资源引用影响和短期删除 token |
| DELETE | `/api/data-resources/{id}` | 以 impact token 幂等删除并释放直接 artifact 引用 |
| POST | `/api/study-configurations/{id}/confirm` | 以当前 revision 最终确认并原子创建受管 workflow/provisional run |
| GET/DELETE | `/api/study-configurations/{id}` | 读取配置卡/以 `expectedRevision` 取消未确认配置 |
| PUT | `/api/study-configurations/{id}/facade` | 以 revision 更换目录门面并重建输入/结果卡 |
| PUT | `/api/study-configurations/{id}/bindings/{inputId}` | 选择或清除兼容的普通数据资源 |
| POST | `/api/study-configurations/{id}/bindings/{inputId}/mapping` | 受控列选择/重命名/类型转换并发布不可变派生资源后绑定 |
| PUT | `/api/study-configurations/{id}/structs/{inputId}` | 以 `{expectedRevision,values}` 更新本次研究的 Struct 参数草稿 |
| PUT | `/api/study-configurations/{id}/results` | 提交有序且唯一的结果集合 |
| GET | `/api/workflows/{id}` | 查询受管执行、attempt、脚本锚点和结果数据集 |
| POST | `/api/workflows/{id}/cancel` | 幂等取消非终态受管执行 |
| GET/POST | `/api/sessions/{id}/scripts` | 列表/上传单个 `.rss` |
| POST | `/api/sessions/{id}/scripts/text` | 以文本创建脚本 |
| GET | `/api/scripts/{id}` | 懒加载脚本全文 |
| GET | `/api/sessions/{id}/workspace` | active、draft、导入状态 |
| GET | `/api/model-dataframes/{id}` | 已发布版本表的只读元数据 |
| GET | `/api/model-dataframes/{id}/rows` | 已发布版本表的 cursor 分页与列投影 |
| GET | `/api/model-dataframes/{id}/download` | 下载已发布版本表的 Parquet |
| GET | `/api/draft-dataframes/{id}` | 指定草稿 revision 的只读表元数据 |
| GET | `/api/draft-dataframes/{id}/rows` | 指定草稿 revision 的 cursor 分页与列投影 |
| GET | `/api/draft-dataframes/{id}/download` | 下载指定草稿 revision 的 Parquet |
| GET/POST | `/api/sessions/{id}/versions` | 历史/显式生成版本 |
| GET/DELETE | `/api/versions/{id}` | 详情/带 impact token 删除 |
| POST | `/api/versions/{id}/activate` | 激活历史版本；草稿需确认放弃 |
| POST | `/api/versions/{id}/deletion-preview` | 生成短期删除影响 token |
| POST | `/api/scripts/{id}/runs` | 唯一运行创建入口 |
| POST | `/api/scripts/{id}/retry-previews` | 失败 Agent 脚本按当前输入创建重试或差异预览 |
| POST | `/api/retry-previews/{id}/confirm` | 用户确认输入差异后创建派生 workflow |
| GET | `/api/sessions/{id}/runs` | 运行记录 |
| GET | `/api/runs/{id}` | 冻结运行快照 |
| POST | `/api/runs/{id}/cancel` | 幂等取消排队中或运行中的普通仿真；受管运行必须取消 workflow |
| POST | `/api/runs/{id}/deletion-preview` | 返回终态运行、直接结果和派生引用影响 token |
| DELETE | `/api/runs/{id}` | 以 impact token 原子 tombstone 运行与全部直接结果 |
| GET | `/api/runs/{id}/datasets` | 运行的存活结果 |
| GET/DELETE | `/api/datasets/{id}` | 元数据/删除单个结果 |
| GET | `/api/datasets/{id}/rows` | cursor 分页投影 |
| GET | `/api/datasets/{id}/download` | 流式下载完整 Parquet（含 `Content-Length`） |
| GET | `/api/datasets/{id}/chart-capabilities` | 返回可用字段、List 分量和图表预算 |
| POST | `/api/datasets/{id}/chart-previews` | 校验受控配置并返回非持久化 `ChartModel` 预览 |
| POST | `/api/datasets/{id}/views` | 以幂等操作 ID 保存不可变图表与 Plot Data |
| GET | `/api/analysis-runs/{id}` | 分析记录与结果 |
| GET | `/api/dataset-views/{id}` | 已保存图表元数据、冻结配置和 `ChartModel` |
| GET | `/api/dataset-views/{id}/data` | 已保存图表的物化 Plot Data |
| DELETE | `/api/dataset-views/{id}` | tombstone 图表并释放自身 artifact |
| GET | `/api/sessions/{id}/dataset-views` | 会话内存活图表，含已删除来源的图表 |
| GET | `/api/run-comparisons/{id}` | 运行比较及派生结果 |

## Upload batches and draft

上传字段为 `clientBatchId`、可选 `relativePaths`，以及一个或多个 `file.<clientFileId>`。批次状态为
`processing|awaiting_parse_settings|awaiting_purpose|ready_to_confirm|partially_confirmed|confirmed|failed|discarded|expired`。
解析阶段只保存候选，不修改草稿或发布数据资源；`confirm` 才执行真实双发布事务。
CSV/Parquet 每文件一个候选；Excel 只读取可见非空 worksheet 的缓存值，不执行公式、宏或外部链接。候选用途只有
`model_table|data_resource|skip`。普通静态 DSS 仍拆成 20 个模型候选和一个普通 `para` 数据候选；案例按当前
engine snapshot 中 manifest v2 绑定的 projection profile 动态执行，`projectorOutputs.inputKind=model` 形成模型候选，
`data|struct` 形成普通资源候选，diagnostic 不形成候选。其他表格模型 schema 从有效门面动态取得，并按实际列名推断。暂存默认保留 24 小时，配额以
`used_bytes + reserved_bytes` 原子检查；失败、放弃和到期幂等释放。

确认请求为 `{decisions,replacementAction?,expectedDraft?,clientOperationId}`，其中 `clientOperationId` 必填且由客户端为本次确认生成。普通上传的 ModelTable 候选在一个草稿事务中
发布，DataResource 候选在另一个资源事务中发布（案例确认使用下文“案例整体切换”的单事务合同）；任一侧失败不回滚已成功的另一侧，批次进入
`partially_confirmed`，重试只处理失败侧。两侧均失败回到 `ready_to_confirm`，两侧成功才进入 `confirmed` 并释放
剩余 reservation；只有失败侧仍引用的暂存文件和 artifact 会继续占用空间，成功侧完成后立即结算，Skip 候选及
已完成侧候选的暂存引用也会回收。成功子事务涉及的既有和新用途决定都被冻结，不能在重试时把 Skip 候选补入已发布侧。响应显式返回
`modelStatus`、`resourceStatus`、可空的 `modelReceipt`、可空的 `resourceReceipt`、`diagnostics` 和新发布的
`resources`，因此部分成功及其重放不需要根据状态猜测已提交对象。非取消确认会持久化 `clientOperationId` 和规范请求 hash；相同操作和相同 payload 重放返回既有 receipt，相同操作携带不同 payload 返回
`409 idempotency_conflict`。正在执行确认或批次已经全部确认后，更换操作也返回该冲突；一次确认已收敛为
`ready_to_confirm` 或 `partially_confirmed` 后，可用新的操作 ID 重试失败侧，已成功侧的决定和 receipt 继续冻结。持久幂等键与短期执行锁分离：正在提交子事务时，候选修改和 DELETE 放弃返回
`409 upload_state_invalid`，后台到期清理延后；尝试结束后仍可取消失败侧或清理到期批次。
同操作、同 payload 的已开始请求优先于暂存 TTL 判断：执行中重放等待既有执行者，已结算重放只读返回既有 receipt，即使原暂存到期时间已经经过也不重新执行。
候选决定写入、输入冻结标记和执行锁在一个事务完成。提交 `expectedDraft` 时必须与当前草稿 ID/revision 一致，否则模型子事务返回
`draft_revision_conflict` 诊断。

内部所有权不改变上述公共协议：`UploadConfirmationService` 是确认编排的唯一入口，集中完成 operation claim/replay、候选冻结、
原文件提交、发布策略选择、失败诊断和 receipt 读取；`UploadBatchService` 只负责接收、解析触发、恢复、过期和放弃。
普通上传选择 `OrdinaryUpload`，继续先应用模型候选到草稿、再发布资源，并允许两个子事务独立成功；`sourceKind=case` 选择
`CaseSwitch`，继续使用下文的单事务整体切换。alias、候选状态和确认 token 的后端校验集中后，路由、请求/响应字段、HTTP 状态和
稳定错误码均保持兼容，客户端仍不得提交服务器路径、SQL、命令或任意 ECharts option。

## Independent data resources

资源 ID 为 opaque 字符串，alias 在会话内按大小写归一后唯一。默认 alias 冲突时服务端生成 `data_<shortId>`；
用户明确指定的冲突 alias 返回 `resource_alias_conflict`。当前引擎 snapshot 新增保留名后，资源读取/列表会把冲突
资源标为 `needs_rename`，改名后恢复 `live`。静态导入的 `para` 是带 `schemaRef` 和来源血缘的普通资源，不进入
模型草稿。其他模型表范围完全由当前 catalog 的 Model schema 决定。

上传资源同时保存原文件和 Snappy Parquet；多个 worksheet 可共享原文件 artifact。结果或分析提升的资源复用
既有规范 artifact，没有原文件入口。`rows` 只接受 limit/cursor/columns；下载不返回服务器路径。普通资源可直接
作为 `run_result_analysis` 的稳定 `resourceId` 来源，派生血缘冻结资源 ID、schema hash 和 content hash。用户配置图表
只接受仿真或分析结果的稳定 dataset ID，不接受数据资源、服务器路径、SQL 或任意 ECharts option。

重新上传与独立 Struct 资源工作台覆盖会创建新 artifact。重新上传确认必须显式提交 Replace、SaveAsNew 或 Cancel；Replace
原子转移当前 alias、使依赖旧资源的未确认研究配置失效，并把旧资源保留为历史 replacement；
SaveAsNew 在默认 alias 冲突时分配 `data_<shortId>`，显式冲突仍返回错误；Cancel 不发布。Struct preview 校验当前
schemaRef、规范字段和 expected content hash，确认前不创建资源，内容变化或其他路径已替换原资源时返回
`resource_stale`；同一资源的新 preview 会回收被取代的未确认 artifact，过期 preview 由启动恢复和后台清理回收。
删除先生成 impact token，并统计未确认配置、冻结工作流、分析输入及数据集血缘引用；
确认后资源立即从普通读取隐藏、使未确认配置失效、清空 tombstone 行的 artifact 外键并释放直接引用。冻结工作流和
已物化派生结果持有自己的引用或快照，因此删除不破坏历史；重复 DELETE 不重复释放。

## User-configured charts

图表由固定 HTTP API 和确定性编译器生成，不经过对话 Agent。`chart-capabilities` 最多扫描 10,000 行来发现字段与
最多四层的数值 List 分量，并返回服务端预算；`chart-previews` 和保存接口仍读取完整所需投影并重新校验，预览不产生
数据库或 artifact 写入。请求只能提交六类受控 `ChartConfig`（line、bar、scatter、heatmap、boxplot、histogram），
包括模板、字段/分量、顺序、聚合、采样、自定义系列名，以及标题、图例、调色板、网格线、折线线型和数据点设置。
名称去除首尾空白后为 1..=100 个字符，标题最多 200 个字符，系列展示名为 1..=64 个字符；多 measure 折线、柱图和
散点图必须具有同一 unit。LTTB 只适用于数值横轴折线，单系列目标点数为 100..=10,000。

服务端响应的 `ChartModel` 是稳定的 `chart-model-v1` 数据与表现合同，不是 ECharts JSON。前端只把它映射为本地
ECharts option，不执行后端 formatter、JavaScript 或任意 option。默认预算为最多扫描 1,000,000 行、256 列、
32 个引用列、16 个 measure、500,000 个原始数值、100 个 series、50,000 个输出点、5,000 个类别、
100,000 个热力格和 10 MiB 响应；每个 List 最多发现
64 个分量路径，capability 响应最多 1 MiB。超限或不兼容不会截断成误导图表，而是返回稳定错误。

保存请求携带 UUID `clientOperationId`；同一来源、同一操作和相同规范请求幂等返回原图表，不同 payload 返回
`chart_operation_conflict`，遗留不完整操作返回 `chart_operation_gone`。成功保存冻结规范配置、配置 hash、诊断、
来源名称/schema/content 快照和物化的 Snappy Parquet Plot Data；元数据及 Plot Data 发布后不可修改。来源结果删除后，
图表仍可从会话“图表”页发现、打开和删除，并标记来源已删除；复制为新图表必须在来源仍存活时重新预览并保存。
删除图表只 tombstone 图表并释放自己的 Plot Data artifact，不恢复来源，也不删除其他图表。

七条图表路由均显式按 `UserId` 校验，跨用户和不存在资源统一返回 404。稳定图表错误为
`chart_config_invalid`、`chart_capabilities_too_large`、`chart_column_missing`、
`chart_column_incompatible`、`chart_component_invalid`、`chart_unit_conflict`、
`chart_series_name_conflict`、`chart_duplicate_requires_aggregation`、`chart_no_data`、
`chart_too_large`、`chart_name_conflict`、`chart_operation_conflict`、`chart_operation_gone`、
`chart_source_deleted`、`chart_materialization_failed`、`chart_not_found` 和 `chart_cursor_invalid`。

## Managed study configurations

`search_study_facades` 只查询本次 engine snapshot 的有效门面，显式函数名优先，返回目录中的方法差异和禁用诊断；
`prepare_study` 参数包含目录 `facadeId`、结构化 `intent`、必填 `explanation` 与可选来源 workflow ID；`explanation` 去除首尾空白后必须为 1..=4000 个 Unicode 字符，否则返回
`invalid_study_explanation`，并且不执行领域写入。成功时工具持久化模型说明和服务端生成的实际研究标题/配置 ID 页脚，再持久化配置卡；不生成脚本或运行。状态为
`choosing_facade|collecting_inputs|ready_to_confirm|confirmed|invalidated|cancelled`，所有卡片写入都必须提交
`expectedRevision`。跨用户和不存在配置统一 404，revision、资源或 snapshot 竞争返回
`configuration_stale`。

模型输入只按门面声明从 active 版本的实际表名动态绑定；`para` 等 Struct 只绑定普通数据资源。兼容资源唯一时自动
绑定，多个兼容资源才要求用户选择；同门面的历史 Struct 只返回复用建议，不能在存在歧义时静默沿用。Struct 卡由规范 schema
生成字段类型、单位、范围和枚举，并通过可向后兼容的 `resultSelectorFields:string[]` 标记当前门面由结果选择管理的字段。
这些字段不接受 Struct 更新请求中的伪造值，也不会进入用户覆盖。所有可编辑普通字段都只形成当前研究的私有覆盖：已绑定资源
只作为只读参数模板，未绑定资源则从目录默认值建立有效值；两者都不要求 Replace/SaveAsNew，也不创建、替换、重命名或发布
`data_resources`。Struct 更新请求固定为 `{expectedRevision,values}`，服务端按 schema 过滤 selector 字段并保存草稿。
Struct 差异按 JSON 数值语义比较，`50` 与 `50.0` 不产生虚假的用户覆盖。旧的 selector-only `struct_edit` 只有在来源资源仍
存活、同 session/schema 且 lineage 合法时才折叠出候选集；否则仍作为真实候选参与消歧。

Data 输入的 mapping 只接受源列到目标列的受控映射。服务端按目标 schema 转换为 Snappy Parquet，经统一上传发布
事务创建 `sourceKind=mapping` 的新普通资源，并冻结源资源 ID、目标 schema 和映射血缘；不会修改源资源。结果集合
必须有序、唯一、属于门面且满足 fixed/minimum/conditional 约束。

`PUT /api/study-configurations/{id}/results` 与 Agent 工具 `update_study_results` 共用同一个
`select_results` 领域操作。工具参数固定为 `{configurationId:string, expectedRevision:integer, results:string[]}`，其中
配置 ID 是十进制稳定字符串；工具先要求 `study-planning`，再校验配置属于当前用户和当前会话。跨会话或不存在配置统一
`not_found`，过期 revision 或 confirmed/invalidated/cancelled 配置返回 `configuration_stale`，空、重复、未知或违反目录约束的
结果返回 `result_selection_invalid`。成功只按传入顺序持久化结果、递增 revision 并更新同一条 `study-config` 消息；不确认配置、
不创建脚本、workflow 或 run。

## Managed study workflows

`POST /api/study-configurations/{id}/confirm` 只接受
`{expectedRevision, clientOperationId}`。服务在一个 immediate transaction 中重新校验配置、当前引擎快照、active
模型、资源 ID/hash/schema 和结果合同。所有 Struct 用户参数和结果 selector 都合成为本次工作流私有的单行 Snappy Parquet
artifact，不创建或修改 `data_resources` 行；有效值与模板字节一致时可复用模板 artifact。工作流输入以可空 `resourceId`
和 `source` 标识长期模板：有模板时冻结其 ID/alias/artifact/schema/content hash，无模板时两者为 `null`；顶层
`artifactId/contentHash` 与 `effective` 指向实际执行文件，并冻结 `values`、`userOverrides`、`selectorOverrides` 以及兼容旧记录的
合并 `overrides`。随后确认配置、创建 managed workflow、持久进度消息和 provisional run。确认路由在领域事务成功后读取权威配置，
以同一 configuration ID 原位发布 `confirmed` 的 `study-config` 卡；重试仍按相同操作 ID 幂等返回同一 workflow/run，并补写当前终态，
不会创建重复卡片或 workflow。同一操作 ID 配合不同请求返回 `idempotency_conflict`。按钮固定为“确认并生成脚本”。

`GET /api/workflows/{id}` 返回工作流、修正预算、run、可空脚本消息锚点和脱敏 attempt 摘要。每个 attempt 还返回
`durationMs`、按 `stdout|stderr` 区分的 `engineLogs`、可空 `failureAnalysis` 和
`scriptChangedFromParent`；日志只含服务端已脱敏的保留文本、总/发送字节数与截断标志，不返回审计 artifact ID。分析固定区分
已确认事实、可能原因、未知信息和建议动作，事实与推断都携带 `stderr:` 日志引用。旧记录缺少这些字段时返回空值或空数组；
`POST /api/workflows/{id}/cancel` 幂等收敛到 `cancelled_by_user`。同一会话只允许一个
`queued|generating|executing|repairing` 工作流，冲突返回 `409 workflow_in_progress` 及当前 workflow ID。跨用户和不存在
统一 404。首份候选脚本产生前 managed run 的脚本资源/artifact/hash 和 `scriptMessageId` 均为空；终态发布脚本资源与
script 消息时一次性绑定，之后不可更换。正常完成、取消或服务重启时，只要已有候选脚本，都会把最后一个候选原子发布为
稳定 script 消息锚点；只有候选尚未产生时终态 run 才继续为空。服务重启把非终态 workflow、attempt 和 run 收敛为
`service_restarted`，不从临时文件续跑。

首次确认提交后，后端立即调用无工具 `ScriptAuthor` 生成候选并保存 candidate artifact。结构检查只验证冻结门面函数、
输入 alias/顺序、Struct 资源引用和结果绑定未变化；一般 RSS 语义与运行可行性以 tensoreval 为准。候选通过结构检查后
立即使用确认时冻结的 model/Data/Struct artifact 构造私有命名输入树并执行。非零退出、结果数量/大小限制和
`result_contract_invalid` 可进入修正；`result_monitor_failed`、路径安全、配额和基础设施错误直接失败。生产引擎执行没有
固定墙钟上限，默认等待完成；取消工作流会取消当前引擎 future 并终止其进程组。
默认最多 3 个 repair candidate，因此最多 4 次引擎调用。每次引擎失败先持久化公开脱敏日志，再 best-effort 调用独立的
失败分析模型；模型不可用、超时或返回非法 JSON 不得隐藏日志、阻断修正或改变终态。每个 repair prompt 固定包含当前完整脚本、脱敏 stderr、结构或
结果诊断、冻结合同和“只输出修改后的完整脚本，不要解释”。发送内容按 UTF-8 边界保留 64 KiB 首尾；完整有界 stderr
仅保存为审计 artifact，不进入普通 API、消息、搜索或模型请求。

成功要求结果稳定名称集合与冻结选择完全相等，每个文件是可读 Snappy Parquet，列名与顺序必须精确一致。逻辑字符串
接受 Arrow `Utf8/LargeUtf8/Utf8View`，逻辑 list/matrix 接受 `List/LargeList/ListView/LargeListView` 且元素类型必须
匹配；物理 Arrow nullable 标志不直接替代数据合同，服务会扫描合同声明为非空的列并拒绝实际 null。输出携带 unit
元数据时必须与冻结合同一致；tensoreval 未写 unit 元数据时允许缺省，不能接受冲突单位。零行合法。全部结果验证通过后，最终 attempt、脚本资源/消息锚点、
run、所有 result dataset 和 workflow 终态在同一数据库事务发布，任一失败不留下部分结果。取消可中断正在等待的
ScriptAuthor 或引擎 future，并由既有终态事务收敛。

固定按钮的发布请求携带 `expectedDraftId`、`expectedDraftRevision`、`clientPublishId`。对话中的
`publish_model_version` 工具只在用户当前消息明确要求发布时可用，参数只含刚由 `inspect_workspace` 读取的
`expectedDraft: {id, revision}` 与可选描述；内部幂等键由 request message ID 和 draft ref 生成，LLM 不能提供。
两条路径都复用同一版本服务：成功冻结版本、清空草稿并自动激活，内容重复时复用既有版本。研究、修改、案例导入或
“生成脚本/结果”意图不会隐式发布；revision 已变化时返回 `draft_revision_conflict`。
工具运行时不只依赖 Prompt：当前 request message 未明确授权时返回 `publish_authorization_required`，当前轮次未成功执行
`inspect_workspace` 时返回 `workspace_inspection_required`；描述不合法返回 `version_description_invalid`。授权采用保守的消息开头肯定命令，
否定、问句、引用和解释请求均不授权。这些失败均不改变工作区。
激活历史版本只移动 active 指针；存在草稿时必须提交 `discardDraft=true` 以及准确 expected draft。

模型版本表路由只依赖不可变的 `modelDataframeId`。草稿表的三个路由都必须提交 `draftId` 与
`expectedDraftRevision`；任一草稿表变化后，旧 revision 返回 `409 draft_revision_conflict`。rows 支持
`limit=1..500`、资源绑定的 opaque cursor 和逗号分隔的 `columns` 投影，响应为
`{resourceId,columns,rows,nextCursor,diagnostics}`。版本页的 `resourceId` 为 `model-dataframe:{id}`，草稿页为
`draft-dataframe:{id}:revision:{revision}`；前端不得合并不同 resourceId 的页面。元数据包含 scope、ownerId、revision（仅草稿）、表名、
行数、大小、列 schema、schema hash 与 content hash。download 返回原始 Snappy Parquet。读取不会激活版本、
推进草稿、增加 artifact 引用或创建结果数据集；不存在、已删除或跨用户资源统一返回 404。

## Example catalog

`GET /api/examples` 只接受可选 `query` 和 `studyType`，返回
`{engineResourceSnapshot, tree}`。`tree` 是 `directory`/`example` 判别联合：目录包含展示 `path` 和递归
`children`，叶节点包含稳定 ID、`categoryPath`、`bundlePath`、可空 study/solver/compatibility/sourceEntry、
results、文件计数、验证状态与诊断。搜索匹配名称、bundle 路径、study、solver、compatibility、results 和来源
入口，并保留匹配叶节点的全部祖先；不再分页或截断为扁平列表。无效 manifest 叶节点仍返回，但
`importable=false`。

`GET /api/examples/{id}` 增加可空 `entryPath`/`manifest`、角色化 `files`、`treeHash` 和 diagnostics。manifest
保留 schema、artifact、generator、PSA contract、study、source、规范 projection、文件声明与 inputs，不返回物理源路径或文件内容。
应用请求提交 `{exampleId, engineResourceSnapshot, clientBatchId}`（过渡期也接受字段别名 `clientImportId`）；后端只从
已发布快照读取 manifest 声明清单，全部哈希通过后由 adapter 执行 profile 声明的 projector，并将精确有序输出进入
统一上传用途确认。它不会直接生成草稿、版本或运行，也不会把源路径或 projector 暴露给客户端。

Agent 的 `search_examples` 使用相同搜索谓词并返回最多 50 个规范摘要；`inspect_example` 只按稳定 ID 返回规范
manifest、角色化文件哈希和验证诊断。两个工具只读，Agent 无权直接应用案例。

## Scripts and runs

上传脚本 multipart 只接受字段 `file` 的单个 UTF-8 `.rss`；`POST /api/sessions/{id}/scripts/text` 接受 JSON
`{name, content}` 直接以文本创建（`sourceKind: "manual"`）。两条路径只做名称、UTF-8 和大小校验后不可变落盘，
不使用 zjudss RSS 静态语法校验器；语法和运行可行性以用户点击运行后 tensoreval 的退出码、stderr 和结果为准。
脚本详情返回全文；列表和消息只返回有界预览。V4 受管研究已提供确认、workflow 查询与取消接口；生成、执行和自动
修正循环由受管工作流执行。普通运行请求为 `{clientRunId}`，前端不提交模型版本。后端在创建 run 的事务中按脚本类型
解析并冻结当前输入：成功 Agent 脚本只读取冻结门面实际需要的当前 active 表；Struct 模板只接受原资源本身或
`lineage.replacesResourceId` 显式指向它的最多 64 层 replacement 后代，拒绝仅同 alias 的案例切换/重新上传资源、循环和超深血缘，
并按用户覆盖后 selector 覆盖的顺序重建私有参数。无来源 Struct 直接复用冻结的有效 artifact；旧 snapshot 缺少分类字段时回退到
`effective.overrides`；
manual/upload 脚本读取存在时的全部 active 表和全部 live Data resource，没有 active 仍可执行。Agent 没有普通运行、
历史版本激活或删除工具；当前草稿发布只通过上述受控工具。

运行创建响应、session 运行列表和单运行详情中的 `SimulationRun` 都包含 `scriptName`；首份受管脚本生成前显示“脚本尚未
生成”，其 `scriptResourceId/scriptArtifactId/scriptContentHash/scriptMessageId` 可空，其他运行必须绑定不可变脚本资源。
运行快照继续冻结模型、脚本 artifact/hash、engine contract、Agent bundle
和 skill refs；`GET /api/runs/{id}/datasets` 只列出该运行当前仍存活的全部结果数据集。失败时保留实际执行阶段；API 将内部
adapter/IO/校验错误映射为受控 `error.code`/`error.message`。对普通 adapter 失败，`error` 还可选返回 `stage`、
`durationMs`、`engineLogs` 和 `analysisStatus`；`engineLogs` 每项只有 `stream|text|totalBytes|sentBytes|truncated`，
`analysisStatus=complete` 时可选包含有界 `analysis`。出网层会再次脱敏和校验日志引用，不返回 hash、adapter diagnostic、
provider 错误、命令或服务器路径；非法的可选分析降级为 `unavailable` 而不隐藏日志。旧记录的受控
`engineOutput` 仍兼容返回，不进行历史分析回填。adapter 对 stdout/stderr 分别持续排空、保留有界首尾并记录完整 hash；
tensoreval 以非零状态退出时保留实际退出码。原始有界 stderr 只供受限审计，不进入普通 API。manual/upload 使用开放结果合同，至少产生一个
名称合法且唯一的 Snappy Parquet；成功 Agent 脚本再次运行沿用冻结的结果名称和 schema 合同，合同漂移以
`result_contract_invalid` 失败且不修正。`POST /api/scripts/{id}/retry-previews` 只面向失败的受管 Agent 脚本：当前输入
未变化时直接创建带完整新修正预算的派生 workflow，变化时返回持久差异；用户通过
`POST /api/retry-previews/{id}/confirm` 确认后冻结最新输入。重试比较长期来源 hash 而不是私有有效 artifact hash；来源变化时
用最新普通字段重新物化原 selector overrides，来源未变时复用原有效 artifact。相同 `clientRetryId` 或 `clientRunId` 的
已创建操作在调度、读取当前资源和物化 overlay 之前直接返回原 workflow/run；事务内仍二次检查以处理并发。门面 hash
变化返回 `study_contract_changed`。

`POST /api/runs/{id}/cancel` 只用于普通仿真。排队中或运行中的 run 原子收敛为 `status=failed`、
`stage=cancelled_by_user` 和公开 `error.code=cancelled_by_user`，不会发布结果或启动失败分析；并发完成与取消只允许第一个终态提交。
重复取消已终态普通 run 幂等返回当前快照。不存在或跨用户统一返回 404；受管 run 返回
`409 managed_run_cancel_via_workflow`，客户端必须改用 `POST /api/workflows/{id}/cancel`。前端将
`cancelled_by_user` 显示为中性的“已取消”，不作为普通计算失败展示。
数据库中既有的 `engine_timeout` 错误 JSON 不迁移也不重写，仍经通用历史失败兼容读取；当前生产合同和稳定错误枚举不再
包含或产生该错误码。

终态运行通过 `POST /api/runs/{id}/deletion-preview` 获取短期影响 token，再用
`DELETE /api/runs/{id}?impactToken=...` 原子 tombstone run 和全部存活直接结果并释放其 artifact 引用。排队/运行中返回
`run_busy`，任一直接结果正被分析消费时返回 `result_busy`；已完成的派生、比较、视图和提升资源保留冻结血缘。
全部运行路由保持跨用户 404，创建仍只由显式按钮触发。

运行比较只选择同名、稳定键兼容且至少含一个同 dtype/同 unit 标量数值列的数据集。服务端生成受控 RSS：先用
`select` 重命名左右数值列，再按稳定键 `join`，最后用 `with_columns` 生成 `delta_*`；不依赖 eig-rs 目录中不存在的
私有比较函数。列表值保留为原始结果供表格/分析查看，不被伪装成标量差值。

## Messages and errors

持久消息类型是 `user`、`assistant`、`tool-call`、`progress`、`result`、`attachment`、`model-status`、
`comparison`、`script`、`upload-batch`、`study-config`、`workflow-progress`。普通对话消息追加写入；后三种操作卡按
`sessionId + kind + operationId` 更新同一稳定消息 ID，内容变化时递增正整数 `revision`，相同终态重放保持原 revision。
上传卡携带数据库单调 `batchRevision`；仓储拒绝低于已保存 config revision 的更新，同 revision 只允许配置状态向前推进，
并继续拒绝低于或等于已保存 batch revision、低于执行轮次的延迟状态，也不允许已经落库的终态被另一状态覆盖；GET upload/config
会幂等补写当前事实，以修复领域事务成功而消息写入中断的窗口。EventWriter 必须先提交数据库再广播。SSE `id` 为
`messageId:revision`，重连时同时按消息 ID 和 revision 去重；补发除游标后的普通消息外，还返回当前最新操作卡，即使该卡的
稳定消息 ID 早于游标，避免错过旧卡的新 revision。旧的纯数字 `Last-Event-ID` 仍按 revision 1 解释。卡片只携稳定 ID、
公共状态和脱敏诊断，不序列化服务器路径或原始 stderr；attempt
详情由前端默认折叠。运行关联字段统一为 `runId`。V4 对话 Agent 只持久化研究配置卡，不具备脚本创建权限。
`prepare_study` 成功时，工具先持久化一条 assistant 消息：正文是模型依据用户目标和目录结果生成的详细说明，页脚由服务端以实际研究标题与配置 ID 生成；随后才持久化 `study-config` 卡。
成功后的 provider 尾声不再写入，provider 收尾失败也不追加通用 Agent 失败，因此用户立即确认时工作流仍严格位于说明和配置之后。
自然语言修改结果时，`update_study_results` 使用当前 durable 卡的精确 ID/revision 原位更新该卡；成功后的 assistant 只说明
结果已更新并提醒用户仍需点击“确认并生成脚本”，不会把自然语言肯定解释为最终确认。
Agent 历史先以当前用户和会话查询权威配置状态，仅附带最新一张仍可编辑研究配置的有界摘要（配置 ID、revision、状态、门面 ID、
已选结果、可选结果 ID），不因消息 payload 滞后而注入已终态配置，也不附带完整 action card 或路径；工具仍在服务端重新校验用户、
会话、revision 和结果合同。
Agent 在完成配置前耗尽单轮模型调用预算时，assistant 明确提示本轮尚未完成配置并可重新发送请求；后台日志记录
session ID 和失败类别，消息正文不暴露 provider 诊断。

消息列表默认返回最新连续窗口，`beforeId` 向前分页；`aroundId` 必须精确属于当前用户的当前会话，否则统一 404，成功响应
必须包含目标并返回前后有界的连续窗口。历史搜索只索引卡片可读摘要，不索引脚本全文或 stderr。

删除会话必须先调用 deletion preview。预览返回模型、普通数据资源、脚本、运行、结果、分析 artifact 数和逻辑字节数；
影响 hash 还覆盖可变草稿及其 DataFrame revision/artifact、尚未发布的上传 source/candidate artifact、资源替换 preview 及其批次状态，防止预览后内容或临时占用变化仍使用旧 token 删除。
DELETE 携带 opaque `impactToken` 并在事务中重新计算影响。影响变化返回 `deletion_impact_changed`，无效或过期 token 返回
`invalid_deletion_impact_token`。确认后会话立即进入 `deleting`，从普通读取隐藏且 repository 拒绝新消息和领域写入；服务取消
该会话正在进行的 Agent/engine 工作，清理会话所有内容后保留最小 `deleted` tombstone。清理中断后只有同一已授权 token
可以恢复清理；其他 token 不能接管。重复 DELETE 幂等成功，删除中或已删除
会话的普通读取仍统一 404。

主要稳定错误包括认证/隔离、配额与上传、导入与草稿冲突、模型表分页/读取/受控查询、脚本输入兼容、active race、
运行队列、版本删除影响变化、结果 busy、分析/比较/视图校验、历史索引不可用、engine snapshot/contract 不匹配，
以及结果监控/容量/路径失败。权威枚举由
`cargo run --bin contract-manifest` 产生，并由功能目录逐项覆盖。

上传批次详情的 `diagnostics` 包含模型和数据发布子事务的失败信息；对象形态合并至 `errors`，数组形态追加。`resource_alias_conflict` 的 `message` 列出当前会话内冲突资源名称和别名，提示修改别名后重试，不改变稳定错误码。

### 案例整体切换

`sourceKind=case` 的确认操作是当前会话的完整案例切换，不采用普通上传的双子事务语义。准备批次不改变当前模型或资源；`diagnostics.caseProjection.title` 给出目标案例名称，`diagnostics.caseSwitch` 保存准备时的 active/draft 基线、当前资源基线、上一案例和待替换资源清单。确认时基线必须仍匹配；变化时诊断要求重新准备案例，不自动覆盖新修改。

案例候选须按目录角色整体确认。一次数据库事务退出上一案例资源及其 replacement 后代、完整替换模型草稿、创建/复用并激活模型版本、发布本案例 Data/Struct、失效未确认研究配置。用户普通资源保留，与新案例 alias 冲突时整体失败。成功的 `modelReceipt` 为 `{versionId,activated:true,deduplicated,caseBatchId}`；历史版本、运行、结果和旧资源引用保持可读。失败时模型和数据均未切换，两个子事务状态为 failed，批次回到 ready_to_confirm，已有模型/草稿/资源保持原状。普通上传和单资源替换仍使用原合同。
