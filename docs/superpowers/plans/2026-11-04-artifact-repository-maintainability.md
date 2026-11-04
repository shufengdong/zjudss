# Artifact Repository Maintainability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 CAS artifact 提交、逻辑引用增减、storage scope 所有权和唯一配额计费各有一个权威实现，同时保持全部公共合同和领域事务语义不变。

**Architecture:** 新增 repository 私有 `artifact_refs` 模块，集中已有 scope link 的 retain/release 与用户 scope 解析；`artifact.rs` 保留 CAS commit/GC，并让所有提交路径调用同一个 `commit_draft_in_transaction`。上传、版本、资源、运行和工作流仓储只决定领域事务中的引用集合与错误映射，不再直接编写 refcount SQL。

**Tech Stack:** Rust 2024、Diesel 2 / SQLite immediate transactions、Tokio、内容寻址文件存储、Snappy Parquet、Rust 集成测试。

**Spec:** `docs/superpowers/specs/2026-11-04-artifact-repository-maintainability-design.md`

## Global Constraints

- 所有 tensoreval 进程调用继续只位于 `backend/src/adapters/`；本阶段不触碰命令执行边界。
- 仿真和派生结果继续以 Snappy Parquet 为事实来源，不恢复 CSV 或固定 metrics。
- 浏览器和 LLM 只使用稳定资源 ID 与受控 spec；不得暴露 storage scope ID、服务器路径、SQL 或任意 ECharts option。
- artifact、模型版本、脚本、结果、分析和图表发布后不可原地修改；tombstone 和血缘语义保持不变。
- repository 的领域查询继续显式接收 `UserId`；跨用户和不存在资源统一 404。
- `SPARROWZZ_DIR` 保持只读；不得为获得绿色测试放宽当前 eig-rs 资源校验。
- 本阶段不修改 initial migration、公共路由、DTO、消息、Agent 工具、skill、上传格式或 UI，因此不修改功能目录。
- 共享引用原语只操作调用方已经确定的 artifact 集合，不拥有上传、版本、资源、运行、工作流或删除状态机。

## Review Focus

- 同一 artifact ID 在一个模型/输入快照中出现多次时，retain/release 必须按逻辑出现次数结算，而不是按 ID 去重；Task 1 单元测试和 Task 3 `active_model_clone_counts_every_deduplicated_dataframe_reference` 固定该行为。
- retain 列表中先有合法 link、后有缺失 link 时，外层 immediate transaction 必须整体回滚；Task 1 增加批量原子性测试。
- upload source 已有指针的幂等重放不得再次 retain，首次多资源转移又必须逐引用计数；Task 2 运行并发确认与重复候选行为测试。
- workflow/run 冻结输入中的任一 artifact 不再属于用户 scope 时，不得留下部分 snapshot/run 或错误引用；Task 4 增加缺失 link 回滚测试并运行现有 stale/恢复测试。
- `ref_count` 的零/负 count、加法溢出和 release 下溢必须失败关闭且不改变 usage；Task 1 单元测试固定边界。

---

### Task 1: 建立对称的 artifact 引用原语

**Files:**
- Create: `backend/src/db/repositories/artifact_refs.rs`
- Modify: `backend/src/db/repositories/mod.rs`
- Modify: `backend/src/db/repositories/artifact.rs`
- Modify: `backend/src/db/repositories/case_switch.rs`
- Modify: `backend/src/db/repositories/chart_view.rs`
- Modify: `backend/src/db/repositories/data_resource.rs`
- Modify: `backend/src/db/repositories/run.rs`
- Modify: `backend/src/db/repositories/session.rs`
- Modify: `backend/src/db/repositories/upload.rs`

**Interfaces:**
- Produces: `storage_scope_id_for_user(conn: &mut SqliteConnection, user: &UserId) -> RepoResult<i32>`。
- Produces: `retain_refs(conn, scope_id, artifact_ids: impl IntoIterator<Item = i32>) -> RepoResult<()>`。
- Produces: `retain_ref_count(conn, scope_id, artifact_id, count: i64) -> RepoResult<()>`。
- Moves: `release_refs(conn, scope_id, artifact_ids, now_ms) -> RepoResult<()>` from `artifact.rs` to `artifact_refs.rs` without semantic changes。
- Preserves: unique artifact usage counters change only when a scope link is created or removed；retain only changes `ref_count`。

