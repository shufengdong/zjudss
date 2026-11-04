# Engine Adapter and Resource Maintainability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 tensoreval 进程执行、结果目录安全、临时导入脚本、引擎资源扫描和不可变 snapshot 发布各有一个权威实现，同时保持全部公共合同和运行行为不变。

**Architecture:** adapter 内以单次最终扫描返回已验证文件清单，以 RAII 临时任务文件统一 import/projector 清理，并把普通执行与导入实现从门面模块拆开。engine resource 内按只读树、snapshot loader 和 immutable publisher 拆分；删除未使用的 binary feature 参数与 V3 兼容别名，但不建立跨边界的通用文件系统层。

**Tech Stack:** Rust 2024、Tokio process、Axum application startup、Arrow/Parquet 60、Snappy Parquet、SHA-256、临时目录与 Rust 集成测试。

**Spec:** `docs/superpowers/specs/2026-11-04-engine-adapter-resource-maintainability-design.md`

## Global Constraints

- 所有 tensoreval 进程调用继续只位于 `backend/src/adapters/`；application、handler 和 tool 不拼命令或启动进程。
- 生产运行继续显式使用 `--result-dir ... --output-format parquet`；输出事实来源只接受 Snappy Parquet。
- 浏览器和 LLM 不获得服务器路径、命令、catalog 内部路径或任意新增执行面。
- `SPARROWZZ_DIR` 保持只读；snapshot load 不写源树，publish 只写 `DATA_DIR/system/engine-resources`。
- 不改变 route、SSE、DTO、数据库 schema、Agent tool/skill、上传格式、view、catalog/manifest schema 或稳定错误码。
- 运行中 monitor 与退出后 final scan 必须保持两个独立时间点；成功不得复用最后一次 monitor tick 的结果。
- timeout、monitor failure 和 future cancellation 必须终止整个进程组，并清理部分结果和临时 task files。
- 资源 snapshot ID、文件 inventory、容量限制、案例隔离诊断和已发布目录完整性规则逐字节保持现状。
- 当前真实相邻 eig-rs 的 7 个 `case_projection_invalid` 是已知外部基线，不得通过放宽 zjudss 校验消除。
- 本阶段无公共合同变化，不编辑 `integration-tests/catalog/features.yaml`，除非测试符号确实重命名导致既有证据引用需要同步。

## Review Focus

- monitor 后、进程退出前写入的超限或非法文件仍必须被 final scan 拒绝；Task 1 的 `monitor_rejects_...final_scan_rechecks` 保持该行为。
- 最终结果目录中大小写不同的重复名、symlink、目录、非 Parquet、损坏/非 Snappy 文件必须保留原 code 且不返回部分 datasets；Task 1 的 output validation 测试覆盖。
- import 同时拥有 wrapper DSS 与 RSS，任一创建/执行/清理失败或 future cancellation 都不得遗留其中任何一个；Task 3 的 import cleanup 测试和新增 task-file 单元测试覆盖。
- resource scan 与 publish 之间源文件变化、已有 snapshot inventory 损坏和 symlink 路径都必须 fail closed；Task 4/6 的 engine resource 测试覆盖。
- 删除 resource feature 参数后，binary 的命名 Parquet/两个 import profile capability probe 仍必须在 snapshot load 之前独立失败；Task 5 的 bootstrap/probe 测试覆盖。

---

### Task 1: 让结果目录扫描成为单一安全所有者

**Files:**
- Create: `backend/src/adapters/process/result_scan.rs`
- Modify: `backend/src/adapters/process/mod.rs`
- Modify: `backend/src/adapters/process/monitor.rs`
- Modify: `backend/src/adapters/process/output.rs`
- Test: `backend/tests/process_engine_v4.rs`

**Interfaces:**
- Produces: `ScanPhase::{Monitoring, Final}` private enum。
- Produces: `ScannedResultFile { path: PathBuf, size: u64 }` and `ScannedResultDir { files: Vec<ScannedResultFile> }` private values。
- Produces: `scan_result_dir(path: &Path, limits: OutputLimits, phase: ScanPhase) -> Result<ScannedResultDir, AdapterError>`。
- Preserves: `validate_outputs(path, limits) -> Result<EngineRunOutput, AdapterError>` public signature and every existing stable code。

