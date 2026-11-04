# Artifact、数据库与仓储维护性重构设计

## 1. 意图

这是全仓库维护性路线图的第二阶段，范围限定为 CAS artifact、storage scope、引用计数和直接使用这些原语的
repository 事务。目标是在不改变公共 API、V4 空库 schema、稳定错误码或用户行为的前提下，让 artifact 提交、持有、
释放和配额计费各有一个权威实现。

本阶段不借机重写上传、模型、工作流或结果生命周期。各领域仓储仍决定何时建立或释放逻辑引用、何时写 tombstone、
何时拒绝竞争；共享层只执行已经由领域事务决定的引用增减。

## 2. 当前证据

- `backend/src/db/repositories/artifact.rs` 同时承担单个/批量 CAS 提交、upload source 特例、引用释放和 GC 元数据，
  其中普通提交和 upload source 提交各自实现了一遍 hash 去重、配额检查、scope link 和 usage 更新。
- `version.rs`、`data_resource.rs`、`run.rs`、`managed_workflow.rs` 有多段直接更新
  `storage_scope_artifacts.ref_count` 的 SQL；它们表达同一“已有 scope link 增加 N 个逻辑引用”的规则，但对缺失 link、
  更新行数和重复 artifact ID 的处理并不统一。
- `release_refs` 已经集中处理重复 ID、下溢、唯一 artifact/bytes 计费和 link 删除，是可保留并与 retain 对称放置的基础。
- `ArtifactService::commit`、`commit_upload_source` 和图表物化重复把 `StagedArtifact + ArtifactMetadata` 转成
  `ArtifactDraft`。
- tombstone、上传 reservation、模型发布、会话删除、运行/结果删除和工作流终态虽都涉及 artifact，但事务锁、
  幂等键、状态条件和释放时机不同；它们不是可合并的重复。

## 3. 目标

- 只有一个函数实现 hash 去重、同 scope 引用复用、新 artifact 配额检查、CAS publish 和 usage 计费。
- 只有一个模块实现已有 artifact 引用的 retain/release 计数、重复 ID 聚合、上下溢和 scope link 校验。
- 用户到 storage scope 的查询在引用原语中只有一个实现；领域所有权查询继续显式携带 `UserId`。
- upload source 的幂等指针、批次状态和引用转移语义保持不变，但不再复制普通 artifact commit 算法。
- 所有改动由数据库行为测试覆盖：并发、重复 hash、多逻辑引用、配额回滚、缺失 link、删除/GC 和跨用户隔离。
- 本阶段结束时相关 repository 大文件减少基础设施代码，但不以移动行数作为成果。

## 4. 非目标

- 不修改 `backend/migrations/00000000000000_initial`、Diesel schema 或公共 DTO。
- 不创建通用 repository、通用事务 DSL、全局 tombstone service 或 artifact 外键注册表。
- 不把 `storage_scope_id` 暴露给浏览器、LLM 或新的公共 API。
- 不统一上传 reservation 与 committed artifact usage；两者仍是不同生命周期。
- 不改变 GC 的数据库行先认领、文件删除失败记账、启动清理孤儿对象的现有策略。
- 不拆分 `data_resource.rs`、`managed_workflow.rs` 等领域大文件；它们分别属于后续业务子系统阶段。

## 5. 方案

### 方案 A：只提取引用计数 helper

风险最低，但普通提交与 upload source 提交仍会各自维护 CAS/配额算法，无法满足单一所有者目标。

### 方案 B：引用原语加单一提交内核（采用）

新增 repository 私有 `artifact_refs` 模块，把 retain/release 与用户 scope 解析集中；`artifact.rs` 保留 CAS 提交/GC，
但 upload source 先验证领域条件，再调用同一个 `commit_draft_in_transaction`。应用层增加一个从 staged artifact 生成
draft 的小 helper。它覆盖已有真实重复，同时不合并领域事务。