- [ ] **Step 1: 写出引用原语的失败单元测试**

  在 `artifact_refs.rs` 的测试模块建立内存 SQLite、执行当前 initial migration，并插入一个 scope、artifact 和
  `ref_count=1` link。添加：

  - `duplicate_retains_and_releases_preserve_unique_usage`：retain `[id,id]` 后 refcount 为 3、usage 不变；release 两次后回到 1。
  - `retain_batch_rolls_back_when_any_scope_link_is_missing`：在 outer immediate transaction 内 retain `[owned,missing]`，结果
    `RepoError::NotFound`，owned refcount 仍为 1。
  - `invalid_or_overflowing_reference_counts_fail_closed`：count 为 0/负数、`i64::MAX + 1` 和 release 下溢分别返回
    `RepoError::InvalidState`，link/usage 不变。
  - `user_scope_lookup_hides_missing_users`：存在用户返回其 scope；未知用户返回 `RepoError::NotFound`。

- [ ] **Step 2: 运行测试并确认 RED**

  Run: `cd backend && cargo test --features e2e-test --lib db::repositories::artifact_refs::tests`

  Expected: FAIL because `artifact_refs` and its functions do not exist.

- [ ] **Step 3: 实现 `artifact_refs.rs`**

  使用 `BTreeMap<i32, i64>` 聚合重复 ID；`retain_ref_count` 在 Rust 中 checked-add 后写精确值并要求 link 存在；
  `release_refs` 保留当前逐引用、唯一 usage 与 bytes 结算。所有函数都是 `pub(crate)`，不新增公共 repository 类型。

- [ ] **Step 4: 迁移现有 release 调用和 artifact commit 内的单次 retain**

  `artifact.rs` 从新模块导入原语；其他 repository 只改 import path，不调整各自 transaction、释放集合或错误映射。
  `commit_draft_in_transaction` 的已有 scope link 分支调用 `retain_ref_count(..., 1)`。

- [ ] **Step 5: 运行单元与 artifact 行为测试**

  Run: `cd backend && cargo test --features e2e-test --lib db::repositories::artifact_refs::tests && cargo test --features e2e-test --test artifact_store --test quota_concurrency`

  Expected: new unit tests and 10 existing artifact/quota tests PASS.

- [ ] **Step 6: 运行静态门禁并提交**

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`

  ```bash
  git add backend/src/db/repositories
  git commit -m "refactor: centralize artifact reference accounting"
  ```

### Task 2: 统一 CAS commit 内核和 staged draft 构造

**Files:**
- Modify: `backend/src/application/artifacts.rs`
- Modify: `backend/src/db/repositories/artifact.rs`
- Test: `backend/src/application/artifacts.rs` (`#[cfg(test)]` module)
- Test: `backend/tests/artifact_store.rs`
- Test: `backend/tests/data_resources.rs`

**Interfaces:**
- Consumes: Task 1 `retain_ref_count`。
- Produces: `ArtifactMetadata::into_draft(self, staged: &StagedArtifact, created_at_ms: i64) -> Result<ArtifactDraft, ArtifactError>`。
- Preserves: `ArtifactRepo::commit_with_publish`、`commit_upload_source_with_publish`、`commit_batch_with_publish` signatures。
- Preserves: upload source 已有指针时校验 hash/size 并直接重放，不增加引用；首次指针写入与 commit 在一个 immediate transaction。

- [ ] **Step 1: 添加 staged-to-draft 的失败测试**

  在 application unit tests 使用真实 `ArtifactStore::stage` 创建 staged bytes，调用 `ArtifactMetadata::into_draft`，断言
  sha256、size、`ArtifactStore::relative_path`、format、media type 和显式 timestamp。

- [ ] **Step 2: 运行测试并确认 RED**

  Run: `cd backend && cargo test --features e2e-test --lib application::artifacts::tests`

  Expected: FAIL because `draft_for` does not exist.

- [ ] **Step 3: 实现 draft helper 并迁移三个调用点**

  `commit`、`commit_upload_source` 和 `publish_chart_view` 均通过 helper 构造 `ArtifactDraft`；图表仍传入自己的
  `created_at_ms`，普通提交仍使用 `now_ms()`。

- [ ] **Step 4: 删除 upload source 中复制的普通 commit 算法**

  `commit_with_publish_for_upload` 先执行 upload-specific ownership/state/existing-pointer 校验；没有既有指针时调用
  `commit_draft_in_transaction`，随后写 `source_artifact_id`。publish callback 仍恰好消费一次。

- [ ] **Step 5: 运行提交、配额与 upload source 回归**

  Run: `cd backend && cargo test --features e2e-test --lib application::artifacts::tests && cargo test --features e2e-test --test artifact_store --test quota_concurrency --test data_resources`

  Expected: application unit tests plus 37 artifact/quota/data-resource tests PASS, including the three named upload reference cases.