- [ ] **Step 1: 增强最终结果稳定顺序的 characterization test**

  在 `process_engine_v4.rs` 的 output validation 用例中以逆序创建 `zeta.parquet` 与 `alpha.parquet`，断言
  `validate_outputs(...).datasets` 按 `alpha,zeta` 返回；继续断言大小写重复名、symlink、目录、格式、损坏、压缩和三类限额的
  原错误 code。

- [ ] **Step 2: 运行 characterization 基线**

  Run: `cd backend && cargo test --features e2e-test --test process_engine_v4 output_validation_rejects_format_names_corruption_duplicates_and_limits`

  Expected: PASS before refactor；现有行为已经稳定排序，本步骤锁定公开行为而非私有遍历次数。

- [ ] **Step 3: 实现 `result_scan.rs` 并迁移 monitor**

  扫描一次 root 和直接子项；拒绝 root/file symlink、非普通文件和 canonical escape；用 checked-add 计算总量并返回排序后的
  canonical 文件清单。`Monitoring` 只对根目录/目录枚举的瞬时 IO 失败使用 `result_monitor_failed`；具体结构/限额 code 不变。

- [ ] **Step 4: 让 `validate_outputs` 消费已扫描清单**

  删除其第二套 `read_dir`、canonicalize、file type、size 和 total 逻辑；只在 `ScannedResultFile` 上验证扩展名、UTF-8 变量名、
  大小写不敏感唯一性、Parquet metadata 和 Snappy。空目录仍返回 `result_empty`。

- [ ] **Step 5: 运行结果目录与进程回归**

  Run: `cd backend && cargo test --features e2e-test --test process_engine_v4 output_validation_rejects_format_names_corruption_duplicates_and_limits && cargo test --features e2e-test --test process_engine_v4 live_result_monitor_enforces_file_count_file_size_and_total_size && cargo test --features e2e-test --test process_engine_v4 monitor_rejects_symlink_directory_and_monitor_failure_then_final_scan_rechecks`

  Expected: all selected tests PASS with unchanged codes and cleanup assertions.

- [ ] **Step 6: 静态门禁与提交**

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`

  ```bash
  git add backend/src/adapters/process backend/tests/process_engine_v4.rs
  git commit -m "refactor: centralize engine result scanning"
  ```

### Task 2: 分离普通进程执行职责

**Files:**
- Create: `backend/src/adapters/process/execution.rs`
- Modify: `backend/src/adapters/process/mod.rs`
- Modify: `backend/src/adapters/process/control.rs`
- Test: `backend/tests/process_engine_v4.rs`

**Interfaces:**
- Moves: `ProcessEngine::execute(spec, allowed) -> Result<EngineRunOutput, AdapterError>` to `execution.rs` without signature or behavior change。
- Moves: binary availability, exit diagnostic, captured stderr augmentation and result-dir prepare/cleanup helpers to `execution.rs`。
- Preserves: `control::run_command` owns spawn/streams/timeout/process group/monitor race；`ProcessEngine` public constructors and accessors stay in `mod.rs`。

- [ ] **Step 1: 运行执行边界 characterization tests**

  Run: `cd backend && cargo test --features e2e-test --test process_engine_v4 streams_over_one_mib_are_drained_bounded_and_hashed_without_utf8_assumptions && cargo test --features e2e-test --test process_engine_v4 nonzero_exit_keeps_actual_code_and_bounded_binary_stderr_without_partial_datasets && cargo test --features e2e-test --test process_engine_v4 timeout_and_future_cancellation_terminate_the_whole_process_group`

  Expected: PASS before refactor.

- [ ] **Step 2: 创建 `execution.rs` 并移动私有实现**

  保持 `ProcessEngine::execute` 对 `RunMode` 的入口匹配、zero timeout fallback、运行中 monitor、非零退出清理、final validation、
  stderr/exitCode diagnostics 和 success output 组装完全一致。`mod.rs` 只声明模块和保留配置/trait 委托。

- [ ] **Step 3: 运行完整 process adapter 套件**

  Run: `cd backend && cargo test --features e2e-test --test process_engine_v4`

  Expected: 17 current tests plus Task 1 additions PASS.

- [ ] **Step 4: 检查命令边界并提交**

  Run: `rg -n "Command::new|std::process::Command" backend/src -g '*.rs'`

  Expected: production tensoreval process construction remains only under `backend/src/adapters/`；test/support process calls are not production engine paths.

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`

  ```bash
  git add backend/src/adapters/process
  git commit -m "refactor: separate engine execution orchestration"
  ```

### Task 3: 统一 import/projector 临时任务文件并拆出导入实现

