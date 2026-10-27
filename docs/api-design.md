# V4 HTTP 与 SSE API（实施中）

> 本文跟随已实现的 V4 公共接口逐任务更新。Task 7 已实现 revisioned durable message、历史锚定窗口和会话影响预览删除；最终确认/自动执行、运行删除和运行到脚本消息定位接口仍待后续任务实现。已确认的行为边界见
> [`2026-10-26-independent-data-resources-design.md`](superpowers/specs/2026-10-26-independent-data-resources-design.md)；
> 路由和 DTO 将在实施方案中确定，当前接口表不提前声明未实现能力。

## Conventions

除 `/health` 与登录外，所有路由要求 HttpOnly 登录 Cookie。ID 出网为字符串，时间为 ISO 8601，字段为
camelCase。跨用户与不存在资源统一 404。错误响应为
`{"error":"…","code":"stable_code","details":…}`。上传使用 multipart；模型导入返回 202，运行创建返回
202（幂等重放为 200）。

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
| POST | `/api/data-resource-replacement-previews/{token}/confirm` | 以 Replace/SaveAsNew/Cancel 确认参数覆盖 |
| POST | `/api/data-resources/{id}/deletion-preview` | 生成资源引用影响和短期删除 token |
| DELETE | `/api/data-resources/{id}` | 以 impact token 幂等删除并释放直接 artifact 引用 |
| GET/DELETE | `/api/study-configurations/{id}` | 读取配置卡/以 `expectedRevision` 取消未确认配置 |
| PUT | `/api/study-configurations/{id}/facade` | 以 revision 更换目录门面并重建输入/结果卡 |
| PUT | `/api/study-configurations/{id}/bindings/{inputId}` | 选择或清除兼容的普通数据资源 |
| POST | `/api/study-configurations/{id}/bindings/{inputId}/mapping` | 受控列选择/重命名/类型转换并发布不可变派生资源后绑定 |
| PUT | `/api/study-configurations/{id}/structs/{inputId}` | 更新 Struct 卡草稿并声明 Replace/SaveAsNew 意图 |
| PUT | `/api/study-configurations/{id}/results` | 提交有序且唯一的结果集合 |
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
| GET | `/api/sessions/{id}/runs` | 运行记录 |
| GET | `/api/runs/{id}` | 冻结运行快照 |
| GET | `/api/runs/{id}/datasets` | 运行的存活结果 |
| GET/DELETE | `/api/datasets/{id}` | 元数据/删除单个结果 |
| GET | `/api/datasets/{id}/rows` | cursor 分页投影 |
| GET | `/api/datasets/{id}/download` | 下载 Parquet |
| GET | `/api/analysis-runs/{id}` | 分析记录与结果 |
| GET | `/api/dataset-views/{id}/data` | 物化视图数据 |
| GET | `/api/run-comparisons/{id}` | 运行比较及派生结果 |

## Upload batches and draft

上传字段为 `clientBatchId`、可选 `relativePaths`，以及一个或多个 `file.<clientFileId>`。批次状态为
`processing|awaiting_parse_settings|awaiting_purpose|ready_to_confirm|partially_confirmed|confirmed|failed|discarded|expired`。
解析阶段只保存候选，不修改草稿或发布数据资源；`confirm` 才执行真实双发布事务。
CSV/Parquet 每文件一个候选；Excel 只读取可见非空 worksheet 的缓存值，不执行公式、宏或外部链接。候选用途只有
`model_table|data_resource|skip`。静态 DSS/案例固定拆成 20 个模型候选和一个普通 `para` 数据候选；其他模型
schema 从当前 engine snapshot 的有效门面动态取得，并按实际列名推断。暂存默认保留 24 小时，配额以
`used_bytes + reserved_bytes` 原子检查；失败、放弃和到期幂等释放。

确认请求为 `{decisions,replacementAction?,expectedDraft?,clientOperationId}`，其中 `clientOperationId` 必填且由客户端为本次确认生成。ModelTable 候选在一个草稿事务中
发布，DataResource 候选在另一个资源事务中发布；任一侧失败不回滚已成功的另一侧，批次进入
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