- [ ] **Step 6: 检查单一 commit 算法并提交**

  Run: `rg -n "scope.used_artifacts|NewArtifact|StorageScopeArtifact" backend/src/db/repositories/artifact.rs`

  Expected: quota/new artifact/link/usage algorithm occurs only in `commit_draft_in_transaction`; upload wrapper contains only validation, idempotent replay and pointer update.

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`

  ```bash
  git add backend/src/application/artifacts.rs backend/src/db/repositories/artifact.rs
  git commit -m "refactor: share artifact commit kernel"
  ```

### Task 3: 迁移模型版本和数据资源引用持有

**Files:**
- Modify: `backend/src/db/repositories/version.rs`
- Modify: `backend/src/db/repositories/data_resource.rs`
- Test: `backend/tests/data_resources.rs`
- Test: `backend/tests/workspace_versions.rs`
- Test: `backend/tests/model_versions.rs`

**Interfaces:**
- Consumes: Task 1 `storage_scope_id_for_user`、`retain_refs`、`retain_ref_count`。
- Preserves: draft/version revision、model table copy、upload 双子事务、source artifact 转移、replacement/receipt/alias semantics。
- Removes: 两个 `model_dataframes` count subquery refcount SQL、结果提升单次 retain SQL、source additional refs SQL。

- [ ] **Step 1: 增强重复 DataFrame 引用的行为断言**

  在 `active_model_clone_counts_every_deduplicated_dataframe_reference` 中除现有 refcount 外，断言 clone 前后
  `used_artifacts/used_bytes` 不增长，并在释放 clone 后按逻辑次数回到原 refcount。若当前测试已具备等价断言，只补缺失项。

- [ ] **Step 2: 运行聚焦测试记录 GREEN 基线**

  Run: `cd backend && cargo test --features e2e-test --test data_resources active_model_clone_counts_every_deduplicated_dataframe_reference`

  Expected: PASS before refactor; this is characterization for behavior-preserving SQL removal.

- [ ] **Step 3: 迁移 `version.rs`**

  复制 active version rows 后加载 `model_dataframes.artifact_id` 列表，解析 user scope 并调用 `retain_refs`；保持同一
  immediate transaction 和 draft conflict 检查顺序。

- [ ] **Step 4: 迁移 `data_resource.rs`**

  结果提升用单 ID retain；source 多资源转移用 `retain_ref_count(additional_refs)`；active model clone 加载实际 ID 列表并
  `retain_refs`。所有缺失 link 统一失败并回滚领域事务。

- [ ] **Step 5: 运行模型/资源事务回归**

  Run: `cd backend && cargo test --features e2e-test --test data_resources --test workspace_versions --test model_versions --test case_import`

  Expected: 41 data-resource/workspace/model/case tests PASS with no changed receipt or quota assertions.

- [ ] **Step 6: 搜索门禁、静态门禁和提交**

  Run: `rg -n "storage_scope_artifacts SET ref_count|ref_count\.eq\(.*ref_count \+" backend/src/db/repositories/version.rs backend/src/db/repositories/data_resource.rs`

  Expected: no direct refcount increments remain.

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`

  ```bash
  git add backend/src/db/repositories/version.rs backend/src/db/repositories/data_resource.rs backend/tests/data_resources.rs
  git commit -m "refactor: reuse artifact retains in model resources"
  ```

### Task 4: 迁移运行和工作流冻结引用

**Files:**
- Modify: `backend/src/db/repositories/run.rs`
- Modify: `backend/src/db/repositories/managed_workflow.rs`
- Test: `backend/tests/managed_workflows.rs`
- Test: `backend/tests/simulation_runs.rs`
- Test: `backend/tests/workflow_retries.rs`

**Interfaces:**
- Consumes: Task 1 scope/retain primitives。
- Preserves: run client id replay、冻结 script/model/resource 输入、workflow configuration-stale mapping、attempt/audit/candidate refs、终态原子性。
- Removes: per-artifact Diesel update loop and two managed-workflow raw retain SQL helpers。

- [ ] **Step 1: 添加缺失 scope link 的事务回滚测试**

  在 `managed_workflows.rs` 增加 `confirmation_missing_owned_artifact_rolls_back_without_partial_workflow`：准备合法配置后，在确认
  前仅移除目标 artifact 的当前用户 scope link（保留 artifact row），确认返回配置 stale/失败，且 configuration 未确认、
  workflow/run/input snapshot 均未创建。测试直接操纵数据库只用于构造无法经公共 API 产生的损坏前置条件。

- [ ] **Step 2: 运行测试记录损坏前置条件的 GREEN 基线**

  Run: `cd backend && cargo test --features e2e-test --test managed_workflows confirmation_missing_owned_artifact_rolls_back_without_partial_workflow`

  Expected: PASS before refactor；现有 raw SQL 已有正确回滚语义，本测试防止迁移到共享 retain 后发生漂移。

- [ ] **Step 3: 迁移 `run.rs`**

  聚合 script、model frames 和 resource artifact IDs 后调用一次 `retain_refs`。保留 scoped run/resource 校验顺序、
  `RunRepoError` 映射和同一 immediate transaction。