**Files:**
- Create: `backend/src/adapters/process/task_file.rs`
- Create: `backend/src/adapters/process/import.rs`
- Modify: `backend/src/adapters/process/mod.rs`
- Test: `backend/src/adapters/process/task_file.rs` (`#[cfg(test)]` module)
- Test: `backend/tests/process_engine_v4.rs`
- Test: `backend/tests/model_import_adapter.rs`

**Interfaces:**
- Produces: `TemporaryTaskFile::create(root: &Path, prefix: &str, extension: &str, bytes: &[u8], error_code: &'static str) -> Result<Self, AdapterError>`。
- Produces: `TemporaryTaskFile::relative_path(&self) -> &Path` and `TemporaryTaskFile::remove(self) -> Result<(), AdapterError>`。
- Produces: `ProcessEngine::import_model_process(spec)` and `ProcessEngine::project_case_process(spec)` private implementations consumed by `SimEngine` delegates。
- Removes: `RemoveFileGuard` and both hand-written nonce/open/write/drop cleanup sequences from `process/mod.rs`。

- [ ] **Step 1: 写出临时任务文件的失败单元测试**

  添加 `drop_removes_temporary_task_file`、`explicit_remove_is_idempotent_for_missing_file` 和
  `create_new_files_have_safe_unique_relative_names`。断言文件只在传入 root 内、名字由 prefix + lowercase hex nonce + extension
  构成、Drop/显式 remove 后不存在，用户预先创建的相似文件不被覆盖或删除。

- [ ] **Step 2: 运行测试并确认 RED**

  Run: `cd backend && cargo test --features e2e-test --lib adapters::process::task_file::tests`

  Expected: FAIL because `task_file` and `TemporaryTaskFile` do not exist.

- [ ] **Step 3: 实现最小 RAII 原语**

  使用 `getrandom::fill([u8;16])`、`hex::encode`、`OpenOptions::create_new(true)` 和完整 `write_all`；创建/写入失败使用调用方
  error code，写入失败先 best-effort 删除。Drop best-effort 删除；显式 remove 对 `NotFound` 视为成功，对其他 IO 返回同 code。

- [ ] **Step 4: 迁移并拆出模型 import 与案例 projection**

  `process/import.rs` 保留入口解析、wrapper 文本、RSS 文本、profile 输出过滤、精确输出排序和主错误/清理错误合并。
  模型 import 创建两个 `TemporaryTaskFile`，projector 创建一个；trait impl 只委托 private implementation。

- [ ] **Step 5: 扩充并运行生命周期回归**

  在现有 `process_case_projection_builds_a_controlled_script_and_enforces_exact_outputs` 增加 future cancellation 场景，等待临时 RSS
  出现后 abort，断言临时脚本和结果目录均消失。保留 import 的成功、非零退出、unsafe path、cancellation、并发和用户相似文件断言。

  Run: `cd backend && cargo test --features e2e-test --lib adapters::process::task_file::tests && cargo test --features e2e-test --test model_import_adapter && cargo test --features e2e-test --test process_engine_v4 process_engine_import_uses_input_root_wrapper_for_nested_entry && cargo test --features e2e-test --test process_engine_v4 process_case_projection_builds_a_controlled_script_and_enforces_exact_outputs`

  Expected: unit tests, 10 model import tests and both process lifecycle tests PASS.

- [ ] **Step 6: 静态门禁与提交**

  Run: `rg -n "zjudss-model-entry|zjudss-model-import|zjudss-case-projection|OpenOptions::new" backend/src/adapters/process`

  Expected: generated file naming/content remains in `process/import.rs`; exclusive create/write/remove algorithm occurs only in `task_file.rs`.

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`

  ```bash
  git add backend/src/adapters/process backend/tests/process_engine_v4.rs
  git commit -m "refactor: share engine task file lifecycle"
  ```

### Task 4: 提取 engine resource 只读树原语

**Files:**
- Create: `backend/src/engine_resources/tree.rs`
- Modify: `backend/src/engine_resources/mod.rs`
- Modify: `backend/src/engine_resources/examples.rs`
- Test: `backend/src/engine_resources/tree.rs` (`#[cfg(test)]` module)
- Test: `backend/tests/engine_resources.rs`

**Interfaces:**
- Produces: `safe_file` / `safe_directory` / `portable_relative` private tree path functions。
- Produces: `read_bounded_file` / `hash_file` / `hash_bounded_file_with_size` private bounded byte functions。
- Produces: `discover_documents` and `discover_bundle_manifests` returning deterministic sorted paths/specs。
- Preserves: `EngineResourceError` codes and `ResourceLimits` semantics；does not reuse adapter path/error types。