## Independent data resources

资源 ID 为 opaque 字符串，alias 在会话内按大小写归一后唯一。默认 alias 冲突时服务端生成 `data_<shortId>`；
用户明确指定的冲突 alias 返回 `resource_alias_conflict`。当前引擎 snapshot 新增保留名后，资源读取/列表会把冲突
资源标为 `needs_rename`，改名后恢复 `live`。静态导入的 `para` 是带 `schemaRef` 和来源血缘的普通资源，不进入
模型草稿。其他模型表范围完全由当前 catalog 的 Model schema 决定。

上传资源同时保存原文件和 Snappy Parquet；多个 worksheet 可共享原文件 artifact。结果或分析提升的资源复用
既有规范 artifact，没有原文件入口。`rows` 只接受 limit/cursor/columns；下载不返回服务器路径。普通资源可直接
作为 `run_result_analysis` 和 `create_dataset_view` 的稳定 `resourceId` 来源，派生血缘冻结资源 ID、schema hash 和
content hash。

重新上传与 Struct 参数卡覆盖都创建新 artifact。重新上传确认必须显式提交 Replace、SaveAsNew 或 Cancel；Replace
原子转移当前 alias、使依赖旧资源的未确认研究配置失效，并把旧资源保留为历史 replacement；
SaveAsNew 在默认 alias 冲突时分配 `data_<shortId>`，显式冲突仍返回错误；Cancel 不发布。Struct preview 校验当前
schemaRef、规范字段和 expected content hash，确认前不创建资源，内容变化或其他路径已替换原资源时返回
`resource_stale`；同一资源的新 preview 会回收被取代的未确认 artifact，过期 preview 由启动恢复和后台清理回收。
删除先生成 impact token，并统计未确认配置、冻结工作流、分析输入及数据集血缘引用；
确认后资源立即从普通读取隐藏、使未确认配置失效、清空 tombstone 行的 artifact 外键并释放直接引用。冻结工作流和
已物化派生结果持有自己的引用或快照，因此删除不破坏历史；重复 DELETE 不重复释放。

## Managed study configurations

`search_study_facades` 只查询本次 engine snapshot 的有效门面，显式函数名优先，返回目录中的方法差异和禁用诊断；
`prepare_study` 持久化配置卡，不生成脚本或运行。状态为
`choosing_facade|collecting_inputs|ready_to_confirm|confirmed|invalidated|cancelled`，所有卡片写入都必须提交
`expectedRevision`。跨用户和不存在配置统一 404，revision、资源或 snapshot 竞争返回
`configuration_stale`。

模型输入只按门面声明从 active 版本的实际表名动态绑定；`para` 等 Struct 只绑定普通数据资源。兼容资源唯一时可
预选，多个兼容资源必须由用户选择；同门面的历史 Struct 只返回复用建议，不能静默沿用。Struct 卡由规范 schema
生成字段类型、单位、范围和枚举；修改已绑定资源时必须选择 Replace 或 SaveAsNew，没有来源时只生成新资源命令。
卡片保存只记录草稿和受控 replacement command，不提前覆盖资源。

Data 输入的 mapping 只接受源列到目标列的受控映射。服务端按目标 schema 转换为 Snappy Parquet，经统一上传发布
事务创建 `sourceKind=mapping` 的新普通资源，并冻结源资源 ID、目标 schema 和映射血缘；不会修改源资源。结果集合
必须有序、唯一、属于门面且满足 fixed/minimum/conditional 约束。当前没有公共 confirm 路由；Task 9 将以一个原子
事务重新校验配置、发布参数资源、冻结输入并启动 workflow，按钮固定为“确认并生成脚本”。