- [ ] **Step 4: 迁移 `managed_workflow.rs`**

  workflow input snapshot 先解析其 session/user scope，再 retain 单个 artifact；`retain_artifact(user,id,count)` 改为组合
  `storage_scope_id_for_user + retain_ref_count`。缺失 link 在配置冻结路径继续映射为 `ConfigurationStale`，其他路径保持
  `Repository(NotFound)`。

- [ ] **Step 5: 运行运行/工作流回归**

  Run: `cd backend && cargo test --features e2e-test --test managed_workflows --test simulation_runs --test workflow_retries --test script_repair --test result_deletion --test version_deletion --test session_deletion`

  Expected: 58 workflow/run/deletion tests PASS, including the new corruption rollback case.

- [ ] **Step 6: 全 repository 搜索门禁、静态门禁和提交**

  Run: `rg -n "storage_scope_artifacts SET ref_count|ref_count\.eq\(.*ref_count [+-]" backend/src/db/repositories -g '*.rs'`

  Expected: existing-link arithmetic appears only in `artifact_refs.rs`; `artifact.rs` may insert a new link with `ref_count: 1` but must not update an existing link directly.

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`

  ```bash
  git add backend/src/db/repositories/run.rs backend/src/db/repositories/managed_workflow.rs backend/tests/managed_workflows.rs
  git commit -m "refactor: reuse artifact retains in execution snapshots"
  ```

### Task 5: 固化 Artifact/DB 模块所有权和阶段门禁

**Files:**
- Modify: `docs/architecture.md`
- Modify: `docs/db-design.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/development-status.md`

**Interfaces:**
- Documents: `artifact.rs` owns CAS commit/GC；`artifact_refs.rs` owns existing-link retain/release and trusted scope lookup；领域 repositories own transaction timing and tombstones。
- Preserves: current API/schema/Agent/frontend contracts and catalog counts；no functional catalog edit。

- [ ] **Step 1: 更新权威文档**

  在 architecture 添加模块所有权；db-design 把 commit、retain/release、usage 和领域 tombstone 边界写成唯一事实；
  integration-testing-design 记录重复 ID、missing link rollback、并发配额和 GC 证据；development-status 只记录本阶段完成项、
  删除的 raw SQL/重复 commit 算法和实际验证结果。

- [ ] **Step 2: 运行全量后端门禁**

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --features e2e-test`

  Expected: fmt/Clippy PASS。若精确测试仍只在
  `current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count` 报告相邻树同一 7 个
  `case_projection_invalid`，保留输出并运行：

  `cargo test --features e2e-test -- --skip current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count`

  Expected: every remaining backend test PASS；任何新增失败必须修复。

- [ ] **Step 3: 运行前端和跨层门禁**

  Run: `cd frontend && npm test -- --run && npm run build && npm run test:e2e`

  Expected: current Vitest count、production build 和 31 Playwright journeys PASS。

- [ ] **Step 4: 运行功能覆盖与差异门禁**

  Run: `./scripts/check-functional-coverage --require-pr`

  Expected: 350/350 functionality、297/297 contracts、100% PR coverage，且无 missing/duplicate/unknown contract。

  Run: `git diff --check`

  Expected: no whitespace errors.

- [ ] **Step 5: 请求独立代码审查**

  审查重点：commit 是否真有单一算法；retain 是否逐逻辑引用且不重复计费；缺失 link/overflow 是否原子回滚；领域 stale/404
  映射是否漂移；upload source replay 是否额外 retain；tombstone 与 GC 是否仍保持原边界。

- [ ] **Step 6: 提交文档**

  ```bash
  git add docs/architecture.md docs/db-design.md docs/integration-testing-design.md docs/development-status.md
  git commit -m "docs: record artifact repository ownership"
  ```

## Self-Review Result

- **Spec coverage:** Task 1 覆盖 retain/release/scope；Task 2 覆盖单一 commit 和 draft 构造；Task 3/4 迁移全部直接 retain
  消费者；Task 5 覆盖文档与全量门禁。migration、公共合同和未来业务子系统明确排除。
- **Step scan:** 新接口先有失败测试；已有正确事务行为先记录 characterization GREEN，再通过聚焦/全量事务测试证明无漂移；
  每个任务都有搜索门禁和独立提交。
- **Type consistency:** `artifact_refs` 的四个接口在 Task 1 定义，Task 2–4 只消费这些名称；领域错误映射留在调用方。
- **Review Focus:** 重复 ID、部分缺失、upload replay、workflow/run 原子回滚和计数边界分别绑定到 Task 1–4 的具体测试。
- **Proportion:** 计划指定接口、测试和迁移边界，不抄写 Diesel 实现；五个任务各自可编译、可审查、可回滚。