- [ ] **Step 1: 将现有 bounded read/hash tests 定位到新所有者并补充变化检测**

  迁移现有 sparse read 与 streamed hash 单元测试；增加 `safe_tree_paths_reject_symlink_components`（Unix）和
  `bundle_discovery_is_sorted_and_stops_at_bundle_leaf`。测试先引用 `tree` 新接口。

- [ ] **Step 2: 运行测试并确认 RED**

  Run: `cd backend && cargo test --features e2e-test --lib engine_resources::tree::tests`

  Expected: FAIL because `tree` module does not exist.

- [ ] **Step 3: 移动安全路径、walker、受限 IO 和 hash 实现**

  保持 symlink 政策：全局 docs 中出现 symlink 使 load 失败；bundle 分类遍历跳过非目录/symlink，只把直接包含 manifest 的
  `.rssbundle` 当叶节点；manifest 声明文件通过 `safe_file` fail closed。

- [ ] **Step 4: 迁移 `examples.rs` 消费者并删除旧实现**

  `examples.rs` 从 `tree` 导入 bounded read/hash/safe file；`mod.rs` 不再直接拥有 walker/path/hash 算法。

- [ ] **Step 5: 运行资源树聚焦回归**

  Run: `cd backend && cargo test --features e2e-test --lib engine_resources::tree::tests && cargo test --features e2e-test --test engine_resources discovers_only_manifest_backed_rssbundle_leaves && cargo test --features e2e-test --test engine_resources bundle_manifest_rejects_symlinked_files_without_blocking_other_cases && cargo test --features e2e-test --test engine_resources all_resource_limits_fail_with_one_stable_code && cargo test --features e2e-test --test engine_resources rejects_symlinks_anywhere_in_global_document_tree`

  Expected: all selected tests PASS with unchanged codes.

- [ ] **Step 6: 静态门禁与提交**

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`

  ```bash
  git add backend/src/engine_resources
  git commit -m "refactor: isolate engine resource tree safety"
  ```

### Task 5: 分离 snapshot loader 并删除虚假 capability 耦合

**Files:**
- Create: `backend/src/engine_resources/loader.rs`
- Modify: `backend/src/engine_resources/mod.rs`
- Modify: `backend/src/engine_resources/snapshot.rs`
- Modify: `backend/src/adapters/contract.rs`
- Modify: `backend/src/main.rs`
- Modify: `backend/src/testing/engine_fixture.rs`
- Modify: `backend/tests/engine_resources.rs`
- Modify: `backend/tests/release_engine_cases.rs`
- Modify: `backend/tests/process_engine_v4.rs`
- Modify: `backend/tests/script_validation_adapter.rs`

**Interfaces:**
- Produces: `loader::load_snapshot(sparrowzz_dir: &Path, limits: ResourceLimits) -> Result<EngineResourceSnapshot, EngineResourceError>`。
- Changes internal library API: `EngineResourceRegistry::load(path)` and `load_with_limits(path, limits)` no longer accept `HashSet<String>`。
- Removes: `EngineCapabilities::resource_features()` and every synthetic resource feature set。
- Preserves: `EngineCapabilities::probe/require_v3`, startup order and all capability stable errors。

- [ ] **Step 1: 更新 capability 与 registry tests 使旧接口编译失败**

  把 `script_validation_adapter` 改为只断言 `require_v3` 的成功/缺失 profile；所有 resource fixture 调用改成无 feature 参数的
  `load` / `load_with_limits`。不要新增“feature enum”替代测试。

- [ ] **Step 2: 运行 compile check 并确认 RED**

  Run: `cd backend && cargo test --features e2e-test --test script_validation_adapter --test engine_resources --no-run`

  Expected: compile failure because old signatures/method still exist.

- [ ] **Step 3: 实现 loader 并保持 identity 字节顺序**

  从 `mod.rs` 移动 catalog read/JSON/version/shape、registry 构造、docs/examples、source hashes 和 `identity_part` 序列；
  `study_facades.json` 缺失仍使用相同 synthetic bytes 但不进入 source inventory。snapshot ID format 和所有字段不变。

- [ ] **Step 4: 删除 resource feature 参数及方法并迁移启动/发布测试**

  `main` 在 `EngineConfig::probe` 成功后直接 load resources；release test 同样先 probe、后 load，不把能力集合传入 loader。
  测试 fixture 直接 load minimal tree。保持 `EngineConfig::probe` 位于 resource load 之前。

- [ ] **Step 5: 运行 loader、bootstrap 和 capability 回归**

  Run: `cd backend && cargo test --features e2e-test --test script_validation_adapter --test app_bootstrap && cargo test --features e2e-test --test process_engine_v4 capability_probe_exercises_each_model_import_profile && cargo test --features e2e-test --test engine_resources -- --skip current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count`

  Expected: capability tests and 32 deterministic engine resource tests PASS.

- [ ] **Step 6: 搜索门禁、静态门禁和提交**

  Run: `rg -n "resource_features|load_with_limits\([^)]*HashSet|EngineResourceRegistry::load\([^)]*," backend -g '*.rs'`

  Expected: no matches.

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`

  ```bash
  git add backend/src/engine_resources backend/src/adapters/contract.rs backend/src/main.rs backend/src/testing/engine_fixture.rs backend/tests
  git commit -m "refactor: decouple engine resource loading"
  ```

