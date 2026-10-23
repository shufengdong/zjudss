# V3 数据库设计

## Scope

SQLite 只支持从空库执行 `backend/migrations/00000000000000_initial`。模型和结果正文保存在按用户 storage
scope 引用的内容寻址 artifact；数据库保存所有权、状态、hash、schema、快照和血缘。

## Entity groups

- 身份：`users`、`auth_sessions`、`storage_scopes`、`artifacts`、`storage_scope_artifacts`、`system_metadata`。
- 对话：`sessions`、`messages`、`chat_search_documents`、`chat_search_fts`。
- 模型：`workspaces`、`workspace_drafts`、`draft_dataframes`、`model_imports`、
  `model_import_candidates`、`model_versions`、`model_dataframes`、`publish_receipts`。
- 脚本与运行：`script_resources`、`simulation_runs`、`result_datasets`。
- 派生：`result_analysis_runs`、`result_analysis_inputs`、`dataset_lineage_snapshots`、`dataset_views`、
  `run_comparisons`。
- 删除：`version_deletion_previews` 和版本/运行/结果的 tombstone 字段。

## Invariants

每个 session 只有一个 workspace，每个 workspace 最多一个 draft；草稿和版本内的 `table_name` 唯一。
发布版本的内容与 DataFrame 行由触发器禁止修改，同一 workspace 的存活 `content_hash` 唯一。active version
必须属于当前 workspace。

运行插入必须同时绑定同一 session 的模型版本和脚本，`(session_id, client_run_id)` 唯一；运行保存模型
content hash、脚本 artifact/hash、engine contract、Agent bundle/skills。结果只能有 run 或 analysis 一个生产者。

删除结果时 `artifact_id` 置空且同时写 `deleted_at_ms/tombstone_json`；存活结果则反之。派生结果的
`dataset_lineage_snapshots` 保存来源 ID、名称、schema/hash 和生产者摘要。`dataset_views.view_dataset_id` 指向
独立物化结果，因此来源 tombstone 不影响读取。

## Ownership and queries

所有 repository 查询显式接收 `UserId`，通过 session 连接验证所有权。错误 ID 与跨用户 ID 都返回
`RepoError::NotFound`。storage scope ID 只来自认证 principal，不接收客户端值。

## Artifact references

commit 在一个 immediate transaction 中检查配额、去重并增加引用；发布 blob 使用原子 rename。导入 apply、
失败或放弃会释放 staging 引用。版本/结果删除在元数据事务完成后幂等 release，并由 GC 清理零引用 blob。

## Search and recovery

会话标题、消息、模型导入、脚本和运行摘要写入 FTS5 trigram 外部内容表。索引损坏只让搜索返回 503，不影响
会话和消息读取。启动恢复将 processing 导入、queued/running 运行标记失败，不重写已提交资源。
