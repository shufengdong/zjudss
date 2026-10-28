# V4 数据库设计

> 当前 initial migration 已切换为 V4 空库 schema，不迁移或兼容 V3 数据。数据资源、运行输入绑定、可空
> run→script message 锚点、运行/直接结果 tombstone 删除的完整产品要求见
> [`2026-10-26-independent-data-resources-design.md`](superpowers/specs/2026-10-26-independent-data-resources-design.md)；
> 不为 V3 数据库设计迁移或兼容层。

`engine_resource_snapshots` 保存四个独立 catalog 版本、组合 catalog hash、诊断/门面数量摘要和 `is_current`。
启动发布在同一事务中清除旧 current 标记并写入当前 snapshot；partial unique index 保证全库最多一个 current。
旧 snapshot 行不会在发布新 snapshot 时删除，脚本、工作流和运行的外键继续阻止仍有历史引用的 snapshot 被回收。

## Scope

SQLite 只支持从空库执行 `backend/migrations/00000000000000_initial`。模型和结果正文保存在按用户 storage
scope 引用的内容寻址 artifact；数据库保存所有权、状态、hash、schema、快照和血缘。

## Entity groups

- 身份：`users`、`auth_sessions`、`storage_scopes`、`artifacts`、`storage_scope_artifacts`、`system_metadata`、
  `engine_resource_snapshots`。
- 对话：`sessions`、`messages`、`chat_search_documents`、`chat_search_fts`。
- 上传与资源：`upload_batches`、`upload_files`、`upload_candidates`、`data_resources` 及资源替换/删除预览。
- 模型：`workspaces`、`workspace_drafts`、`draft_dataframes`、`model_versions`、`model_dataframes`、
  `publish_receipts`。
- 研究配置与工作流：`study_configurations`、三类研究输入子表、`managed_workflows`、
  `workflow_input_snapshots`、`workflow_attempts`、`retry_previews`。
- 脚本与运行：`script_resources`、`simulation_runs`、`result_datasets`、`run_deletion_previews`。
- 派生：`result_analysis_runs`、`result_analysis_inputs`、`dataset_lineage_snapshots`、`dataset_views`、
  `run_comparisons`。
- 删除：`version_deletion_previews`、`session_deletion_previews`、`run_deletion_previews` 和资源、运行、结果的
  tombstone 字段。

## Invariants

每个 session 只有一个 workspace，每个 workspace 最多一个 draft；草稿和版本内的 `table_name` 唯一。
发布版本的内容与 DataFrame 行由触发器禁止修改，同一 workspace 的存活 `content_hash` 唯一。active version
必须属于当前 workspace。

`model_versions.description` 是最多 200 个 Unicode 字符的冻结展示信息，不参与 `content_hash`。相同内容再次发布时
复用已有版本及其原描述，不能借重复发布改写描述；空描述由服务端根据规范 diff 生成确定性摘要。

`messages` 的普通对话记录保持追加写入；可更新操作卡以非空 `operation_id` 在同一 `session_id + kind` 下唯一，内容变化时在
原行递增 `revision`，相同终态重放不创建新行也不推进 revision。`upload_batches.revision` 由数据库在每次事实更新后单调递增；
写入根据 upload/config revision、workflow 执行/修正序号和状态顺序
拒绝延迟旧卡覆盖新卡；已经落库的终态不能被不同终态改写。消息搜索摘要由触发器随卡片更新。会话只有 `live` 时才允许
写入消息、工作区、上传、资源、研究配置、脚本、运行、分析和工作流；进入 `deleting` 后数据库触发器阻止绕过 application 层的
新领域写入。

运行必须绑定同一 session 的脚本，模型版本/内容 hash 只在该门面实际需要模型时冻结，
`(session_id, client_run_id)` 唯一；运行保存冻结输入、脚本 artifact/hash、engine contract、Agent bundle/skills。
脚本消息锚点只能由服务写入同一 session 的 script 消息且设置后不可更换。结果只能有 run 或 analysis 一个生产者。

删除结果时 `artifact_id` 置空且同时写 `deleted_at_ms/tombstone_json`；存活结果则反之。派生结果的
`dataset_lineage_snapshots` 保存来源 ID、名称、schema/hash 和生产者摘要。`dataset_views.view_dataset_id` 指向
独立物化结果，因此来源 tombstone 不影响读取。

## Ownership and queries

所有 repository 查询显式接收 `UserId`，通过 session 连接验证所有权。错误 ID 与跨用户 ID 都返回
`RepoError::NotFound`。storage scope ID 只来自认证 principal，不接收客户端值。

模型表只读查询从 `model_dataframes -> model_versions -> workspaces -> sessions` 或
`draft_dataframes -> workspace_drafts -> workspaces -> sessions` 验证所有权。草稿查询还比较调用方提交的
draft ID 和 revision：只有已确认属于当前用户的资源才可返回 revision 冲突，外部 ID 仍为 404。读取仅解析
现有 artifact，不更新 active 指针、draft revision、artifact 引用或任何业务表；所有权校验与打开文件在
ArtifactService 的 read gate 内完成，release/GC 使用 write gate，避免已校验读取与并发回收竞争。本功能不增加 migration。