### Task 6: 分离 immutable snapshot publisher

**Files:**
- Create: `backend/src/engine_resources/publisher.rs`
- Modify: `backend/src/engine_resources/mod.rs`
- Modify: `backend/src/engine_resources/loader.rs`
- Test: `backend/tests/engine_resources.rs`

**Interfaces:**
- Produces: `publisher::publish_snapshot(snapshot: EngineResourceSnapshot, data_dir: &Path) -> Result<(EngineResourceSnapshot, PathBuf), EngineResourceError>`。
- Preserves: `EngineResourceRegistry::publish(snapshot, data_dir)` public wrapper and return type。
- Owns: generated catalog/docs indexes, source inventory, verified copy, staging cleanup, atomic rename, existing snapshot verification and path rebase。

- [ ] **Step 1: 增强 publisher characterization assertions**

  在 `publish_freezes_only_manifest_declared_case_sources_and_rebases_the_snapshot` 断言 manifest 的 `snapshotId`、
  `engineContractVersion`、catalog versions 和 `files` inventory 精确包含 generated + declared source files；继续断言未声明 RSS 不发布。

- [ ] **Step 2: 运行 publisher 基线**

  Run: `cd backend && cargo test --features e2e-test --test engine_resources publish_freezes_only_manifest_declared_case_sources_and_rebases_the_snapshot && cargo test --features e2e-test --test engine_resources publish_rejects_source_changes_and_corrupt_existing_snapshots`

  Expected: PASS before refactor.

- [ ] **Step 3: 移动 immutable publication 实现**

  把 generated files、published path、source verified copy、manifest write/parse、inventory/hash verification、staging cleanup、rename 和
  rebase 移到 `publisher.rs`。loader 只需把 `source_hashes` 留在 snapshot；`mod.rs` wrapper 委托 publisher。

- [ ] **Step 4: 运行完整 deterministic engine resource 套件**

  Run: `cd backend && cargo test --features e2e-test --test engine_resources -- --skip current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count`

  Expected: 32 deterministic tests plus added assertions PASS.

- [ ] **Step 5: 静态门禁与提交**

  Run: `rg -n "std::fs::rename|manifest.json|engine_resource_hash_mismatch|rebase_snapshot" backend/src/engine_resources`

  Expected: publication/staging integrity implementation is owned by `publisher.rs`; example manifest parsing remains separately in `examples.rs`.

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`

  ```bash
  git add backend/src/engine_resources backend/tests/engine_resources.rs
  git commit -m "refactor: isolate engine snapshot publication"
  ```

### Task 7: 删除 V3/类型别名并固化模块所有权

**Files:**
- Modify: `backend/src/adapters/import.rs`
- Modify: `backend/src/adapters/mod.rs`
- Modify: `backend/src/adapters/process/output.rs`
- Modify: `backend/src/adapters/process/execution.rs`
- Modify: `backend/src/adapters/stub.rs`
- Modify: `backend/src/testing/scenario_engine.rs`
- Modify: `backend/tests/model_import_adapter.rs`
- Modify: `backend/tests/process_engine_v4.rs`
- Modify: `docs/architecture.md`
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/v4-deployment.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/development-status.md`