发布请求携带 `expectedDraftId`、`expectedDraftRevision`、`clientPublishId`。成功冻结版本、清空草稿并自动激活。
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
保留 schema、artifact、generator、PSA contract、study、source、文件声明与 inputs，不返回物理源路径或文件内容。
应用请求提交 `{exampleId, engineResourceSnapshot, clientBatchId}`（过渡期也接受字段别名 `clientImportId`）；后端只从
已发布快照读取 manifest 声明清单，全部哈希通过后进入统一上传用途确认。它不会直接生成草稿、版本或运行。

Agent 的 `search_examples` 使用相同搜索谓词并返回最多 50 个规范摘要；`inspect_example` 只按稳定 ID 返回规范
manifest、角色化文件哈希和验证诊断。两个工具只读，Agent 无权直接应用案例。

## Scripts and runs

上传脚本 multipart 只接受字段 `file` 的单个 UTF-8 `.rss`；`POST /api/sessions/{id}/scripts/text` 接受 JSON
`{name, content}` 直接以文本创建（`sourceKind: "manual"`）。两条路径只做名称、UTF-8 和大小校验后不可变落盘，
不使用 zjudss RSS 静态语法校验器；语法和运行可行性以用户点击运行后 tensoreval 的退出码、stderr 和结果为准。
脚本详情返回全文；列表和消息只返回有界预览。V4 受管研究的结构合同、确认和自动执行接口由 Tasks 9–11 补充；
当前过渡运行请求仍为 `{modelVersionId, clientRunId}`。Agent 没有普通运行、激活或删除工具。

运行创建响应、session 运行列表和单运行详情中的 `SimulationRun` 都包含必填 `scriptName`；它来自运行所绑定的
不可变脚本资源，不由前端按 ID 二次猜测。运行快照继续冻结模型、脚本 artifact/hash、engine contract、Agent bundle
和 skill refs；`GET /api/runs/{id}/datasets` 只列出该运行当前仍存活的全部结果数据集。失败时保留实际执行阶段；API 将内部
adapter/IO/校验错误映射为受控 `error.code`/`error.message`。adapter 对 stdout/stderr 分别持续排空、保留有界首尾并
记录完整 hash；tensoreval 以非零状态退出时保留实际退出码。原始有界 stderr 只供后续受限审计，普通 API 继续只返回脱敏摘要；没有该标记的
旧记录仍丢弃原始 message，因此不会返回历史调用栈或服务器路径。四个既有运行路由、
跨用户 404、显式按钮创建和运行不可删除的语义不变。

## Messages and errors

持久消息类型是 `user`、`assistant`、`tool-call`、`progress`、`result`、`attachment`、`model-status`、
`comparison`、`script`、`upload-batch`、`study-config`、`workflow-progress`。普通对话消息追加写入；后三种操作卡按
`sessionId + kind + operationId` 更新同一稳定消息 ID，内容变化时递增正整数 `revision`，相同终态重放保持原 revision。
上传卡携带数据库单调 `batchRevision`；仓储拒绝低于或等于已保存 batch/config revision、低于执行轮次的延迟状态，也不允许已经落库的终态被另一状态覆盖；GET upload/config
会幂等补写当前事实，以修复领域事务成功而消息写入中断的窗口。EventWriter 必须先提交数据库再广播。SSE `id` 为
`messageId:revision`，重连时同时按消息 ID 和 revision 去重；补发除游标后的普通消息外，还返回当前最新操作卡，即使该卡的
稳定消息 ID 早于游标，避免错过旧卡的新 revision。旧的纯数字 `Last-Event-ID` 仍按 revision 1 解释。卡片只携稳定 ID、
公共状态和脱敏诊断，不序列化服务器路径或原始 stderr；attempt
详情由前端默认折叠。运行关联字段统一为 `runId`。V4 对话 Agent 只持久化研究配置卡，不具备脚本创建权限；
后续模型收尾失败不追加通用 Agent 失败，缺少 assistant 收尾时由服务端补充“研究配置已准备”的提示。
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
以及引擎超时、结果监控/容量/路径失败。权威枚举由
`cargo run --bin contract-manifest` 产生，并由功能目录逐项覆盖。