## Artifact references

commit 在一个 immediate transaction 中检查配额、去重并增加引用；发布 blob 使用原子 rename。上传确认、
失败、放弃或过期会按仍待处理的子事务结算 staging reservation。版本、资源、运行和结果删除在元数据事务完成后
幂等 release，并由 GC 清理零引用 blob；历史冻结输入通过 artifact 引用或墓碑继续可审计。

所有 artifact 发布持有共享发布锁，结果批量发布和 GC 持有独占锁。受管运行把全部结果 artifact 引用、
`result_datasets`、run 与 workflow 终态放进同一个 immediate transaction；事务回滚会在独占区段删除本轮新写入对象。
启动和运行期 GC 同时扫描 `objects/`，回收进程崩溃留下且数据库中不存在的 CAS 对象。

`upload_batches` 以 `(session_id, client_batch_id)` 幂等，保存总状态、model/resource 两个子发布状态以及重新上传时
受同会话所有权约束的 `replacement_resource_id`；确认时保存可选 `confirm_operation_id` 和规范请求 payload hash，操作 ID
相同但请求不同会被拒绝。确认正在执行或批次已经全部确认时不能更换操作；一次确认收敛为非终态的
`ready_to_confirm|partially_confirmed` 后，可由新操作重试失败侧，已发布侧的决定和 receipt 保持冻结。候选决定写入、`confirmation_started` 和
`confirmation_in_progress` 在同一事务提交，确保确认输入与执行锁原子冻结；后者只在子事务执行期间作为互斥锁，汇总
成功或失败以及异常返回时释放，启动恢复会清除遗留锁。候选替换和用途更新也检查该锁。活动确认期间 DELETE 放弃返回冲突，到期清理延后，均不会
被持久幂等键永久阻塞；启动恢复只对 `confirmation_started` 的批次在判断过期前按已持久化的两个子事务状态执行一次 `finish_batch`，确保仅认领但尚未冻结输入的请求不会被误判，子事务已全部
提交的批次收敛为 `confirmed` 并结算 reservation；
`upload_files` 保存服务端私有暂存位置和来源 hash，公共 DTO 不返回路径；`upload_candidates` 保存 sheet、规范
Parquet artifact、实际 schema/hash/行数、目录推断证据和用户用途决定；`alias_explicit` 直接冻结 alias 是否由用户显式指定，重放不得根据已经更新的建议值重新推断。接收前在 immediate transaction 中增加
`storage_scopes.reserved_bytes`，并发批次不能超卖。`ready_to_confirm` 不包含已发布业务资源；确认后 model/resource
两个子事务独立原子提交，成功侧 receipt 不会在部分重试中重写。部分成功后只保留失败侧候选引用的暂存文件、来源
artifact 和规范 artifact；Skip 候选立即回收，成功侧立即结算 reservation，且候选不能从 Skip 或另一用途迁入已发布侧。
确认响应分别返回两个
可空 receipt，幂等重放复用已持久化内容。

`data_resources` 保存 opaque ID、会话唯一 normalized alias、可选 schemaRef、来源、schema/hash/行数、不可变
normalized/source artifact 和 replacement/lineage。`live|needs_rename` 参与当前 alias 唯一索引；`replaced` 只供
历史血缘按 ID 读取；`deleted` tombstone 的两个 artifact 外键必须为空。删除事务先使未确认研究配置失效，再清空
直接 artifact 外键；冻结工作流、分析输入和已物化输出各自保存 artifact 引用或完整 schema/content hash 快照。
同一上传文件产生的多个资源共享 source artifact，并按资源引用数计数；发布事务把 `upload_files` 持有的原文件引用
转移给首个资源，额外资源增加引用，随后清空文件指针。这样会话删除只按实际持有者释放一次；同一 storage scope 的
其他会话即使复用相同 hash 也仍可读取。原文件 CAS commit 与 `upload_files` 指针写入在同一 immediate transaction
中完成，并发 confirm 重放只获得同一引用。候选/文件/preview 所有权指针的清理与逻辑 refcount 递减也在同一事务中
逐引用结算，不按 artifact ID 去重；进程退出或 repository 失败会整体回滚。只有最后一个引用删除后才允许 GC。

`data_resource_replacement_previews` 保存尚未发布的规范 artifact、待用资源 ID、Struct schema/values、expected hash、
影响摘要和过期 token；preview 本身不写 `data_resources`。确认才建立新资源并执行 Replace/SaveAsNew，Cancel 释放
待用 artifact；同一用户、同一资源的新 preview 会删除并释放被取代 preview 的 artifact；服务启动及每分钟后台任务
批量删除过期 preview 并按用户 storage scope 释放 artifact，过期确认也会同步回收。重新上传的 Replace
与参数卡 Replace 都使引用旧资源的未确认研究配置失效，并在新资源 lineage 中记录 action、源资源 ID、被替换资源 ID
及原 schema/content hash。`result_analysis_inputs` 和 `dataset_lineage_snapshots` 使用 dataset/resource 二选一来源，始终保存
来源 schema hash 与 content hash，源资源后续替换或删除不会改变既有派生血缘。

