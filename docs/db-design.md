# V4 数据库设计（实施中）

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

`upload_batches` 以 `(session_id, client_batch_id)` 幂等，保存总状态、model/resource 两个子发布状态以及重新上传时
受同会话所有权约束的 `replacement_resource_id`；
`upload_files` 保存服务端私有暂存位置和来源 hash，公共 DTO 不返回路径；`upload_candidates` 保存 sheet、规范
Parquet artifact、实际 schema/hash/行数、目录推断证据和用户用途决定。接收前在 immediate transaction 中增加
`storage_scopes.reserved_bytes`，并发批次不能超卖。Task 4 的 `ready_to_confirm` 仍未发布任何模型或资源，Task 5
将以同一批次执行两个可独立诊断的发布子事务。

## Search and recovery

会话标题、消息、上传摘要、脚本和运行摘要写入 FTS5 trigram 外部内容表。索引损坏只让搜索返回 503，不影响
会话和消息读取。启动恢复将未完成上传、工作流及 queued/running 运行收敛到各自明确状态，不重写已提交资源。