### 方案 C：schema 驱动的通用 artifact 引用表/触发器

可以让数据库自动推导所有引用，但需要把几十个领域外键统一注册或改写 schema，且 tombstone、nullable 引用和逐引用
转移语义差异很大。当前没有证据证明收益足以承担风险，因此不采用。

## 6. 权威模块与接口

### 6.1 `backend/src/db/repositories/artifact_refs.rs`

该模块仅在 repository crate 内可见，拥有以下同步事务原语：

- `storage_scope_id_for_user(conn, user: &UserId) -> RepoResult<i32>`：从认证用户解析可信 scope；不存在统一返回
  `RepoError::NotFound`。
- `retain_refs(conn, scope_id, artifact_ids) -> RepoResult<()>`：聚合重复 artifact ID，并为每个已有 scope link 增加
  对应逻辑引用数。
- `retain_ref_count(conn, scope_id, artifact_id, count) -> RepoResult<()>`：用于已有计数的批量转移；`count <= 0`、
  refcount 溢出或 link 缺失时失败关闭。
- `release_refs(conn, scope_id, artifact_ids, now_ms) -> RepoResult<()>`：迁入现有实现，继续按逻辑引用次数结算，只有
  scope 的最后一个引用移除时才减少 `used_artifacts/used_bytes`。

retain 不增加 `used_artifacts/used_bytes`，因为它只允许操作已经属于该 scope 的 artifact；首次建立 scope link 和唯一
artifact 计费仍只属于 commit 内核。

### 6.2 `backend/src/db/repositories/artifact.rs`

`commit_draft_in_transaction` 是唯一 CAS commit 内核，负责：

1. 读取 scope 和同 hash artifact；
2. 校验同 hash 大小一致；
3. 已有 scope link 时 publish/dedupe 并 retain 一次；
4. 新 scope link 时检查 artifact 数量和字节配额；
5. publish 到规范相对路径，插入必要的 artifact/link，并更新唯一 usage。

`commit_with_publish`、批量结果发布、图表发布和 upload source 最终都调用该内核。upload source 在调用前仍验证：文件属于
同 scope 的 live session、批次处于允许状态、已保存 source 指针与 staged hash/size 一致。已有 source 指针重放直接返回，
不会增加引用；首次提交调用内核后原子写入 `upload_files.source_artifact_id`。

GC、失败重试记录和物理 object 扫描继续留在 `artifact.rs`，因为它们与 CAS 元数据发布属于同一职责。

### 6.3 `backend/src/application/artifacts.rs`

`ArtifactMetadata` 提供一个 crate 内 helper，把 `StagedArtifact` 和 `created_at_ms` 转成 `ArtifactDraft`，统一 size 转换、
规范 CAS 相对路径、format 和 media type。普通 commit、upload source 和物化图表使用该 helper；它不负责持久化或权限。

## 7. 调用方迁移

- `version.rs`：复制 active version 表到 draft 后，加载实际 artifact ID 列表并调用 `retain_refs`；保留 revision、workspace
  所有权和 draft 事务。
- `data_resource.rs`：结果提升、source artifact 多资源转移、active version 克隆等路径调用 retain 原语；保留 alias、
  replacement、receipt 和部分发布语义。
- `run.rs`：冻结 script/model/resource 输入后一次 retain 实际 ID 列表；缺失任何 link 继续使创建事务失败。
- `managed_workflow.rs`：冻结输入和 candidate/audit 引用使用 retain 原语；需要把缺失 artifact 映射为
  `ConfigurationStale` 的路径继续在领域仓储映射，不把该错误语义下沉。
- 其他只释放引用的仓储改为从 `artifact_refs` 导入 `release_refs`，不改变删除/tombstone 事务结构。

迁移完成后，除 `artifact_refs.rs` 和首次 scope link 创建外，repository 中不得直接更新
`storage_scope_artifacts.ref_count`。

## 8. 保留的独立实现