**Interfaces:**
- Removes: `STATIC_MODEL_TABLES` transitional alias；tests use `MODEL_TABLE_OUTPUTS` for 20 model frames and `STATIC_IMPORT_OUTPUTS` for 21 import outputs。
- Renames owner: define `pub struct EngineRunOutput` directly and remove `EngineExecution` + type alias layer；all consumers keep the current `EngineRunOutput` name。
- Documents: process/result/task-file and tree/loader/publisher ownership；binary probe and resource snapshot remain independent fail-closed gates。

- [ ] **Step 1: 迁移测试到 V4 权威常量并删除别名**

  使用 `MODEL_TABLE_OUTPUTS` 表达模型表，使用 `STATIC_IMPORT_OUTPUTS` 表达含 `para` 的 import 输出；删除注释和 re-export。

- [ ] **Step 2: 直接命名执行结果结构体**

  把 `EngineExecution` 结构体重命名为 `EngineRunOutput`，删除 type alias；所有字段、serde/Debug/Clone 行为和 trait signatures 不变。

- [ ] **Step 3: 运行 adapter/resource 聚焦全套**

  Run: `cd backend && cargo test --features e2e-test --test model_import_adapter --test process_engine_v4 --test script_validation_adapter --test app_bootstrap && cargo test --features e2e-test --test engine_resources -- --skip current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count`

  Expected: all deterministic adapter/resource tests PASS.

- [ ] **Step 4: 更新权威文档**

  `architecture` 记录模块所有权；`agent-prompt-skills-design` 只澄清资源 snapshot 不消费 binary feature 集合；`v4-deployment`
  保持 probe→load→publish 顺序和操作语义；`integration-testing-design` 记录单次 final scan、临时文件取消清理和 snapshot
  完整性证据；`development-status` 记录本阶段完成项与实际门禁，不复制完整设计。

- [ ] **Step 5: 运行全量后端门禁**

  Run: `cd backend && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --features e2e-test`

  Expected: fmt/Clippy PASS。若精确测试仍只在
  `current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count` 报告相邻树同一 7 个
  `case_projection_invalid`，保留输出并运行：

  `cargo test --features e2e-test -- --skip current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count`

  Expected: every remaining backend test PASS；任何新增失败必须修复。

- [ ] **Step 6: 运行前端、跨层与差异门禁**

  Run: `cd frontend && npm test -- --run && npm run build && npm run test:e2e`

  Expected: current Vitest suite、production build 和 31 Playwright journeys PASS。

  Run: `./scripts/check-functional-coverage --require-pr && git diff --check`

  Expected: 350/350 functionality、297/297 contracts、100% PR coverage；no whitespace errors.

- [ ] **Step 7: 请求独立代码审查**

  审查重点：final scan 是否仍在进程退出后独立执行；monitor/final code 是否漂移；临时 wrapper/RSS 在取消时是否可靠清理；
  capability probe 是否仍早于 resource load；snapshot identity/inventory/atomic publish 是否逐字节保持；是否误创建跨边界通用层。

- [ ] **Step 8: 提交文档**

  ```bash
  git add backend/src/adapters backend/src/testing/scenario_engine.rs backend/tests/model_import_adapter.rs backend/tests/process_engine_v4.rs docs/architecture.md docs/agent-prompt-skills-design.md docs/v4-deployment.md docs/integration-testing-design.md docs/development-status.md
  git commit -m "docs: record engine boundary ownership"
  ```

## Self-Review Result

- **Spec coverage:** Task 1 覆盖单一结果扫描；Task 2/3 覆盖执行与临时文件职责；Task 4–6 覆盖 tree/loader/publisher；
  Task 5/7 删除虚假 capability 和 V3/类型别名；Task 7 覆盖文档与全量门禁。
- **Step scan:** 新 task-file/tree 接口先有 RED 单元测试；纯行为保持的扫描、执行、loader/publisher 拆分先跑 GREEN
  characterization；每项迁移后有聚焦套件、静态门禁和独立提交。
- **Type consistency:** `ScannedResultDir` 只由 output/monitor 消费；`TemporaryTaskFile` 只由 process import 消费；resource
  loader/publisher 都返回现有 `EngineResourceSnapshot`；删除 features 后所有 load 调用采用统一新签名。
- **Review Focus:** final race、完整输出拒绝、双临时文件取消、source/inventory 损坏和 probe/load 顺序分别绑定到 Task 1、3、
  4/6、5 的具体测试。
- **Proportion:** 计划锁定接口、行为证据和模块边界，不抄写 process/tree/publisher 函数体；七个任务都可独立编译、测试、审查和回滚。