`study_configurations` 保存门面 ID/hash、engine snapshot、状态、单调 revision、原始意图和可选来源 workflow；
`study_input_bindings` 只保存门面声明的 model table 或普通资源 ID，Struct 草稿的 baseline/replacement action 和 Data
mapping 摘要放在受控 `mapping_json`；`study_struct_values` 保存按规范 schema 校验后的字段；
`study_result_selections` 以 ordinal 保存有序唯一结果。仓储每次查询都通过 session 校验用户，编辑使用 immediate
transaction 比较 revision 并整体替换三个子表。资源替换/删除会把引用它的非终态配置置为 `invalidated`；已确认
工作流只读自己的冻结 input snapshot。

列映射先按目标 schema 在应用层生成 Snappy Parquet，再通过统一上传候选事务发布新 `data_resources` 行；发布时即
写入 `source_kind=mapping` 和源资源/目标 schema/列映射血缘，避免发布后修改不可变 payload。配置确认事务重新校验
revision/snapshot/hash，在同一事务内发布 Struct replacement、确认配置、新增 workflow/provisional run、逐项冻结输入
artifact 并建立持久进度消息。`client_operation_id` 在会话内唯一，确保重复确认不重复工作；partial unique index 保证
每会话只有一个非终态 workflow。

首次受管 run 在脚本尚未产生时允许脚本资源/artifact/hash 三列同时为空；该例外只适用于 `run_kind=managed`。一旦候选
脚本成为用户可见终态，三列、同会话 script 消息锚点和 workflow 最终引用原子写入，数据库触发器禁止清空或改换锚点。
`workflow_attempts` 保存不可变候选 artifact、父 attempt 和脱敏诊断。workflow 输入、candidate、最终 script 与 run 分别
持有 artifact 引用；会话删除逐引用释放。正常完成、用户取消和启动恢复共用终态事务：存在候选时均发布最后一个候选为
不可变 script/message/run 锚点，确实尚无候选时才允许终态 managed run 保持空脚本三元组。启动恢复把非终态 workflow、candidate/executing attempt 和 provisional run
收敛到明确失败态。

Task 10 执行时，generation candidate 不占修正预算，每个 repair candidate 在持久化时递增
`managed_workflows.corrections_used`。attempt 依次记录 `candidate/contract_rejected/executing/engine_failed/result_rejected/succeeded`
状态、实际退出码、脱敏 stderr、完整/保留/发送 hash 与字节数；原始有界 stderr 只由
`stderr_audit_artifact_id` 引用，不进入公共 payload。成功终态事务同时插入全部 `result_datasets` 并绑定最终 script/message/run；
失败、取消或事务回滚不插入部分结果，私有 result-dir 被清理。

普通运行的 `input_snapshot_json` 保存实际使用的可空 active 模型版本/hash/表 artifact 以及 Data resource
ID/alias/schema/hash；创建事务为脚本和全部输入 artifact 增加独立引用，因此资源覆盖或删除不会改写既有 run。普通运行
结果也使用 publication 独占锁，在一个事务内提交全部 artifact 引用、`result_datasets` 和 run 成功终态；失败时在锁内
回滚本次新增 CAS 文件。`run_deletion_previews` 保存短期 token hash 和完整影响 hash；确认删除复核终态、直接结果集合与
busy consumer，释放脚本、输入和直接结果引用后写 run/result 墓碑，派生和提升资源保留冻结生产者/schema/hash 血缘。

`retry_previews` 以来源 workflow 保存当前 active/资源差异的影响 hash。输入未变化时直接创建
`source_workflow_id` 指向来源的派生 workflow；输入变化时确认事务重新计算影响并冻结最新 artifact。派生脚本通过
`source_script_resource_id` 指向原失败脚本，两个脚本及各自 run/message anchor 都保持不可变。

## Search and recovery

会话标题、消息中的可读摘要、上传摘要、脚本和运行摘要写入 FTS5 trigram 外部内容表；脚本全文、原始 stderr 和私有路径不进入
搜索正文。消息命中保留稳定 message ID，以便 API 在同一存活会话中验证目标后读取精确的连续 around window。索引损坏只让
搜索返回 503，不影响会话和消息读取。

`session_deletion_previews` 保存短期 token hash、影响摘要/hash 和过期时间；确认删除必须在 immediate transaction 中重新计算
当前影响；hash 同时计入可变草稿及其 DataFrame revision/artifact、未发布 upload batch、source/candidate artifact 和资源 replacement preview，不能以旧 token 绕过内容变化或新增占用。
会话先转为 `deleting` 并立即从普通查询隐藏，再由统一 cancellation token 停止 Agent/engine 工作并清理其 artifact
引用。清理中断时，同一预览 token 可恢复幂等清理，其他 token 仍被拒绝；nullable artifact 外键按实际非空引用释放，
不会因已 tombstone 资源而卡住删除状态。清理完成后删除业务记录，最终保留标题为“已删除会话”的最小 `deleted`
tombstone。重复删除不重复释放引用。启动恢复将未完成上传、
工作流及 queued/running 运行收敛到各自明确状态，不重写已提交资源。