| 候选 | 决定 | 理由 |
|---|---|---|
| 普通与 upload source 的 CAS commit 主算法 | 抽象 | hash、配额、link、usage 和 publish 语义相同 |
| retain SQL | 抽象 | 多处只是在同 scope 已有 link 上增加逻辑引用 |
| release SQL | 复用并迁移 | 已有正确的重复 ID 与唯一 usage 结算语义 |
| staged artifact 到 draft 的构造 | 抽象 | 三个稳定消费者，字段和失败语义一致 |
| 用户到 storage scope 查询 | 复用 | 引用事务都从认证 `UserId` 得到同一可信 scope |
| upload reservation | 保留 | committed usage 之前的临时字节占用，生命周期不同 |
| 各领域 tombstone/删除事务 | 保留 | 锁、影响 token、busy consumer、血缘和释放集合不同 |
| 各领域 scoped 查询 | 保留 | join、状态条件、404 与 revision 语义不同 |
| GC 与物理文件操作 | 保留 | CAS 元数据和文件失败恢复的专属职责 |

## 9. 错误与原子性

- retain/release 必须运行在调用方现有 immediate transaction 内；任一 artifact 缺失、计数无效或溢出会回滚整个领域事务。
- `RepoError::NotFound` 继续用于不存在的用户、scope link 或跨 scope artifact；领域仓储可以像当前一样映射成 stale 或 404。
- `RepoError::QuotaExceeded` 只由首次 scope link commit 产生；已有 hash 的额外逻辑引用不重复计费。
- publish callback 仍在数据库 immediate transaction 内执行；数据库回滚后残留的 CAS 对象继续由既有 orphan GC 回收。
- release 不按 artifact ID 去重丢失次数，而是先计数再扣减；低于零、`used_bytes` 溢出或缺失 link 失败关闭。

## 10. 测试与验证

新增 repository 单元测试覆盖 retain/release 对称性、重复 ID、缺失 link、非法 count 和 overflow；现有行为测试继续覆盖：

- 同 hash 并发 commit 只计一个唯一 artifact，但保留两个逻辑引用；
- 不同 scope 各自计费且不能越权读取；
- quota 并发写不能超卖，失败事务不留下 usage/link；
- upload source 并发确认只建立一次原文件引用，多个资源逐引用转移；
- active model clone 按重复 DataFrame artifact 次数 retain；
- run/workflow 冻结输入、删除、tombstone 和 session cleanup 精确释放；
- 打开的文件句柄可在 release/GC 后完成读取，孤儿 CAS 可恢复清理。

本阶段无公共合同变化，因此不修改功能目录。聚焦门禁至少包含 `artifact_store`、`quota_concurrency`、`data_resources`、
`workspace_versions`、`managed_workflows`、`simulation_runs`、`result_deletion`、`version_deletion`、`session_deletion` 和
`v4_schema`；阶段末执行 AGENTS.md 全量确定性门禁。相邻 eig-rs 的 7 个已知外部案例继续单独报告。

## 11. 文档归属

- `docs/architecture.md`：记录 artifact commit/ref primitive 的模块所有权。
- `docs/db-design.md`：记录唯一计费、逐引用 retain/release 和领域事务边界。
- `docs/integration-testing-design.md`：记录引用计数、并发、回滚和 GC 的行为证据。
- `docs/development-status.md`：只记录本阶段完成项、删除的重复 SQL/算法和最近验证结果。

## 12. 完成标准

- 普通、批量、图表和 upload source 提交共用一个 commit 内核。
- repository 中只有 `artifact_refs.rs` 直接增减已有 link 的 `ref_count`。
- 所有领域调用方保持现有事务、用户作用域、稳定错误与 tombstone 语义。
- 无 migration、公共路由、DTO、Agent 工具或 UI 变化。
- 聚焦与全量门禁通过；唯一允许单独报告的失败仍是已知相邻 eig-rs 外部断言。
