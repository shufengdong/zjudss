# V4 Independent Data Resources and Managed Studies Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 从空数据库交付 V4：把辅助 DataFrame 作为独立资源管理，以动态引擎门面驱动研究输入确认，在用户最终确认后自动生成、执行和有限修正 RSS 脚本，并允许用户确认删除终态运行及其直接结果。

**Architecture:** V4 用一个统一上传批次把来源文件解析为候选，再按用户确认拆成模型和本地资源两个独立原子发布事务；静态导入的 21 个输出在 adapter 边界固定拆为 20 张基础网络模型表和一个普通 `para` 数据资源，但模型版本按 engine snapshot 的 `inputKind=model` 声明接纳其他模型表，不设固定表清单。研究配置是持久化状态机，全部函数、输入、Struct 参数和结果合同来自本次 `$SPARROWZZ_DIR/dss/docs` snapshot；确认事务冻结首次执行输入。受管工作流把脚本生成、tensoreval 执行、结果合同校验和有限修正编排为可恢复审计链，成功脚本的再次“运行”则按当前 alias 绑定最新数据资源并只执行一次。

**Tech Stack:** Rust 2024、Axum 0.8、Diesel/SQLite、Tokio、rig/DeepSeek、Arrow/Parquet 60（Snappy）、calamine/csv、React 19、TypeScript 5.9、ECharts 6、Vitest 5、Playwright 1.64。

**Spec:** `docs/superpowers/specs/2026-10-26-independent-data-resources-design.md`

## Global Constraints

- V4 只支持空数据库初始化；直接替换 `00000000000000_initial`，不提供 V3 migration、双读、导入或兼容分支。
- 未被规格明确改变的认证、会话、模型、案例、历史、分析、比较、可视化、隔离和安全行为沿用 V3 当前合同。
- 所有 tensoreval 调用只经过 `backend/src/adapters/`；生产调用始终带私有 `--result-dir` 和 `--output-format parquet`。
- 数据资源、研究配置、运行和 Agent 工具只传稳定 ID 与受控 spec，不接收服务器路径、命令、SQL、任意 RSS 修正
  表达式或任意 ECharts option；专用手写/粘贴/上传 `.rss` 入口仍可接收用户脚本文本。
- 模型版本、数据资源正文/artifact、attempt 脚本、任务脚本、结果和分析发布后不可修改；资源的 display name、alias 和
  lifecycle 元数据只能通过受控事务更新，正文变化通过新资源、replacement 和血缘表达。
- 每个模型版本随发布冻结非空的单行简短描述；用户可选填写，缺省由后端按草稿差异确定性生成。描述不参与模型内容
  hash；相同内容去重或幂等重放返回已有版本及原描述，不能修改历史描述或只因描述不同创建重复版本。
- 静态模型导入的 `para` 永远不是模型表：该 profile 把 `bus` 至 `storage` 的 20 张基础网络表加入 model draft/version，把 `para` 发布为有普通 alias 的本地 Data 资源。model draft/version/active 还可包含 engine snapshot 声明为 `inputKind=model` 的其他表，表集合不固定；`para` 可由参数卡修改并通过 replacement 覆盖。
- 每个 repository 查询显式接收 `UserId`；跨用户与不存在资源统一 404。
- `$SPARROWZZ_DIR/dss/docs` 是 RSS、全部 DataFrame 研究门面、输入 schema、Struct 参数和结果合同的唯一权威来源；不扫描 eig-rs 源码，不内置竞争目录。
- 权威目录中的路径/URL/import 门面不得进入受管研究；单门面合同无效只禁用该门面并在对话显示诊断。
- 固定按钮仅用于规格允许的上传、确认卡、会话/历史、查看、下载、删除、运行、重新尝试和取消；模型、alias、研究、分析和比较仍以自然语言入口为主。
- 默认配置必须精确为：上传 50 MiB/500 MiB、归档 1000 项/2 GiB/压缩比 100、暂存 24 小时、用户 20,000 artifacts/20 GiB/80% 告警、结果 100 文件/512 MiB 单文件/2 GiB 总量、引擎 120000 ms、进程输出每流 1 MiB、修正 stderr 64 KiB、修正 3 次、模型传输重试 2 次。
- 公共 route、message、tool、skill、upload、view、error 的任何变化，必须在同一任务更新公共类型、manifest、功能目录和行为测试。
- Tasks 1–13 的最后一个验证步骤除任务自身命令外，都运行
  `cargo test --test contract_manifest --features e2e-test`（尚未建立后端合同的纯前端任务可省略）和
  `./scripts/check-functional-coverage --require-pr`；对应提交必须包含 `integration-tests/catalog/features.yaml`，
  Tasks 2–11 还必须包含 `backend/src/contract_manifest.rs` 与 `backend/tests/contract_manifest.rs`。Task 14 只补充跨层
  Playwright 证据和做总审计，不延后登记前面任务已经引入的公共合同。
- 每个任务在当前 `master` 上提交，只暂存该任务列出的文件，保留用户已有的其他未提交修改。

V4 新增或重新定义的稳定错误码在各任务中统一使用，不得由 handler 临时改名：

```text
upload_file_too_large, upload_batch_too_large, archive_limits_exceeded,
upload_batch_expired, excel_formula_value_missing, resource_alias_conflict,
resource_stale, run_busy, result_busy, deletion_impact_changed, storage_quota_exceeded,
configuration_stale, workflow_in_progress, model_schema_incompatible,
study_contract_changed, repair_contract_violated,
script_generation_model_unavailable, repair_model_unavailable,
engine_timeout, result_monitor_failed, result_contract_invalid,
result_file_limit, result_file_too_large, result_total_too_large,
no_result_dataset, service_restarted, cancelled_by_user
```

归属错误或不存在资源仍统一使用现行 404 `not_found`，不通过错误码泄露对象是否属于其他用户。

## Review Focus

- 混合上传在解析失败、配额竞争、确认与 TTL 到期竞争时，模型子事务和数据子事务各自原子；部分成功状态可恢复，
  暂存来源和预留字节按仍待处理的子事务只结算一次（Task 4、5 测试）。
- 新 snapshot 中单个门面缺字段、合同 hash 漂移或 alias 新增冲突时，其他门面和历史运行仍可用（Task 2、6、11 测试）。
- 删除任一资源或重新上传普通数据恰逢最终确认、执行、重新尝试时，冻结输入不漂移，未确认配置失效，未来运行只解析当前普通数据 alias（Task 5、6、11 测试）。
- 删除终态运行时 run 与全部直接结果原子隐藏，busy 消费阻止整批删除，完成的派生/提升资源通过墓碑继续可读且 artifact 引用正确释放（Task 11、13、14 测试）。
- 输出持续增长、符号链接、监控器失败、超时、大量 stdout/stderr 和取消竞态都能终止整个进程组且不发布部分结果（Task 3、10 测试）。
- 服务重启、SSE 重放、重复确认/取消/重试和模型提供方重试不会重复创建工作流、attempt、tensoreval 进程或结果（Task 9、10、13 测试）。

## External Engine-Documentation Prerequisite

2026-10-27 重新检查 eig-rs `385a98b48` / sparrowzz `671aabd`，外部文档前置条件已经满足：catalog validator 报告
30 个 DSA 函数、15 个 DataFrame 研究门面、28 个 model、16 个 data、7 个 struct 和 85 个直接结果 schema（含继承基类
共 102 个），完整性诊断为零；28 个 catalog 单测全部通过。`dataframes.json` schema version 3 已发布完整
`dtype_grammar`、统一 `contract`、Struct `card`、机器规则、多形态表、`base_schema/baseMerge` 和完整结果 dtype；
`dss/docs/catalog/README.md` 明确区分执行字段与展示字段。Task 2 的真实依赖测试仍必须独立复核这些事实，不能把
eig-rs 的 Python validator 当作 zjudss 生产解析器。

三个目录文件拥有独立版本：`functions.json=2`、`study_facades.json=1`、`dataframes.json=3`，其中
`dtype_grammar.version=1`。这是当前有效合同，不是版本不一致。zjudss 必须分别校验每个文件及 dtype grammar 的支持版本，
不得继续要求三者的 `schema_version` 相等。当前 V3 `EngineResourceRegistry` 会以“不支持 catalog schema version 3”拒绝
最新目录；该已知失败是 Task 2 首个红灯测试和实现内容，不是新的 eig-rs 外部堵点。

zjudss 只读并验证 `study_facades.json` 及其传递 schema：文件缺失时发布“没有可用研究门面”的全局诊断；单 entry 或
传递 schema 无效时仅禁用受影响 entry；不会回退硬编码、扫描 `eig-rs/psa/src/dsa` 或修改 SPARROWZZ 工作树。

---

### Task 1: 建立 V4 空库 schema、运行配置和公共合同基线

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `backend/migrations/00000000000000_initial/up.sql`
- Modify: `backend/migrations/00000000000000_initial/down.sql`
- Modify: `backend/src/db/schema.rs`
- Modify: `backend/src/db/models.rs`
- Modify: `backend/src/db/repositories/version.rs`
- Modify: `backend/src/application/versions.rs`
- Modify: `backend/src/api/workspaces.rs`
- Modify: `backend/src/config.rs`
- Modify: `backend/.env.example`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/v3_schema.rs` → rename to `backend/tests/v4_schema.rs`
- Modify: `backend/tests/app_bootstrap.rs`
- Modify: `backend/tests/model_versions.rs`
- Modify: `backend/tests/contract_manifest.rs`

**Interfaces:**
- Consumes: V3 用户、会话、workspace、artifact、结果/分析/比较的行为合同。
- Produces: V4 全部 repository 使用的表、`RuntimeConfig` 限制字段、V4 contract manifest 版本。

- [ ] **Step 1: 写失败的 V4 schema/config 测试。** 新测试 `v4_initial_schema_contains_only_current_tables`、`one_live_data_alias_and_one_nonterminal_workflow_are_database_constraints`、`para_resources_cannot_join_model_versions`、`model_version_description_is_frozen_and_nonempty`、`model_version_description_defaults_from_canonical_diff`、`duplicate_model_content_keeps_original_description`、`resource_replacement_preserves_frozen_artifacts`、`run_and_result_tombstones_are_consistent`、`run_script_message_anchor_is_same_session_and_write_once`、`staging_reservations_cannot_be_negative`、`runtime_config_uses_v4_defaults_and_rejects_zero_or_inconsistent_limits` 应断言：旧库初始化不受支持；live Data alias 按 `(session_id, normalized_alias)` 唯一；所有资源统一为 Data，`para` 没有通向 draft/model dataframe 的外键；每个模型版本有非空不可变描述，用户描述按单行 200 Unicode 字符合同校验，缺省描述由实际草稿差异确定性生成且不调用 LLM；描述不进入模型内容 hash，相同内容去重和幂等重放返回原描述；覆盖只转移 alias 且不改写冻结 artifact；run 删除时所有直接结果必须同时 tombstone；run 的可空 script message anchor 只能由服务事务写入同会话 script 消息且一旦设置不可更换；每会话最多一个非终态 workflow；`used_bytes + reserved_bytes` 均非负；结果总量不小于单文件；所有规格默认值精确匹配。

- [ ] **Step 2: 运行 `cd backend && cargo test --test v4_schema --test app_bootstrap --test model_versions --test contract_manifest --features e2e-test`。** 预期因 V4 表、模型版本描述合同和配置字段不存在而失败。

- [ ] **Step 3: 用空库 V4 schema 替换 initial migration。** 保留认证、会话、artifact、workspace/model、消息、结果/分析/比较/历史实体；删除 V3 `model_imports` 的旧状态合同并建立以下新实体及外键/partial unique index：

  ```text
  engine_resource_snapshots
  upload_batches, upload_files, upload_candidates
  data_resources, data_resource_deletion_previews, data_resource_replacement_previews
  session_deletion_previews, run_deletion_previews
  study_configurations, study_input_bindings, study_struct_values, study_result_selections
  managed_workflows, workflow_input_snapshots, workflow_attempts, retry_previews
  script_resources, simulation_runs, result_datasets（按 V4 字段重建）
  ```

  `storage_scopes` 增加 `reserved_bytes`；`sessions` 增加删除中/删除完成终态；所有带用户内容的表可沿 session→user 验证归属。`model_versions` 增加不可空 `description`，它不参与 `content_hash`，也没有发布后的更新路径。`data_resources` 保存 opaque 公共 ID、live/deleted/replaced 状态、normalized alias、可选 declared schemaRef、来源种类、normalized/source artifact、schema/hash/row count、同批模型来源、replacement 和 lineage；所有资源均为 Data，不增加 `parameter_preset` discriminator。partial unique index 约束 live 资源的 `(session_id, normalized_alias)`。`simulation_runs` 的 model version/hash 改为可空，保存可空且只写一次的 `script_message_id`、tombstone/deleted 状态；`result_datasets` 保存 tombstone/deleted 状态；`run_deletion_previews` 保存短期用户/运行/影响集合绑定 token；repository 事务验证 message 与 run 同 session 且 kind=script。`managed_workflows` 用 partial unique index 限制每会话一个非终态实例。

- [ ] **Step 4: 补全模型版本发布合同。** `PublishDraftRequest` 增加可选 `description`；服务端归一为单行、校验最多 200 个 Unicode 字符并拒绝控制字符。空值依据 draft 与其基准版本的规范表差异生成确定性描述，首次发布使用表数量，后续发布使用新增/删除/修改表摘要；摘要按规范表顺序且使用确定性截断。版本列表、详情和发布响应都返回实际冻结描述。内容去重仍只由模型内容 hash 决定，命中已有版本或 publish receipt 重放时返回原描述和既有 `deduplicated/replayed`，不更新记录。repository 不提供描述更新方法。

- [ ] **Step 5: 扩充 `RuntimeConfig`。** 使用 `USER_ARTIFACT_LIMIT`、`USER_STORAGE_LIMIT_BYTES`、`AGENT_MAX_USER_RUNS` 分别替换 V3 `OWNER_ARTIFACT_LIMIT`、`OWNER_STORAGE_LIMIT_BYTES`、`AGENT_MAX_OWNER_RUNS`，不接受旧名；加入 `UPLOAD_STAGING_TTL_HOURS`、`RESULT_MAX_FILES`、`RESULT_MAX_FILE_BYTES`、`RESULT_MAX_TOTAL_BYTES`、`ENGINE_PROCESS_OUTPUT_MAX_BYTES`、`SCRIPT_REPAIR_STDERR_MAX_BYTES`、`SCRIPT_REPAIR_MAX_ATTEMPTS`、`AGENT_MODEL_RETRY_MAX`、`TENSOREVAL_TIMEOUT_MS`。继续校验 `STORAGE_WARNING_RATIO` 和既有 upload/archive 限制，并对所有正数、比例、单文件/总量关系执行启动校验。把 `.env.example` 和测试环境同步到 V4 名称。

- [ ] **Step 6: 将 manifest 基线切换为 V4。** manifest 仍从注册表生成 route/message/tool/skill/upload/view/error 枚举；本任务只建立 V4 版本和空集合变更入口，后续任务每次新增合同即同步测试。

- [ ] **Step 7: 运行聚焦测试和 `cd backend && cargo test --no-run --features e2e-test`。** 聚焦测试必须通过；全量只允许暴露后续任务尚未迁移的编译调用点，不得通过兼容字段消除它们。

- [ ] **Step 8: 提交。**

  ```bash
  git add backend/migrations backend/src/db backend/src/application/versions.rs backend/src/api/workspaces.rs backend/src/config.rs backend/.env.example backend/src/contract_manifest.rs backend/tests/v4_schema.rs backend/tests/app_bootstrap.rs backend/tests/model_versions.rs backend/tests/contract_manifest.rs integration-tests/catalog/features.yaml
  git commit -m "refactor: define v4 persistence contracts"
  ```

### Task 2: 从引擎 snapshot 发布全部可用 DataFrame 研究门面

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/contract_manifest.rs`
- Create: `backend/src/engine_resources/facades.rs`
- Modify: `backend/src/engine_resources/mod.rs`
- Modify: `backend/src/engine_resources/snapshot.rs`
- Modify: `backend/src/catalog/schema.rs`
- Modify: `backend/src/agent/prompt.rs`
- Modify: `backend/src/application/mod.rs`
- Replace: `backend/src/application/study_planning/contracts.rs`
- Replace: `backend/src/application/study_planning/policy.rs`
- Modify: `backend/src/application/study_planning/mod.rs`
- Modify: `backend/tests/engine_resources.rs`
- Modify: `backend/tests/agent_knowledge.rs`
- Modify: `backend/tests/common/mod.rs`
- Modify: `backend/tests/study_planning.rs`
- Modify fixtures under: `backend/tests/fixtures/engine_resources/`

**Interfaces:**
- Consumes: `EngineResourceSnapshot` 读取的 catalog/document 原始字节。
- Produces: `CatalogSchemaVersions`、`SchemaRegistry::resolve`、`FacadeRegistry::search`、`FacadeRegistry::get_enabled`、`DataFrameSchemaContract`、`FacadeContract`、`DisabledFacadeDiagnostic`，供上传兼容性、研究配置、脚本合同和重试使用。

- [ ] **Step 1: 写失败的动态目录测试。** 覆盖 `accepts_independent_supported_catalog_versions`、`rejects_each_unsupported_catalog_or_dtype_grammar_version`、`snapshot_hash_includes_study_facades_and_all_version_numbers`、`publishes_every_valid_dataframe_facade`、`excludes_path_url_and_import_facades`、`invalid_facade_is_disabled_without_disabling_neighbors`、`incomplete_transitive_schema_disables_only_affected_facades`、`prose_constraints_do_not_satisfy_struct_card_contract`、`base_schema_resolves_once_without_cycles`、`facade_hash_changes_with_transitive_executable_schema`、`facade_hash_ignores_presentation_only_text`、`explicit_newton_is_not_replaced_by_radial`。fixture 使用真实的 `functions=2/study_facades=1/dataframes=3/dtype_grammar=1` 组合；门面缺少 input kind、直接/传递 schema、完整字段 dtype、Struct card、结果 schema、推荐结果或组合约束时，只为受影响门面产生稳定诊断。

- [ ] **Step 2: 运行 `cd backend && cargo test --test engine_resources --test study_planning --features e2e-test` 并确认失败。**

- [ ] **Step 3: 定义动态门面类型。**

  ```rust
  pub struct CatalogSchemaVersions { pub functions: u32, pub study_facades: u32,
      pub dataframes: u32, pub dtype_grammar: u32 }
  pub enum DataFrameContractKind { Model, Struct, Data, Result }
  pub enum FacadeInputKind { Model, Struct, Data }
  pub struct SchemaFieldContract { pub name: String, pub dtype: DataTypeContract,
      pub required: bool, pub nullable: bool, pub unit: UnitContract,
      pub card: Option<StructCardContract> }
  pub struct StructCardContract { pub default: CardDefault, pub unit: CardUnit,
      pub range: CardRange, pub enum_values: CardEnum,
      pub visible_when: CardCondition, pub enabled_when: CardCondition }
  pub struct DataFrameSchemaContract { pub schema_ref: String, pub kind: DataFrameContractKind,
      pub row_count: RowCountConstraint, pub shape: FrameShape,
      pub column_policy: ColumnPolicy, pub fields: Vec<SchemaFieldContract>,
      pub stable_keys: Vec<String>, pub sort_keys: Vec<String>,
      pub column_aliases: BTreeMap<String, String>,
      pub allowed_conversions: Vec<UploadConversion>, pub rules: Vec<SchemaRule>,
      pub discriminator: Option<Discriminator>, pub variants: Vec<SchemaVariant>,
      pub schema_hash: String }
  pub struct FacadeInputContract { pub id: String, pub alias: String, pub kind: FacadeInputKind,
      pub schema_ref: String, pub required: bool, pub position: u32 }
  pub struct FacadeResultContract { pub id: String, pub name: String, pub schema_ref: String,
      pub description: String, pub recommended: bool }
  pub struct FacadeContract { pub id: String, pub function: String, pub family: String,
      pub title: String, pub description: String, pub differences: Vec<String>,
      pub inputs: Vec<FacadeInputContract>, pub results: Vec<FacadeResultContract>,
      pub result_constraints: Vec<ResultSelectionRule>, pub contract_hash: String }
  pub struct DisabledFacadeDiagnostic { pub facade_id: String, pub code: String, pub message: String }
  ```

  `EngineResourceSnapshot` 保存三个原始 JSON/bytes/hash 以及 `CatalogSchemaVersions`；Prompt 和内部诊断使用
  `catalogSchemaVersions` 对象，不再使用含义模糊的单个 `catalogSchemaVersion`。支持版本分别固定为 dataframes 3、
  functions 2、study facades 1、dtype grammar 1，不做相等比较。`study_facades.json` 必须参与 snapshot 身份、发布 inventory
  和历史重载。

  `SchemaRegistry` 严格按 eig-rs schema v3 和 dtype grammar v1 解析 primitive/constructor/semantic dtype、五种 rowCount、
  uniform/discriminated shape、open/closed/dynamic column policy、Struct card 的显式状态、README 登记的规则联合类型、
  discriminator/variant 和 `base_schema/baseMerge`；未知 enum、rule、condition、conversion 或 dtype 一律使受影响门面失效。
  `FacadeRegistry` 保存解析后的 schema 引用，供上传、参数卡和结果校验共用，不允许各层再次解释原始 JSON。

  `ResultSelectionRule` 是 internally tagged enum，覆盖当前目录的 `ordered_unique_selection`、`default_results`、
  `fixed_results` 和 `conditional_unavailable`；未知规则禁用受影响门面，不能把未验证的任意 JSON 传给 LLM 或前端。

  `contract_hash` 覆盖该门面的稳定执行字段，以及按引用闭包解析后的输入和结果完整 schema；同一 schema 变化只改变
  引用它的门面 hash。hash 还包含这些 schema 实际引用的 dtype/semantic type 和 conversion 定义；未被该门面引用的全局
  dtype 条目变化只改变 snapshot，不改变该门面 hash。title、description、differences、说明文字、排版和 source location
  等展示字段不进入 hash。registry 另行保留 snapshot ID 与展示元数据。缺少用户可解释差异、输入 schema、Struct 字段
  合同、结果 schema/推荐集合或组合约束时禁用该门面。路径/URL/import 参数门面即使结构合法也排除。

  registry 的唯一门面来源固定为 `dss/docs/catalog/study_facades.json`；`functions.json` 只继续提供 RSS 函数/表达式
  语法索引，不能补齐或覆盖门面合同。

- [ ] **Step 4: 把 `StudyPlanningService` 改成目录驱动搜索。** 移除五类研究硬编码函数和 arity 策略，提供 `search(query, explicit_function)` 与 `get(facade_id)`；显式函数唯一且有效时直接返回，多个候选返回完整差异，禁用项返回其诊断。模型事实只用于推荐说明，不得改写显式方法。

- [ ] **Step 5: 启动发布并持久化 snapshot 元数据。** 当前 snapshot 原子标记 `is_current`，持久化三个 catalog schema version 和 dtype grammar version；历史引用阻止旧 snapshot GC。更新资源树仍要求重启，不 checkout/pull/改写 eig-rs。

- [ ] **Step 6: 运行聚焦测试。** 同时执行 `cargo test --test catalog_inference --features e2e-test`，确认现有模型表推断未回退；再对实际 `$SPARROWZZ_DIR` 运行真实目录测试，要求 `enabled_count == catalog_entry_count`（本次审计基线为 15）且没有 schema 诊断。该断言不满足时 Task 2 保持未完成；产品代码和 fixture 不维护固定数量枚举。

- [ ] **Step 7: 提交。**

  ```bash
  git add backend/src/engine_resources backend/src/catalog backend/src/agent/prompt.rs backend/src/application/mod.rs backend/src/application/study_planning backend/tests/engine_resources.rs backend/tests/agent_knowledge.rs backend/tests/common/mod.rs backend/tests/study_planning.rs backend/tests/fixtures/engine_resources integration-tests/catalog/features.yaml backend/src/contract_manifest.rs backend/tests/contract_manifest.rs
  git commit -m "feat: publish dynamic study facade contracts"
  ```

### Task 3: 加固唯一的 tensoreval 进程执行边界

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/contract_manifest.rs`
- Create: `backend/src/adapters/process/capture.rs`
- Create: `backend/src/adapters/process/monitor.rs`
- Create: `backend/src/adapters/process/control.rs`
- Modify: `backend/src/adapters/process/mod.rs`
- Modify: `backend/src/adapters/process/output.rs`
- Modify: `backend/src/adapters/mod.rs`
- Modify: `backend/src/adapters/stub.rs`
- Modify: `backend/src/testing/scenario_engine.rs`
- Modify: `backend/tests/process_engine_v3.rs` → rename to `backend/tests/process_engine_v4.rs`

**Interfaces:**
- Consumes: Task 1 的进程、超时和结果限制配置。
- Produces: `EngineExecution`, `CapturedStream`, `EngineFailureDiagnostic` 和安全的 `EngineRunOutput`，供所有导入、分析和研究运行复用。

- [ ] **Step 1: 写失败的进程测试。** 使用测试 helper binary 覆盖每流超过 1 MiB 仍持续排空、首尾保留/hash/总字节数、非 UTF-8、非零退出码、120000 ms 配置超时、取消、子进程组、运行中超过文件数量/大小/总量、symlink/目录逃逸、监控器失败和退出后最终扫描；断言任何失败都不返回部分 dataset。

- [ ] **Step 2: 运行 `cd backend && cargo test --test process_engine_v4 --features e2e-test` 并确认当前 `command.output()` 和退出后扫描不能满足测试。**

- [ ] **Step 3: 定义统一执行结果。**

  ```rust
  pub struct CapturedStream { pub retained: Vec<u8>, pub full_sha256: String,
      pub retained_sha256: String, pub total_bytes: u64, pub truncated: bool }
  pub struct EngineExecution { pub exit_code: i32, pub stdout: CapturedStream,
      pub stderr: CapturedStream, pub datasets: Vec<EngineDatasetOutput> }
  pub struct EngineFailureDiagnostic { pub code: String, pub exit_code: Option<i32>,
      pub stderr: CapturedStream, pub details: serde_json::Value }
  ```

  用两个并发 reader 持续排空 pipe，每流保留等额首尾；流式计算完整 hash。用任务私有 result-dir 的有界轮询监控普通文件、数量和累计字节，不跟随 symlink；超限、取消、超时和监控失败终止整个进程组，再执行最终严格扫描与清理。

- [ ] **Step 4: 统一错误码。** 非零退出保留实际 exit code；超时为 `engine_timeout`；监控失败为 `result_monitor_failed`；限制分别为 `result_file_limit`、`result_file_too_large`、`result_total_too_large`；路径问题使用已有窄化安全码。adapter 不做 V4 RSS 静态语法校验或 binary hash 变化拦截，只验证当前路径存在且为普通可执行文件。

- [ ] **Step 5: 更新 Stub/ScenarioEngine。** 添加可排队的 stderr、合同错误、监控错误、取消 barrier 和多次执行 trace，使后续 workflow 测试不调用真实 binary。

- [ ] **Step 6: 运行聚焦测试及 `cargo test --test model_import_adapter --test script_validation_adapter --features e2e-test`。** 将旧静态脚本校验测试改为只覆盖仍需要的导入能力探测；V4 脚本执行不经该校验器。

- [ ] **Step 7: 提交。**

  ```bash
  git add backend/src/adapters backend/src/testing/scenario_engine.rs backend/tests/process_engine_v4.rs backend/tests/model_import_adapter.rs backend/tests/script_validation_adapter.rs integration-tests/catalog/features.yaml backend/src/contract_manifest.rs backend/tests/contract_manifest.rs
  git commit -m "feat: bound and monitor engine execution"
  ```

### Task 4: 建立可恢复的统一上传、解析设置和用途确认流水线

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/contract_manifest.rs`
- Create: `backend/src/application/uploads.rs`
- Create: `backend/src/db/repositories/upload.rs`
- Create: `backend/src/api/uploads.rs`
- Remove: `backend/src/api/model_imports.rs`
- Modify: `backend/src/ingestion/tabular.rs`
- Modify: `backend/src/ingestion/archive.rs`
- Modify: `backend/src/ingestion/bundle.rs`
- Modify: `backend/src/adapters/import.rs`
- Modify: `backend/src/adapters/process/mod.rs`
- Modify: `backend/src/adapters/stub.rs`
- Modify: `backend/src/application/model_import.rs`
- Modify: `backend/src/application/case_import.rs`
- Modify: `backend/src/api/examples.rs`
- Modify: `backend/src/api/mod.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/tests/model_imports.rs`
- Modify: `backend/tests/case_import.rs`
- Create: `backend/tests/upload_batches.rs`
- Modify: `backend/tests/quota_concurrency.rs`
- Add fixtures under: `integration-tests/fixtures/uploads/v4/`

**Interfaces:**
- Consumes: Task 1 upload tables/config、Task 2 model schema、Task 3 adapter import、现有 staging/CAS。
- Produces: `UploadBatchService` 和受控 `UploadBatchView`；Task 5 用确认后的 model/resource candidate 发布事务。

- [ ] **Step 1: 写失败的上传行为测试。** 测试名至少包括 `static_import_splits_twenty_model_tables_from_para_resource`、`case_import_never_puts_para_in_the_draft`、`catalog_model_schema_outside_network20_creates_model_candidate`、`csv_and_parquet_create_one_candidate_each`、`excel_visible_nonempty_sheets_default_selected_and_hidden_sheets_do_not`、`ambiguous_parse_requires_settings`、`formulas_macros_and_external_links_are_never_executed`、`missing_required_formula_cache_is_rejected`、`mixed_candidates_require_explicit_purpose`、`staging_reservation_survives_restart_and_expires_once`、`archive_and_batch_limits_leave_no_partial_batch`。断言列名/dtype 来自实际表头而非文件名；所有静态导入来源都得到恰好 20 个 model candidates 和一个固定进入资源子事务的普通 `para` Data candidate；与当前目录 `frames.fuse` 等额外 model schema 匹配的上传仍生成 ModelTable candidate。本任务只建立可恢复批次和候选，不提前发布模型或资源。

- [ ] **Step 2: 运行 `cd backend && cargo test --test upload_batches --test quota_concurrency --features e2e-test` 并确认失败。**

- [ ] **Step 3: 定义上传状态与请求。**

  ```rust
  pub enum UploadBatchStatus { Processing, AwaitingParseSettings, AwaitingPurpose,
      ReadyToConfirm, PartiallyConfirmed, Confirmed, Failed, Discarded, Expired }
  pub enum ChildPublishStatus { NotApplicable, Ready, Published, Failed }
  pub enum CandidatePurpose { ModelTable, DataResource, Skip }
  pub struct ParseSettings { pub delimiter: Option<u8>, pub encoding: String,
      pub header_row: u32, pub decimal: String, pub thousands: Option<String>,
      pub datetime_format: Option<String>, pub timezone: Option<String>,
      pub excel_range: Option<String> }
  pub struct CandidateDecision { pub candidate_id: String, pub purpose: CandidatePurpose,
      pub selected: bool, pub table_name: Option<String>, pub display_name: Option<String>,
      pub alias: Option<String> }
  ```

  流程为 `accept -> parse/infer -> await settings/purpose -> ready -> partially-confirmed|confirmed|failed|discarded|expired`。批次同时保存 model/resource 两个 `ChildPublishStatus`、各自 receipt/diagnostic 和仍待处理候选；部分成功只允许修正或重试失败子事务，已发布子事务不可重复执行。两个发布子事务都校验失败时总状态回到 `ReadyToConfirm` 并保留诊断；`Failed` 只表示来源损坏、暂存丢失等不可恢复终态，不允许再次 confirm。每个 CSV/Parquet 一个候选；Excel 每个非空 sheet 一个候选。所有 tabular candidate 只允许 ModelTable/DataResource/Skip；模型候选集合来自当前 engine snapshot 中有效门面的 `inputKind=model` schema，不维护固定模型表枚举。adapter 的 `import_dss` 静态 profile 仍按常量 `STATIC_IMPORT_OUTPUTS`（21 项）校验完整性，再拆成该 profile 的 `MODEL_TABLE_OUTPUTS`（`bus..storage` 20 项）与固定 `PARA_RESOURCE_OUTPUT="para"`；这两个常量不得用于限制其他上传或导入 profile 可发布的模型表。`para` 只允许 DataResource/Skip，默认 alias 为 `para`，并保存目录 schemaRef 与同批来源，但公共类型中没有专用参数资源 purpose。批次保存来源 hash、parser 版本、规范 parse spec、有限样例和诊断；任何公共 DTO 不含 staging path。公共 serde 状态使用 `snake_case`，因此 `PartiallyConfirmed` 出网为 `partially_confirmed`。

- [ ] **Step 4: 实现配额预留和 TTL。** 接收字节前事务增加 `reserved_bytes`；流式超限立即中止。确认、部分确认、放弃、24 小时到期和启动恢复通过来源 artifact/暂存所有权账本幂等结算：成功子事务需要的来源转为正式引用，只为失败子事务仍需的暂存保留 reservation；全部终态后释放其余预留。过期确认返回 `upload_batch_expired`。服务重启恢复合法候选和两个子状态，丢失暂存文件则只使仍未发布的子事务收敛为明确失败。

- [ ] **Step 5: 实现安全解析。** 支持受控编码/分隔符/表头/数字/日期/时区/Excel range；歧义进入卡片而非使用 locale 默认。calamine 只读缓存值，永不执行公式、宏、链接或网络；必填缓存缺失返回 `excel_formula_value_missing`，非必填写 null 并列出位置/数量。

- [ ] **Step 6: 暴露上传 API。**

  ```text
  POST   /api/sessions/{id}/upload-batches                 multipart, 202
  GET    /api/upload-batches/{id}
  PUT    /api/upload-batches/{id}/parse-settings
  PUT    /api/upload-batches/{id}/candidates
  DELETE /api/upload-batches/{id}
  POST   /api/data-resources/{id}/replacement-batches     multipart, 202
  ```

  案例应用也创建同一种 upload batch，并使用同一 20+1 拆分。旧 `/model-imports` 公共入口删除，不保留双协议。
  `POST /api/upload-batches/{id}/confirm` 延后到 Task 5 与真实双发布事务一起注册，避免本任务依赖尚不存在的 publisher
  或提供会返回占位错误的公共路由。

- [ ] **Step 7: 运行聚焦测试和 `cargo test --test case_import --test bundle_ingestion --test failure_atomicity --features e2e-test`。**

- [ ] **Step 8: 提交。**

  ```bash
  git add backend/src/application/uploads.rs backend/src/db/repositories/upload.rs backend/src/api/uploads.rs backend/src/api/model_imports.rs backend/src/ingestion backend/src/adapters/import.rs backend/src/adapters/process/mod.rs backend/src/adapters/stub.rs backend/src/application/model_import.rs backend/src/application/case_import.rs backend/src/api/examples.rs backend/src/api/mod.rs backend/src/application/mod.rs backend/tests integration-tests/fixtures/uploads/v4 integration-tests/catalog/features.yaml backend/src/contract_manifest.rs backend/tests/contract_manifest.rs
  git commit -m "feat: add recoverable v4 upload batches"
  ```

### Task 5: 发布、管理和覆盖普通数据资源

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/contract_manifest.rs`
- Create: `backend/src/application/data_resources.rs`
- Create: `backend/src/db/repositories/data_resource.rs`
- Create: `backend/src/api/data_resources.rs`
- Create: `backend/src/tools/list_data_resources.rs`
- Create: `backend/src/tools/read_data_resource.rs`
- Create: `backend/src/tools/update_data_resource_alias.rs`
- Create: `backend/src/tools/promote_result_dataset.rs`
- Modify: `backend/src/application/uploads.rs`
- Modify: `backend/src/api/uploads.rs`
- Modify: `backend/src/application/model_import.rs`
- Modify: `backend/src/db/repositories/model_import.rs`
- Modify: `backend/src/application/artifacts.rs`
- Modify: `backend/src/application/dataframe_reader.rs`
- Modify: `backend/src/application/analysis.rs`
- Modify: `backend/src/application/visualization.rs`
- Modify: `backend/src/tools/mod.rs`
- Create: `backend/tests/data_resources.rs`
- Modify: `backend/tests/upload_batches.rs`
- Modify: `backend/tests/model_imports.rs`
- Modify: `backend/tests/auth_scope.rs`
- Modify: `backend/tests/quota_concurrency.rs`

**Interfaces:**
- Consumes: Task 4 已解析候选、Task 2 schema/保留名、现有 result dataset/analysis。
- Produces: `DataResourceService`、统一的本地 Data 资源 HTTP DTO、四个受控 Agent 工具；Task 6 通过稳定 resource ID 绑定 Data 或 Struct 输入。

- [ ] **Step 1: 写失败的数据资源测试。** 覆盖通用资源可无 schemaRef、静态导入 `para` 是带 alias/schemaRef/source association 的普通 Data 且不进入 draft、`catalog_model_table_outside_network20_publishes_to_draft_and_version`、session data alias 唯一/归一/保留名、原文件与 Snappy Parquet 双 artifact、多 sheet 共享来源、hash 去重计费、混合批次两个子事务各自原子、`partial_confirmation_keeps_only_failed_child_retryable`、删除影响 preview、冻结引用存活、重新上传和参数卡覆盖都使用 replacement 且有三种动作、structured preview 不写资源、expected hash 过期返回 `resource_stale`、覆盖不改历史 artifact、引擎升级 `needs_rename`、结果/分析提升和跨用户 404。

- [ ] **Step 2: 运行 `cd backend && cargo test --test data_resources --test auth_scope --test quota_concurrency --features e2e-test` 并确认失败。**

- [ ] **Step 3: 定义服务接口。**

  ```rust
  pub struct DataResourceService { /* repo, artifacts, reader, facade registry */ }
  pub enum ReplacementAction { Replace, SaveAsNew, Cancel }
  pub async fn publish_candidates(&self, user: &CurrentUser, batch_id: i32,
      decisions: Vec<CandidateDecision>) -> Result<UploadConfirmation, DataResourceError>;
  pub async fn list(&self, user: &CurrentUser, session_id: i32,
      cursor: Option<String>, limit: u16) -> Result<DataResourcePage, DataResourceError>;
  pub async fn rename_alias(&self, user: &CurrentUser, resource_id: &str,
      alias: &str) -> Result<DataResourceView, DataResourceError>;
  pub async fn promote_dataset(&self, user: &CurrentUser, session_id: i32,
      dataset_id: i32, display_name: String) -> Result<DataResourceView, DataResourceError>;
  ```

  所有资源都使用 opaque UUID/ULID 和会话唯一 alias。默认 alias 由文件/sheet/adapter 输出名生成，冲突用 `data_<shortId>`；静态 `para` 保存 Struct schemaRef、同批来源和原始输出名，但 DTO 与存储类型仍是普通 Data。alias 校验当前 snapshot 的标识符、保留字、模型/结果/函数/临时命名空间；Struct 输入名允许成为满足该输入的资源 alias。新 snapshot 启动后批量标记冲突资源为 `needs_rename`，不自动改名。

- [ ] **Step 4: 实现混合确认的双事务语义。** 本批全部 ModelTable candidate 在一个事务应用 draft revision；包括静态 profile 固定 `para` 在内的全部 Data candidate 在另一个资源事务发布。任一子事务内部全成全败，一方失败不回滚另一方成功，批次进入 `PartiallyConfirmed` 并分别返回两个子状态、receipt 和诊断。已成功子事务的来源转为长期 artifact 且不再占 staging reservation；失败子事务保留可重试候选及仍需 reservation，修正后只重跑失败侧。全部成功才进入 `Confirmed` 并释放剩余预留；重复请求返回相同 receipt，不能重复发布或计费。`para` 记录与同批 workspace/import/case 的来源关联，但不加入 draft/version 表。

  在本步骤注册 `POST /api/upload-batches/{id}/confirm`；请求接受 expected draft、完整 candidate decisions、可选
  replacement action 和幂等 client operation ID，响应始终分别返回 model/resource 子状态。Task 4 的其他批次路由
  不承担发布职责。

- [ ] **Step 5: 实现删除与替换。** `POST /api/data-resources/{id}/deletion-preview` 返回短期 impact token、名称、alias、size、未确认配置/冻结/历史引用数；`DELETE /api/data-resources/{id}?impactToken=...` 重算影响后幂等隐藏 live resource、释放直接引用并使未确认配置失效。重新上传和 Task 6 参数卡修改共用 replacement 服务：确认比较 schema/row/time/hash，`Replace` 原子转移原 alias、给旧资源备用 alias、标记 replaced 并保留冻结引用，`SaveAsNew` 生成非冲突 alias，`Cancel` 不发布候选。任何路径都不得原地改写 artifact。

- [ ] **Step 6: 暴露数据 API。**

  ```text
  GET    /api/sessions/{id}/data-resources
  GET    /api/data-resources/{id}
  PATCH  /api/data-resources/{id}
  GET    /api/data-resources/{id}/rows
  GET    /api/data-resources/{id}/download?kind=original|normalized
  POST   /api/data-resources/{id}/structured-replacement-previews
  POST   /api/data-resource-replacement-previews/{id}/confirm
  POST   /api/data-resources/{id}/deletion-preview
  DELETE /api/data-resources/{id}?impactToken=<opaque>
  ```

  structured preview 只接受当前 snapshot 中明确匹配的 Struct schemaRef、规范字段和 expected content hash，返回服务端
  校验后的字段差异与短期 token，不发布资源；confirm 接受 `Replace|SaveAsNew|Cancel`，hash 已变化返回
  `resource_stale`。`PATCH` 更新资源的 display name 或 alias。提升资源没有 original 下载；响应显示来源
  run/analysis/dataset。静态 `para` 与其他资源一样返回 alias 和 replacement action，并额外显示已有的 schemaRef 与
  case/import 来源。行读取复用受控列投影和 cursor，下载不暴露路径。

- [ ] **Step 7: 接入自然语言分析。** `run_result_analysis`、物化 view 和受控 ECharts 来源联合类型增加 `DataResourceId`；输出血缘保存 resource ID/hash。Agent 工具只接受稳定 ID、受控列/过滤 spec。`promote_result_dataset` 存在歧义时拒绝并让 Agent 请求用户选择。

- [ ] **Step 8: 运行聚焦测试及 `cargo test --test result_deletion --features e2e-test`。**

- [ ] **Step 9: 提交。**

  ```bash
  git add backend/src/application backend/src/db/repositories/data_resource.rs backend/src/db/repositories/model_import.rs backend/src/api/data_resources.rs backend/src/api/uploads.rs backend/src/tools backend/tests/data_resources.rs backend/tests/upload_batches.rs backend/tests/model_imports.rs backend/tests/auth_scope.rs backend/tests/quota_concurrency.rs backend/tests/result_deletion.rs integration-tests/catalog/features.yaml backend/src/contract_manifest.rs backend/tests/contract_manifest.rs
  git commit -m "feat: manage independent dataframe resources"
  ```

### Task 6: 实现目录驱动的研究配置与最终确认状态机

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/contract_manifest.rs`
- Create: `backend/src/application/study_configurations.rs`
- Create: `backend/src/application/data_mapping.rs`
- Create: `backend/src/db/repositories/study_configuration.rs`
- Create: `backend/src/api/study_configurations.rs`
- Create: `backend/src/tools/search_study_facades.rs`
- Create: `backend/src/tools/prepare_study.rs`
- Remove: `backend/src/tools/plan_study.rs`
- Modify: `backend/src/tools/mod.rs`
- Modify: `backend/src/api/mod.rs`
- Create: `backend/tests/study_configurations.rs`
- Modify: `backend/tests/study_planning.rs`
- Modify: `backend/tests/tool_manifest.rs`

**Interfaces:**
- Consumes: Task 2 `FacadeRegistry`、Task 5 data resource/model workspace。
- Produces: `StudyConfigurationService`、卡片 DTO、`PreparedStudyConfirmation`；Task 9 的 repository unit-of-work 在同一
  数据库事务内重新校验准备结果、冻结 `LockedStudyContract`/`WorkflowInputSnapshot` 并启动 workflow。

- [ ] **Step 1: 写失败的配置状态机测试。** 覆盖显式函数直接选择、多个门面解释卡、门面的全部 model 输入只从 active 的实际模型表动态绑定且不含 `para`、`network[20]` 正确展开但不限制其他 model 输入、保护配合只绑定其声明的 `fuse/recloser/relay` 模型表、无 model 输入的门面没有 active 模型也能进入确认、单一兼容 Data 自动预选、多 Data 选择、无 Data 上传提示、列映射派生资源、Struct schema 卡、同源唯一 `para` 普通资源自动预填、多个兼容资源要求选择、参数卡修改必须选择覆盖或另存、覆盖产生 replacement 且历史引用不变、只复用同门面最后一个 `para`、结果推荐与组合约束、资源/alias/snapshot race 返回 `configuration_stale`、未确认时禁止脚本/workflow。

- [ ] **Step 2: 运行 `cd backend && cargo test --test study_configurations --test study_planning --test tool_manifest --features e2e-test` 并确认失败。**

- [ ] **Step 3: 定义状态和合同。**

  ```rust
  pub enum StudyConfigurationState { ChoosingFacade, CollectingInputs, ReadyToConfirm,
      Confirmed, Invalidated, Cancelled }
  pub struct LockedStudyContract { pub configuration_id: i32, pub facade_id: String,
      pub function: String, pub facade_contract_hash: String, pub engine_snapshot_id: String,
      pub input_aliases: Vec<String>, pub structs: Vec<CanonicalStructValue>,
      pub results: Vec<FrozenResultContract> }
  pub struct WorkflowInputSnapshot { pub model: Option<FrozenModelInput>,
      pub resources: Vec<FrozenStudyInput> }
  pub struct ConfirmedStudyContract { pub locked: LockedStudyContract,
      pub inputs: WorkflowInputSnapshot }
  pub struct PreparedStudyConfirmation { pub configuration_id: i32,
      pub expected_revision: i64, pub expected_snapshot_id: String,
      pub replacement_actions: Vec<PreparedResourceReplacement> }
  pub struct CanonicalStructValue { pub input_id: String, pub schema_ref: String,
      pub fields: serde_json::Value, pub resource_id: String,
      pub resource_content_hash: String, pub resource_diff: serde_json::Value }
  pub async fn prepare_confirmation(&self, user: &CurrentUser,
      configuration_id: i32, expected_revision: i64)
      -> Result<PreparedStudyConfirmation, StudyConfigurationError>;
  ```

  每次卡片提交带 expected revision。`inputKind=model` 只按门面声明从 active 的实际模型表取值；没有 model 输入时
  `WorkflowInputSnapshot.model=None`，不要求 active。`struct` 只使用 Task 2 已规范化的 `DataFrameSchemaContract` 生成类型/单位/范围/枚举/联动校验卡，并绑定 schema 兼容的普通 Data resource；`data` 也使用同一合同校验和绑定普通 Data resource，但用资源选择/映射卡。配置层不得读取原始 catalog JSON 或解释 prose constraint。Struct 字段被修改时，确认事务调用 Task 5 replacement 服务生成新的单行 Parquet 资源。映射确认创建 Task 5 的不可变派生 Data 资源。任何 catalog 把 `para` 声明为 model 都禁用该门面并显示合同诊断。

- [ ] **Step 4: 定义研究配置 API。**

  ```text
  GET    /api/study-configurations/{id}
  PUT    /api/study-configurations/{id}/facade
  PUT    /api/study-configurations/{id}/bindings/{inputId}
  POST   /api/study-configurations/{id}/bindings/{inputId}/mapping
  PUT    /api/study-configurations/{id}/structs/{inputId}
  PUT    /api/study-configurations/{id}/results
  DELETE /api/study-configurations/{id}
  ```

  `prepare_study` 创建配置并持久化对话卡；`search_study_facades` 返回差异与禁用诊断。最终确认按钮文本固定为“确认并生成脚本”；自然语言肯定不能调用 confirm。Task 9 在 workflow API 中注册 `POST /api/study-configurations/{id}/confirm`，并通过 repository unit-of-work 在一个事务内重新验证 `PreparedStudyConfirmation`、发布 replacement/new resource、创建确认记录/workflow/run/input snapshot；本任务不得先提供一个只确认但尚未启动的公共端点，也不得让 application service 持有 Diesel connection。

- [ ] **Step 5: 实现参数资源、复用和修改研究。** 同门面历史配置先询问是否复用最后一个 `para`；用户拒绝复用后再用唯一同源兼容 Data resource 或目录默认值初始化参数卡，多个合理资源必须选择。字段有变化时最终摘要必须要求 `Replace` 或 `SaveAsNew`；没有来源资源时只能创建新普通资源。卡片草稿更新不得提前写资源；`prepare_confirmation` 只产生受控 replacement command，Task 9 的 unit-of-work 才发布 resource 并冻结新 ID/hash。其他 Struct/Data 不自动复用；用户从脚本/run/result 明确修改时预填基准配置，多个候选显示选择卡。新确认保存 `derivedFromWorkflowId`、资源血缘和结构化差异，不改旧脚本、旧结果或历史 artifact。

- [ ] **Step 6: 运行聚焦测试。** 确认旧固定五研究 policy 已删除，任何新有效门面无需改 Rust 枚举即可进入搜索和配置。

- [ ] **Step 7: 提交。**

  ```bash
  git add backend/src/application/study_configurations.rs backend/src/application/data_mapping.rs backend/src/db/repositories/study_configuration.rs backend/src/api/study_configurations.rs backend/src/tools backend/src/api/mod.rs backend/tests/study_configurations.rs backend/tests/study_planning.rs backend/tests/tool_manifest.rs integration-tests/catalog/features.yaml backend/src/contract_manifest.rs backend/tests/contract_manifest.rs
  git commit -m "feat: confirm dynamic study configurations"
  ```

### Task 7: 扩展 durable message、SSE 和会话删除合同

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/contract_manifest.rs`
- Modify: `backend/src/application/events.rs`
- Modify: `backend/src/application/chat.rs`
- Modify: `backend/src/application/history.rs`
- Modify: `backend/src/db/repositories/message.rs`
- Modify: `backend/src/db/repositories/session.rs`
- Modify: `backend/src/api/sessions.rs`
- Modify: `frontend/src/types/message.ts`
- Modify: `frontend/src/state/messageReducer.ts`
- Modify: `frontend/src/state/sessionFeed.ts`
- Modify: `frontend/src/hooks/useSessionFeed.ts`
- Modify: `frontend/src/api/client.ts`
- Modify: `frontend/src/api/client.test.ts`
- Modify: `frontend/src/components/Sidebar.tsx`
- Modify: `frontend/src/components/Sidebar.test.tsx`
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/state/sessionFeed.test.ts`
- Create: `frontend/src/hooks/useSessionFeed.test.ts`
- Modify: `backend/tests/sse_recovery.rs`
- Modify: `backend/tests/history_search.rs`
- Modify: `backend/tests/auth_scope.rs`

**Interfaces:**
- Consumes: Tasks 4、6 的 upload/config view 与 Task 1 message/session schema。
- Produces: 三种新消息 `upload-batch`、`study-config`、`workflow-progress` 的独立公共 envelope，可持久化、分页、搜索摘要和 SSE 回放；Tasks 8–11 只写这些已定义的消息，不再新增临时 payload。还产生按稳定 message ID 加载连续上下文并切换独立 anchored window 的定位能力。

- [ ] **Step 1: 写失败的消息和会话删除测试。** 断言同一 upload/config/workflow 用稳定 operation ID 覆盖更新而非堆积；终态幂等；刷新前建立 SSE 再读 snapshot 不丢事件；attempt 详情默认折叠；路径/原始 stderr 不序列化；`aroundId` 必须属于当前用户/会话、响应必含精确目标且前后有界，foreign/missing 统一 404；远端 context page 切换为连续 anchored window，不与有缺口的 latest page 拼接，最新缓存/SSE 保留为“有新消息”，`returnLatest` 恢复或重载最新页；会话删除先取 preview，dialog 显示模型/数据/脚本/run/结果/空间，取消不发 DELETE，影响变化刷新 preview；会话进入删除中后 repository 拒绝新消息和领域写入。

- [ ] **Step 2: 运行 `cd backend && cargo test --test sse_recovery --test history_search --test auth_scope --features e2e-test` 以及 `cd frontend && npm test -- --run src/api/client.test.ts src/state/sessionFeed.test.ts src/hooks/useSessionFeed.test.ts src/components/Sidebar.test.tsx`，确认失败。**

- [ ] **Step 3: 扩展事件联合类型。** `upload-batch` 只携稳定 batch/candidate/两个子事务状态/diagnostic；`study-config` 携 revision 和当前 action card；`workflow-progress` 携 state、execution index/max、repair index/max、elapsed、折叠 attempt 摘要、最终 script/output refs 或失败码。EventWriter 先事务 upsert message 再广播，SSE event ID 使用 message revision，回放去重键为 `(message_id, revision)`。

- [ ] **Step 4: 更新历史与前端 reducer 合同。** 历史搜索只索引卡片可读摘要，不索引脚本全文或 stderr；消息分页恢复当前最新卡片 revision。`GET /api/sessions/{id}/messages?aroundId=<message>&limit=50` 先验证目标归属，再返回包含目标的连续有界上下文。`useSessionFeed.loadAround(messageId)` 在目标未加载时请求 context page，用独立 reducer action 进入 anchored window，保留 latest cache、分页游标和 SSE revision但不拼接不连续区间；暴露 `returnLatest/hasNewerMessages`。前端 types 与 Rust serde 字段完全对齐，合同测试双向验证。

- [ ] **Step 5: 实现 V4 会话删除。** `POST /api/sessions/{id}/deletion-preview` 返回模型、数据、脚本、run、结果/分析 artifact 数、逻辑空间和短期 impact token；`DELETE /api/sessions/{id}?impactToken=<opaque>` 重新校验影响，变化时返回 `deletion_impact_changed`。Sidebar 第一次点击只加载 preview，命名 dialog 显示全部影响，确认才携 token 删除，取消不变更；重复提交期间禁用按钮。后端确认后立即隐藏并阻止写入，通过统一 session cancellation token 取消当前已有 Agent/engine 工作，再清理记录与引用；Task 9 的 managed workflow 必须注册并遵守同一 token。其他会话血缘需要的内容留下最小 tombstone。重复删除幂等，普通读取对删除中/已删除会话返回 404。

- [ ] **Step 6: 运行聚焦测试。** 再跑 `cargo test --test contract_manifest --features e2e-test` 并登记三种新 message。

- [ ] **Step 7: 提交。**

  ```bash
  git add backend/src/application/events.rs backend/src/application/chat.rs backend/src/application/history.rs backend/src/db/repositories/message.rs backend/src/db/repositories/session.rs backend/src/api/sessions.rs frontend/src/types/message.ts frontend/src/state frontend/src/hooks/useSessionFeed.ts frontend/src/hooks/useSessionFeed.test.ts frontend/src/api/client.ts frontend/src/api/client.test.ts frontend/src/components/Sidebar.tsx frontend/src/components/Sidebar.test.tsx frontend/src/App.tsx backend/tests integration-tests/catalog/features.yaml backend/src/contract_manifest.rs backend/tests/contract_manifest.rs
  git commit -m "feat: stream v4 configuration workflows"
  ```

### Task 8: 发布 V4 Agent bundle 和受控工具权限

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/contract_manifest.rs`
- Create: `backend/resources/agent/v4/manifest.json`
- Create: `backend/resources/agent/v4/system.md`
- Create: `backend/resources/agent/v4/skill-index.json`
- Create/modify skills under: `backend/resources/agent/v4/skills/`
- Modify: `backend/src/agent/knowledge.rs`
- Modify: `backend/src/agent/prompt.rs`
- Modify: `backend/src/application/chat.rs`
- Modify: `backend/src/tools/support.rs`
- Modify: `backend/src/tools/mod.rs`
- Modify: `backend/tests/agent_knowledge.rs`
- Modify: `backend/tests/agent_factory.rs`
- Modify: `backend/tests/scripted_agent.rs`
- Modify: `backend/tests/tool_manifest.rs`

**Interfaces:**
- Consumes: Tasks 5–6 工具与配置状态机。
- Produces: V4 conversational Agent；它只能准备配置和管理数据资源，不能提前生成/运行受管脚本。

- [ ] **Step 1: 写失败的 Agent 行为测试。** 原始短指令“求解牛顿法潮流”必须搜索/选择 Newton 门面并创建配置卡；缺 Data/Struct 时只返回卡；禁用门面显示具体诊断；资源歧义不猜测；结果提升先唯一定位；模型不得调用已删除的 `create_script`/`plan_study` 来绕过确认。

- [ ] **Step 2: 运行 `cd backend && cargo test --test agent_knowledge --test agent_factory --test scripted_agent --test tool_manifest --features e2e-test` 并确认失败。**

- [ ] **Step 3: 建立 `v4.0.0` bundle。** system prompt 只常驻策略：先查 active/资源/门面，显式方法优先，所有输入/参数/结果完整后等待卡片最终确认；不常驻整套 docs，不写死模型表或函数。skills 覆盖 model intake、study configuration、resource analysis、comparison、troubleshooting；RSS 生成从聊天工具权限移到 Task 9 的专用 author。

- [ ] **Step 4: 更新工具注册表。** 加入 `search_study_facades`、`prepare_study`、`list_data_resources`、`read_data_resource`、`update_data_resource_alias`、`promote_result_dataset`；移除 `plan_study` 和对话 Agent 的 `create_script`。保留 V3 未变的模型修改、历史、分析、比较、引擎文档工具。manifest/注册表双向校验且所有新工具有行为测试。

- [ ] **Step 5: 修改 ChatService 成功判定。** `prepare_study` 成功后由已持久化配置卡作为本轮领域结果，provider 收尾失败不得用通用错误覆盖；卡片等待用户操作，ChatService 不启动 workflow。会话同一非终态 workflow 存在时，新的研究请求返回 `workflow_in_progress`，只读和取消仍可用。

- [ ] **Step 6: 运行聚焦测试及 bundle hash 校验。** 更新 `AGENT_BUNDLE_VERSION=v4.0.0` 默认值和 `.env.example` 注释。

- [ ] **Step 7: 提交。**

  ```bash
  git add backend/resources/agent/v4 backend/src/agent backend/src/application/chat.rs backend/src/tools backend/src/config.rs backend/.env.example backend/tests integration-tests/catalog/features.yaml backend/src/contract_manifest.rs backend/tests/contract_manifest.rs
  git commit -m "feat: publish v4 study configuration agent"
  ```

### Task 9: 建立专用脚本 author、workflow 和 attempt 持久化状态机

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/contract_manifest.rs`
- Create: `backend/src/agent/script_author.rs`
- Create: `backend/src/application/managed_workflows.rs`
- Create: `backend/src/db/repositories/managed_workflow.rs`
- Create: `backend/src/api/workflows.rs`
- Modify: `backend/src/api/study_configurations.rs`
- Create: `backend/src/testing/scripted_author.rs`
- Modify: `backend/src/agent/factory.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/api/mod.rs`
- Modify: `backend/src/lib.rs`
- Modify: `backend/src/testing/mod.rs`
- Create: `backend/tests/managed_workflows.rs`
- Modify: `backend/tests/sse_recovery.rs`

**Interfaces:**
- Consumes: Task 6 `StudyConfigurationService::prepare_confirmation`/`PreparedStudyConfirmation`、Task 7 durable message/EventWriter 与 session cancellation token、Task 1 workflow 表和现有 scheduler。
- Produces: `ScriptAuthor`、`ManagedWorkflowService`、持久化 workflow/attempt 查询与取消 API，以及首次受管 run 到终态脚本消息的稳定锚点；Task 10 实现执行循环。

- [ ] **Step 1: 写失败的状态机测试。** 覆盖最终确认幂等创建一个 workflow/run、数据库拒绝同会话第二个非终态 workflow、candidate artifact 不可变且有 parent attempt、重复 SSE/HTTP 不重复工作、取消和成功只允许一个终态、会话删除 cancellation token 与 workflow 成功提交竞争时只能有一个终态且删除中不得发布、重启把非终态收敛为 `service_restarted`、首份脚本前/后重启的重试起点不同；首份可见脚本产生前 `scriptMessageId=null`，终态 script resource/message/run anchor 原子建立且重复终态不能改锚点。

- [ ] **Step 2: 运行 `cd backend && cargo test --test managed_workflows --test sse_recovery --features e2e-test` 并确认失败。**

- [ ] **Step 3: 抽象专用模型接口。**

  ```rust
  #[async_trait]
  pub trait ScriptAuthor: Send + Sync {
      async fn generate(&self, request: ScriptGenerationRequest) -> Result<String, ScriptAuthorError>;
      async fn repair(&self, request: ScriptRepairRequest) -> Result<String, ScriptAuthorError>;
  }
  pub struct ScriptGenerationRequest { pub locked: LockedStudyContract,
      pub inputs: WorkflowInputSnapshot,
      pub engine_docs: Vec<EngineDocRef>, pub instruction: String }
  pub struct ScriptRepairRequest { pub current_script: String, pub sanitized_stderr: String,
      pub diagnostic: Option<serde_json::Value>, pub locked_contract: LockedStudyContract,
      pub inputs: WorkflowInputSnapshot,
      pub instruction: String }
  ```

  DeepSeek 实现使用 rig 但不注册聊天工具；测试实现按序排队完整脚本/错误/barrier。generation/repair 的传输失败按 `AGENT_MODEL_RETRY_MAX=2` 独立重试并审计，不创建 attempt、不消耗修正预算。

- [ ] **Step 4: 定义 workflow 状态机。**

  ```rust
  pub enum WorkflowState { Queued, Generating, Executing, Repairing,
      Succeeded, Failed, CancelledByUser, ServiceRestarted }
  pub enum AttemptState { Candidate, ContractRejected, Executing, EngineFailed,
      ResultRejected, Succeeded }
  pub async fn confirm_configuration_and_start(&self, user: &CurrentUser,
      configuration_id: i32, expected_revision: i64)
      -> Result<ManagedWorkflowView, WorkflowError>;
  pub struct ConfirmedStudyStart { pub confirmed: ConfirmedStudyContract,
      pub workflow: ManagedWorkflowView }
  #[async_trait]
  pub trait StudyStartRepository {
      async fn confirm_and_start(&self, user_id: &UserId,
          prepared: PreparedStudyConfirmation) -> Result<ConfirmedStudyStart, RepoError>;
  }
  pub async fn cancel(&self, user: &CurrentUser, workflow_id: i32)
      -> Result<ManagedWorkflowView, WorkflowError>;
  ```

  `confirm_configuration_and_start` 先调用 Task 6 `prepare_confirmation`，再由 `StudyStartRepository` 在一个 SQLite
  immediate transaction 中重新校验 configuration revision、active/model tables、resource ID/hash/alias/schema、
  snapshot 和 result contract，完成 Struct replacement/new resource、确认记录、workflow、simulation run、精确
  input snapshot 和 durable progress message 后才提交并排队；任何一步失败整体回滚，竞态返回
  `configuration_stale`。application service 不接触 Diesel connection。每次 candidate 先保存 artifact/hash/父 attempt，再执行。终态
  一次性创建用户可见 Agent script 和 script message，验证同 session/kind 后把该 message ID 只写一次到 run；成功
  指向最后成功 candidate，失败指向最后 candidate 并允许“重新尝试”。没有任何候选脚本的模型失败允许锚点为空。

- [ ] **Step 5: 暴露 workflow API。**

  ```text
  POST /api/study-configurations/{id}/confirm
  GET  /api/workflows/{id}
  POST /api/workflows/{id}/cancel
  ```

  confirm body 只接受 `expectedRevision` 和幂等 client operation ID，不接受客户端提供的冻结输入。状态查询返回 attempt 摘要和脱敏诊断，不返回原始 stderr/path。取消幂等停止排队、通知 author 或 Task 3 进程 token、清理私有未发布结果；已持久化 attempt 保留。

- [ ] **Step 6: 实现启动恢复。** 进程启动在接收请求前，把所有非终态 workflow/attempt 事务性标记 `service_restarted`，清理已登记的私有 result-dir，发布可恢复的失败消息；绝不根据临时文件自动续跑。

- [ ] **Step 7: 运行聚焦测试。** 再跑 `cargo test --test agent_scheduler --features e2e-test`，确认全局/用户队列限制继续生效且不代替数据库每会话约束。

- [ ] **Step 8: 提交。**

  ```bash
  git add backend/src/agent/script_author.rs backend/src/application/managed_workflows.rs backend/src/db/repositories/managed_workflow.rs backend/src/api/workflows.rs backend/src/api/study_configurations.rs backend/src/testing backend/src/agent/factory.rs backend/src/application/mod.rs backend/src/api/mod.rs backend/src/lib.rs backend/tests/managed_workflows.rs backend/tests/sse_recovery.rs integration-tests/catalog/features.yaml backend/src/contract_manifest.rs backend/tests/contract_manifest.rs
  git commit -m "feat: persist managed study workflows"
  ```

### Task 10: 实现生成、执行、合同校验和自动修正循环

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/contract_manifest.rs`
- Create: `backend/src/application/managed_rss.rs`
- Create: `backend/src/application/script_contract.rs`
- Create: `backend/src/application/workflow_worker.rs`
- Create: `backend/src/application/diagnostics.rs`
- Modify: `backend/src/application/managed_workflows.rs`
- Modify: `backend/src/application/runs.rs`
- Modify: `backend/src/application/artifacts.rs`
- Modify: `backend/src/adapters/mod.rs`
- Modify: `backend/src/testing/scripted_author.rs`
- Modify: `backend/src/testing/scenario_engine.rs`
- Create: `backend/tests/script_repair.rs`
- Modify: `backend/tests/managed_workflows.rs`
- Modify: `backend/tests/simulation_runs.rs`

**Interfaces:**
- Consumes: Task 3 引擎执行、Task 9 author/workflow、Task 6 冻结合同。
- Produces: 完整的首次受管运行/“重新尝试”循环和冻结结果合同；Task 11 复用最终 script/run。

- [ ] **Step 1: 写失败的循环测试。** 覆盖首次脚本立即执行、第一次成功、非零退出修正后成功、最多 3 个修正/4 次 engine、初始合同违规保存但不占修正预算/修正候选合同违规消耗预算且两者都不执行、结果合同/文件限制进入修正、超时/监控/配额/模型不可用不进入错误预算、64 KiB stderr 首尾脱敏、原始审计不出网、取消中止、成功不发布多余/部分结果。

- [ ] **Step 2: 运行 `cd backend && cargo test --test script_repair --test managed_workflows --test simulation_runs --features e2e-test` 并确认失败。**

- [ ] **Step 3: 实现结构合同检查器。**

  ```rust
  pub struct ScriptContractInspector;
  pub struct ContractViolation { pub fields: Vec<String>, pub diagnostic: serde_json::Value }
  pub fn inspect_candidate(source: &str, locked: &LockedStudyContract)
      -> Result<(), ContractViolation>;
  ```

  `managed_rss.rs` 实现专供 Agent 生成脚本的 tokenizer/AST extractor，按当前 snapshot 的 RSS 语言文档识别顶层变量
  声明、结果解构、直接函数调用、标识符引用和字面量；允许 helper assignments，但拒绝把目标调用藏在控制流、动态
  callee、内嵌脚本或无法证明的间接表达式中。检查门面函数、算法、输入 alias、Struct 规范值和结果集合未变化；
  不用正则或字符串包含判断，也不判断一般 RSS 语义和运行可行性，后两项只由 tensoreval 决定。parser 以 eig-rs
  文档中的有效/无效脚本片段做合同测试；文档语法变化导致 extractor 无法证明时返回 `repair_contract_violated`，
  不能放宽检查。

- [ ] **Step 4: 实现输入树与结果合同。** 首次 workflow 全程使用 `WorkflowInputSnapshot`：只注入门面声明且已确认冻结的 model 表；没有 model 输入时不要求 active 或注入无关网络表；再按冻结 alias 注入包括 Struct `para` 在内的全部 Data resource ID/hash。参数卡修改后的单行 Struct 已由 Task 9 的确认事务作为 replacement/new 普通资源发布，不从 active model 复制 `para`。成功要求退出码 0、结果稳定名称集合精确相等、每个 Snappy Parquet 可读且完整 schema 相等；零行合法。全部通过后才在一个配额事务发布 datasets。

- [ ] **Step 5: 实现循环判定。** 初始 generation 不计修正预算；每个 repair 回复都保存 candidate 并消耗一次预算。非零退出、`result_contract_invalid`、`result_file_limit`、`result_file_too_large`、`result_total_too_large` 可修正；`engine_timeout`、`result_monitor_failed`、路径安全、配额和基础设施错误直接失败。合同违规 attempt 保存后不调用 engine。

- [ ] **Step 6: 固定修正 prompt 和诊断管线。** prompt 必须包含“当前脚本全文 + 脱敏 stderr + 可选结构诊断 + 锁定合同 + `只输出修改后的完整脚本，不要解释`”。先对完整有界 stderr 脱敏服务器绝对路径/secret/控制字符，再按 UTF-8 边界截为 64 KiB 首尾并记录截断前/后 hash、字节数。原始有界 stderr 只进入受限审计表/存储，不进浏览器、普通 API 或 LLM。

- [ ] **Step 7: 保证终态提交原子性。** 最终 attempt、simulation run、script resource、规范 script message、全部成功 result datasets、artifact refs 和 workflow terminal message 在事务中收敛，并校验后把 script message ID 一次性写入 run；失败/取消终态有候选时同样原子发布该候选的 script resource/message 和 run anchor，但不发布结果。任何失败清理私有 result-dir 且不发布部分结果。结果列表只保存元数据，不自动创建分析/view。

- [ ] **Step 8: 运行聚焦测试和 Task 3 的 process tests。**

- [ ] **Step 9: 提交。**

  ```bash
  git add backend/src/application/managed_rss.rs backend/src/application/script_contract.rs backend/src/application/workflow_worker.rs backend/src/application/diagnostics.rs backend/src/application/managed_workflows.rs backend/src/application/runs.rs backend/src/application/artifacts.rs backend/src/adapters backend/src/testing backend/tests/script_repair.rs backend/tests/managed_workflows.rs backend/tests/simulation_runs.rs integration-tests/catalog/features.yaml backend/src/contract_manifest.rs backend/tests/contract_manifest.rs
  git commit -m "feat: execute and repair managed scripts"
  ```

### Task 11: 完成成功脚本运行、失败脚本重试和手写脚本语义

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `backend/src/contract_manifest.rs`
- Modify: `backend/tests/contract_manifest.rs`
- Modify: `backend/src/application/scripts.rs`
- Modify: `backend/src/application/runs.rs`
- Create: `backend/src/application/retries.rs`
- Modify: `backend/src/db/repositories/script.rs`
- Modify: `backend/src/db/repositories/run.rs`
- Create: `backend/src/db/repositories/retry.rs`
- Modify: `backend/src/api/scripts.rs`
- Modify: `backend/src/api/runs.rs`
- Modify: `backend/src/api/mod.rs`
- Modify: `backend/tests/scripts.rs`
- Modify: `backend/tests/simulation_runs.rs`
- Modify: `backend/tests/result_deletion.rs`
- Create: `backend/tests/workflow_retries.rs`

**Interfaces:**
- Consumes: Task 10 最终 Agent script/result contract、Task 5 当前 alias 资源、Task 2 当前 facade hash。
- Produces: 三条互斥执行路径：成功 Agent script 单次 Run、失败 Agent script Retry workflow、手写/上传 RSS 单次 Run；以及终态运行影响预览和级联结果删除。

- [ ] **Step 1: 写失败的脚本/运行测试。** 断言：上传/粘贴只保存不执行；手写执行不静态校验、不修正且开放结果合同至少一个合法 Snappy Parquet；成功 Agent script Run 只重新读取门面实际需要的最新 active（可空）/alias 并执行一次；无 model 输入且无 active 模型的门面可运行；普通/手写 Run 复制脚本资源的规范 message anchor，Run DTO 返回精确 `scriptMessageId`，不存在可见脚本时为 null 而不猜测；旧结果合同漂移直接 `result_contract_invalid`；失败 Retry 比较输入和 facade hash并在终态产生新的派生脚本/message anchor、旧脚本保持不可变；合同变化 `study_contract_changed`；普通 Run 不检查 binary hash。运行删除测试覆盖终态成功/失败/取消与零结果、非终态 `run_busy`、任一结果消费中 `result_busy`、预览 token/影响变化、run 加全部直接结果原子 tombstone、artifact 配额、派生/提升资源存活、重复删除幂等和跨用户 404。

- [ ] **Step 2: 运行 `cd backend && cargo test --test scripts --test simulation_runs --test result_deletion --test workflow_retries --features e2e-test` 并确认失败。**

- [ ] **Step 3: 修改脚本创建合同。** 手写/上传 `.rss` 只校验文件名、UTF-8 和 2 MiB 上限，不调用 `validate_script`。Agent script 只能由 Task 10 terminal transaction 创建，保存 confirmed config、facade hash、冻结 result contract、最终 candidate artifact 和 terminal status。所有 script artifact 不可修改。

- [ ] **Step 4: 修改 Run API。**

  ```text
  POST /api/scripts/{id}/runs            body: { clientRunId }
  POST /api/scripts/{id}/retry-previews  body: { clientRetryId }
  POST /api/retry-previews/{id}/confirm  body: { confirmed: true }
  POST /api/runs/{id}/deletion-preview
  DELETE /api/runs/{id}?impactToken=<opaque>
  ```

  前端不再提交 model version。后端事务按 Agent script 的冻结门面合同只读取当前所需 active 表；门面没有 model 输入时允许 model 为空。Agent script 按 required alias 解析包括 `para` 在内的最新 Data 资源。manual/upload script 注入当前会话全部 live Data alias，并在 active 模型存在时注入其全部实际模型表；没有 active 时仍允许只使用 Data 资源的手写脚本运行，缺少的变量由 tensoreval 实际报错。创建 Run 时解析 script resource 的同会话规范 script message并冻结 `scriptMessageId`；run list/detail DTO 原样返回该可空 stable ID。每次保存可空 model 与实际 resource ID/hash 和 Struct 字段及来源审计；资源被覆盖后 Run 使用新 ID/hash，既有 run 继续引用旧 artifact。

- [ ] **Step 5: 实现单次 Run。** 成功 Agent script 沿用创建时结果合同，失败不修正且不改 script。manual/upload 使用开放合同：至少一个名称合法唯一、可读、未超限的 Snappy Parquet；集合/schema 每次可不同；空目录 `no_result_dataset`；任一非法项整次失败。

- [ ] **Step 6: 实现运行删除。** preview 只允许终态 run，返回状态、live result 数/字节、派生与提升引用数及短期 impact token；非终态返回 `run_busy`。DELETE 重新校验 token、终态、完整结果集合和 busy consumers，在一个事务中 tombstone run 与全部 live 直接结果、保存最小 producer/schema/hash 墓碑并释放直接 artifact refs；任一结果 busy 则整次 `result_busy`。脚本、输入、完成的派生/比较/视图和提升资源不级联，重复请求幂等，普通查询对已删 run/result 返回 404。

- [ ] **Step 7: 实现 Retry preview。** 比较上次冻结与门面实际需要的当前 active（如有）/resource；完全相同直接创建新 workflow，变化则持久化 diff preview，用户确认后冻结最新输入。alias 缺失/`needs_rename`/schema 不兼容阻塞。只比较同 facade 的 `facadeContractHash`；不同返回 `study_contract_changed` 和结构差异，不调用 author/engine。新 workflow 保存来源 workflow/script ID 和完整新修正预算；到达终态并有候选时发布新的派生 Agent script/message，新 run 锚定该消息，原失败脚本与原 run anchor 不变。

- [ ] **Step 8: 运行聚焦测试。** 同时跑运行/结果删除和历史测试，确认资源替换或删除不改变既有 run/result，run 删除不破坏完成的派生或提升资源。

- [ ] **Step 9: 提交。**

  ```bash
  git add backend/src/application/scripts.rs backend/src/application/runs.rs backend/src/application/retries.rs backend/src/db/repositories backend/src/api/scripts.rs backend/src/api/runs.rs backend/src/api/mod.rs backend/tests/scripts.rs backend/tests/simulation_runs.rs backend/tests/result_deletion.rs backend/tests/workflow_retries.rs integration-tests/catalog/features.yaml backend/src/contract_manifest.rs backend/tests/contract_manifest.rs
  git commit -m "feat: manage run execution retry and deletion"
  ```

### Task 12: 实现数据文件工作台与统一上传交互

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Create: `frontend/src/types/dataResource.ts`
- Create: `frontend/src/types/uploadBatch.ts`
- Modify: `frontend/src/types/index.ts`
- Modify: `frontend/src/types/workspace.ts`
- Modify: `frontend/src/api/client.ts`
- Modify: `frontend/src/api/client.test.ts`
- Create: `frontend/src/hooks/useDataResources.ts`
- Create: `frontend/src/hooks/useUploadBatches.ts`
- Remove: `frontend/src/hooks/useModelImports.ts`
- Create: `frontend/src/components/resources/DataResourcePanel.tsx`
- Create: `frontend/src/components/resources/DataResourceList.tsx`
- Create: `frontend/src/components/resources/DataResourceWorkbench.tsx`
- Create: `frontend/src/components/resources/DataResourceDeleteDialog.tsx`
- Create: `frontend/src/components/resources/ReplacementDiffCard.tsx`
- Modify: `frontend/src/components/uploads/UploadQueue.tsx`
- Modify: `frontend/src/components/Composer.tsx`
- Modify: `frontend/src/components/ResultPanel.tsx`
- Modify: `frontend/src/components/workspace/WorkspacePanel.tsx`
- Modify: `frontend/src/components/workspace/WorkspacePanel.test.tsx`
- Modify: `frontend/src/App.tsx`
- Create: `frontend/src/components/resources/DataResourcePanel.test.tsx`
- Create: `frontend/src/hooks/useDataResources.test.ts`
- Create: `frontend/src/hooks/useUploadBatches.test.ts`

**Interfaces:**
- Consumes: Tasks 4–5 HTTP DTO、Task 7 `upload-batch` message。
- Produces: 右侧“数据文件”入口、上传/预览/下载/删除/替换交互；Task 13 在对话卡片中复用 upload hooks。

- [ ] **Step 1: 写失败的 Vitest。** 断言 session 切换隔离资源和批次、“数据文件”中的所有条目统一为普通 Data 资源而模型候选仍进入模型子事务、`para` 显示普通 alias/schemaRef/来源和重新上传、任一 Struct-compatible 单行资源显示“修改参数”且普通表不显示、cursor 列表、schema/行分页、原文件入口仅在可用时显示、删除先加载 impact 再确认、replacement 显示新旧 diff/三个动作、刷新恢复 staging、全选不选择隐藏 sheet、案例导入显示 20 模型表 + 1 个普通 `para` 资源、模型版本列表/详情显示冻结描述、“生成版本”接受可选的 200 字符单行描述、空值由后端生成并回显、内容去重时显示复用已有版本及原描述、401 统一退出、窄屏不横向溢出。

- [ ] **Step 2: 运行 `cd frontend && npm test -- --run src/components/resources/DataResourcePanel.test.tsx src/components/workspace/WorkspacePanel.test.tsx src/hooks/useDataResources.test.ts src/hooks/useUploadBatches.test.ts` 并确认失败。**

- [ ] **Step 3: 定义前端公共类型和 API。** 类型逐字段匹配 Rust DTO，opaque ID 全为 string；`ModelVersion` 增加必需 `description`，发布请求增加可选 `description` 并保留 `deduplicated/replayed` 响应；上传 multipart 只含文件、clientBatchId、入口 hint 和可选 replacement resource ID。parse settings/candidate decisions 只发送服务端允许字段。

- [ ] **Step 4: 用 `useUploadBatches` 替换模型专用上传 hook。** 从模型工作区、数据文件、研究缺参和通用 Composer 传不同 purpose hint；同一 hook 恢复服务端批次、提交 parse/purpose/全选确认并分别刷新 workspace/data resources。两个子事务的部分成功分栏展示，不能显示成整个批次成功。

- [ ] **Step 5: 增加“数据文件”tab。** 列表显示 display name、alias、schema、行列数、size、来源、created time、状态；所有条目都是 Data，`para` 可附加显示 schemaRef/关联案例或 import，但操作与其他资源一致。预览复用 `DataFrameTable`，normalized/original 下载按能力显示。固定按钮为查看、下载、删除、重新上传，以及当前 snapshot 明确匹配 Struct schema 时的“修改参数”；后者打开 Task 13 参数卡。alias 修改仍从自然语言进入 Agent。

- [ ] **Step 6: 显示并提交模型版本描述。** 当前模型和历史版本摘要始终显示冻结描述，空间不足时视觉截断但保留可访问全文。可变草稿的“生成版本”区域提供可选单行描述输入和 200 字符计数；空值原样交给后端生成，成功后以响应中的实际描述刷新版本列表。`deduplicated=true` 时明确提示复用了哪个现有版本及其原描述，不显示用户刚输入但未保存的新文字。切换会话或发布成功后清空输入，失败时保留便于重试。

- [ ] **Step 7: 实现删除与 replacement。** 删除第一击只打开 impact dialog；确认才 DELETE。replacement 旧资源在 processing 时仍可用；diff card 明示 schema/row/time/hash，三种 action 分别调用 confirm，终态后原子刷新 alias owner。

- [ ] **Step 8: 运行聚焦测试及 `cd frontend && npm run build`。**

- [ ] **Step 9: 提交。**

  ```bash
  git add frontend/src/types frontend/src/api frontend/src/hooks frontend/src/components/resources frontend/src/components/uploads frontend/src/components/workspace frontend/src/components/Composer.tsx frontend/src/components/ResultPanel.tsx frontend/src/App.tsx integration-tests/catalog/features.yaml
  git commit -m "feat: add dataframe resource workspace"
  ```

### Task 13: 实现研究输入卡片、受管进度和脚本按钮语义

**Files:**
- Modify: `integration-tests/catalog/features.yaml`
- Create: `frontend/src/types/studyConfiguration.ts`
- Create: `frontend/src/types/workflow.ts`
- Modify: `frontend/src/types/run.ts`
- Modify: `frontend/src/types/message.ts`
- Modify: `frontend/src/api/client.ts`
- Create: `frontend/src/hooks/useStudyConfiguration.ts`
- Create: `frontend/src/hooks/useManagedWorkflow.ts`
- Modify: `frontend/src/hooks/useRunHistory.ts`
- Create: `frontend/src/components/messages/UploadBatchCard.tsx`
- Create: `frontend/src/components/messages/StudyConfigurationCard.tsx`
- Create: `frontend/src/components/studies/FacadeChoiceCard.tsx`
- Create: `frontend/src/components/studies/ResourceBindingCard.tsx`
- Create: `frontend/src/components/studies/StructParameterCard.tsx`
- Create: `frontend/src/components/studies/ColumnMappingCard.tsx`
- Create: `frontend/src/components/studies/ResultSelectionCard.tsx`
- Create: `frontend/src/components/studies/ConfigurationSummaryCard.tsx`
- Create: `frontend/src/components/messages/WorkflowProgressCard.tsx`
- Modify: `frontend/src/components/messages/ScriptCard.tsx`
- Modify: `frontend/src/components/messages/ResultCard.tsx`
- Modify: `frontend/src/components/messages/MessageItem.tsx`
- Modify: `frontend/src/components/runs/RunHistory.tsx`
- Modify: `frontend/src/components/runs/RunHistory.test.tsx`
- Modify: `frontend/src/App.tsx`
- Add/modify matching `*.test.tsx` files.

**Interfaces:**
- Consumes: Tasks 6–11 API/messages（包括 run `scriptMessageId` 和 Task 7 `loadAround`），Task 12 upload entry。
- Produces: 对话内完整 V4 用户确认体验、Run/Retry/Cancel 行为和运行记录确认删除交互。

- [ ] **Step 1: 写失败的组件测试。** 覆盖：多个 facade 显示差异而非只显示函数名；Struct 字段按 bool/number/enum/string/date/unit 渲染并显示服务端错误；`para` 卡显示普通资源来源、当前值/修改差异、恢复默认及“覆盖/另存”动作；唯一 Data 自动预选但可更换；para reuse offer 与资源初始化顺序；结果推荐默认选择；只有完整摘要显示“确认并生成脚本”；自然语言文本不能代替按钮；进度 attempt 折叠；取消幂等；成功只列结果；失败按钮为“重新尝试”；成功按钮为“运行”。RunHistory 覆盖主区域按精确 `scriptMessageId` 定位、未加载时调用 loadAround、null anchor 显示尚未生成、详情独立展开且无嵌套按钮、窄屏关闭面板、目标滚动/聚焦/高亮；还覆盖终态删除按钮、非终态无可执行删除、先 preview 后确认、影响文案、取消无 mutation、重复点击禁用、成功移除整条 run/关闭所属 dataset、失败保留记录和错误提示。

- [ ] **Step 2: 运行 `cd frontend && npm test -- --run` 中这些新组件测试并确认失败。**

- [ ] **Step 3: 实现 upload/config 消息卡。** `UploadBatchCard` 支持 parse settings、purpose、sheet 全选/取消全选和候选诊断；`StudyConfigurationCard` 根据服务端 state 只显示当前所需子卡。所有 mutation 携 expected revision，409 `configuration_stale` 时刷新同一卡，不自行合并旧值。

- [ ] **Step 4: 实现参数、映射和结果控件。** Struct 提交规范字段值、源 resource ID/revision 和 replacement action；Data 绑定提交 stable resource ID；mapping 只提交服务端给出的目标字段和受控转换枚举；结果选择校验组合约束。最终摘要明确 function、门面实际引用的模型版本/hash/表摘要（没有 model 输入时明确“无需模型”）、所有 Struct 的 resource/alias/diff/覆盖动作、其他 Data resource/alias、result 集合和 snapshot。

- [ ] **Step 5: 实现 WorkflowProgressCard。** 非终态显示阶段、服务端 DTO 返回的执行序号/执行上限、修正序号/修正上限、动态耗时和取消；不得写死默认最大 4 次执行，合同拒绝 attempt 只增加修正序号。attempt 默认折叠，展开只显示脱敏 stderr/退出码/hash。刷新、Last-Event-ID 回放和 snapshot 以 workflow ID/revision 去重。成功显示最终脚本和 dataset list，不自动打开表/图；失败显示最后诊断和 Retry。

- [ ] **Step 6: 修改 ScriptCard 与 App handler。** 成功 Agent script 的“运行”调用不带 model ID；失败/取消/service-restarted 的“重新尝试”先创建 retry preview，输入变化时显示 diff confirmation；manual/upload 永远只显示“运行”。运行失败直接展示错误，不把按钮改成自动修正。

- [ ] **Step 7: 实现运行记录定位和删除。** Run DTO 加可空 `scriptMessageId`。运行卡用互不嵌套的主定位按钮、详情按钮和终态删除按钮；主按钮调用 App 的 `locateScriptMessage`，目标缺页时 await Task 7 `loadAround` 进入 anchored history window，渲染后通过 message ID ref 滚动、`tabIndex=-1` 聚焦并短暂高亮，聊天区提供“回到最新消息”，窄屏先关闭结果面板，失败只提示且不滚到近似消息。`useRunHistory` 加 preview/delete API 与请求代次保护；删除 preview 后用命名 dialog 展示结果数、字节和派生/提升引用影响，确认才 DELETE。成功原子失效 run/dataset/storage/lineage 缓存并关闭当前所属 DatasetWorkbench；409 `run_busy|result_busy|deletion_impact_changed` 保留记录并显示可行动诊断。

- [ ] **Step 8: 做可访问性和窄屏验证。** 原生 form 控件、label、focus trap、`aria-expanded`、dialog 名称和错误关联完整；390px 下卡片与参数表单不造成页面横向溢出，定位/详情/删除及确认框均可通过键盘操作。定位成功使用 live region 播报且 reduced-motion 下取消平滑滚动/高亮动画。

- [ ] **Step 9: 运行 `cd frontend && npm test -- --run && npm run build`。**

- [ ] **Step 10: 提交。**

  ```bash
  git add frontend/src/types frontend/src/api frontend/src/hooks frontend/src/components frontend/src/App.tsx integration-tests/catalog/features.yaml
  git commit -m "feat: add managed study conversation cards"
  ```

### Task 14: 建立 V4 确定性全链路测试和功能追溯

**Files:**
- Modify: `backend/src/testing/control.rs`
- Modify: `backend/src/testing/scenario_engine.rs`
- Modify: `backend/src/testing/scripted_agent.rs`
- Modify: `backend/src/testing/scripted_author.rs`
- Modify: `backend/src/bin/e2e-server.rs`
- Create: `integration-tests/e2e/v4-data-resources.spec.ts`
- Create: `integration-tests/e2e/v4-managed-studies.spec.ts`
- Create: `integration-tests/e2e/v4-recovery-security.spec.ts`
- Modify: `integration-tests/e2e/helpers/control.ts`
- Modify: `integration-tests/e2e/helpers/journeys.ts`
- Modify: `integration-tests/e2e/helpers/uploads.ts`
- Modify: `integration-tests/e2e/responsive-accessibility.spec.ts`
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `integration-tests/catalog/schema.json` only if V4 needs new evidence fields.
- Modify: `frontend/src/release/semantic-assertions.test.ts`

**Interfaces:**
- Consumes: 全部 V4 后端/前端行为。
- Produces: 不依赖真实 LLM/binary 的 PR gate 和 route/message/tool/skill/upload/view/error 100% 功能追溯。

- [ ] **Step 1: 审计功能目录并先登记跨层测试。** 复核 Tasks 1–13 已逐步建立的 `MODEL-VERSION-DESCRIPTION`、`UPLOAD-BATCH`、`DATA-RESOURCE`、`STUDY-CONFIG`、`MANAGED-WORKFLOW`、`SCRIPT-REPAIR`、`RUN-NAVIGATION`、`RUN-DELETION` 功能族，每个公共合同只出现一次并已引用当时存在的 Rust/Vitest 行为测试；再加入本任务将创建的 Playwright 测试符号。运行 `./scripts/check-functional-coverage --require-pr`，预期只因这些 Playwright 文件/符号尚未存在而失败。

- [ ] **Step 2: 扩展确定性控制端。** ScriptedAgent 与 ScriptedAuthor 分开排队/trace；ScenarioEngine 能产生 stderr、合同漂移、大小超限、barrier、monitor failure；控制端只在 loopback + token 下可用，不允许普通测试路由修改生产状态。

- [ ] **Step 3: 编写数据资源与模型版本浏览器旅程。** 登录→案例/DSS 导入 21 个 adapter 输出→只发布 20 张基础模型表并把 `para` 发布为普通 Data resource→另上传当前目录声明的 `network[20]` 之外模型表并发布到同一草稿/版本→用用户描述发布模型版本→修改草稿后留空描述并验证确定性摘要→以不同描述重复发布同一内容并验证复用原版本/原描述→通用 CSV/Excel/Parquet→解析歧义→sheet 全选/隐藏 sheet→混合用途→两个子事务（含一次部分成功后只重试失败侧）→数据预览/原文件与规范文件下载→自然语言重命名 alias→以稳定 resource ID 做一次派生表格和受控 ECharts 分析→把指定结果提升为普通 Data resource→`para` 与其他 Data replacement diff 三动作→删除 impact；覆盖刷新恢复、TTL、配额、恶意 archive、两用户隔离和 390px/axe。

- [ ] **Step 4: 编写受管研究旅程。** 自然语言→多门面/显式 Newton→从同源普通资源读取 `para` 并修改→选择覆盖→Struct/Data/para reuse/result 卡→最终按钮→原 alias 转移且首次 workflow 冻结新 ID/hash→初始失败→脚本自动修正→成功只列 datasets→再次覆盖 `para`→成功 Run 使用最新 Data/Struct 资源且失败不修正→失败 script Retry 输入 diff/合同 hash并产生新派生脚本/新 message anchor；另用不含 model 输入的 fixture 门面在没有 active 模型时仅凭 Data/Struct 资源完成一次研究。再制造超过首屏消息的历史，点击不同 run 验证按各自 `scriptMessageId` 加载 around context、定位/聚焦精确脚本，null anchor 不误跳。断言 active 模型没有 `para`、历史 run 仍引用覆盖前 artifact、engine 调用次数、脚本全文和 stderr 不泄漏到不应出现的消息。

- [ ] **Step 5: 编写恢复、运行删除与安全旅程。** workflow 并发拒绝、取消 barrier、页面刷新/SSE replay、服务重启收敛、终态 run 删除 preview/confirm、非终态与 result busy 阻塞、全部直接结果消失、派生/提升资源带墓碑存活、配额释放、删除后刷新、session 删除竞态、stdout/stderr 截断、result-dir 超限/符号链接、模型 provider retry、跨用户 ID 猜测全部验证稳定码和无部分发布。

- [ ] **Step 6: 运行完整确定性门禁。**

  ```bash
  cd backend
  cargo fmt --all -- --check
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --features e2e-test
  cd ../frontend
  npm ci
  npm test -- --run
  npm run build
  npm run test:e2e
  cd ..
  ./scripts/check-functional-coverage --require-pr
  ```

  全部必须通过；不允许 skip、ignore、manual-only 或仅注册表枚举来代替行为证据。

- [ ] **Step 7: 提交。**

  ```bash
  git add backend/src/testing backend/src/bin/e2e-server.rs integration-tests frontend/src/release
  git commit -m "test: cover v4 managed studies end to end"
  ```

### Task 15: 对齐 V4 文档、真实依赖门禁和发布切换

**Files:**
- Modify: `README.md`
- Modify: `AGENTS.md`
- Modify: `docs/architecture.md`
- Modify: `docs/api-design.md`
- Modify: `docs/db-design.md`
- Modify: `docs/frontend-design.md`
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/development-status.md`
- Create: `docs/v4-deployment.md`
- Modify: `scripts/test-release-engine`
- Modify: `scripts/test-release-integration`
- Modify: `integration-tests/release/real-dependencies.spec.ts`
- Modify: `integration-tests/release/semantic-assertions.ts`
- Modify: `deploy/zjudss.service` only if environment names are enumerated there.

**Interfaces:**
- Consumes: 已通过 Task 14 的最终代码与公共 manifest。
- Produces: V4 当前设计文档、空库部署说明和真实 eig-rs/DeepSeek 候选发布证据。

- [ ] **Step 1: 写失败的 release semantic assertions。** 断言 V4 bundle/version、所有新 env、无 V3 route/table/tool、`SPARROWZZ_DIR/dss/docs` 动态门面、真实目录 `enabled_count == catalog_entry_count`（不把本次基线 15 写进产品枚举）且所有传递 schema 完整、静态导入始终为 20 个基础 model + 1 普通 `para` Data resource、目录额外 model schema 可进入版本、真实 managed generation/repair/run、run→script message 定位与终态删除、Snappy outputs、无 static validation/binary-change gate、报告不含 secret/path/user credentials。

- [ ] **Step 2: 运行 `cd frontend && npm test -- --run src/release/semantic-assertions.test.ts` 和 `./scripts/check-functional-coverage --require-release`，确认文档/报告尚未对齐。**

- [ ] **Step 3: 把当前设计文档切到 V4。** API 文档列出 Task 1、4–11 的精确路由/DTO/status/error，包括模型版本可选发布描述、响应中的冻结描述与内容去重语义、run 的 `scriptMessageId`、message `aroundId` 和 run deletion preview/confirm；DB 文档列出 V4 空库表、不可变且不参与内容 hash 的模型版本描述、run→script message anchor、run/result tombstone、不可变 artifact/替换血缘/partial index；前端文档描述模型版本描述输入/显示/去重反馈、统一 Data 文件、`para` 参数卡、运行定位历史脚本和终态运行确认删除；Agent 文档列出 V4 bundle、工具和专用 ScriptAuthor；architecture 描述 21 个导入输出拆为 20 张模型表 + 1 个普通 `para` Data resource、双事务上传、确认事务和 workflow；development-status 只在所有验证通过后标记 V4 complete。历史 plans/spec 保留为记录，不成为当前合同。

- [ ] **Step 4: 编写空库部署步骤。** 停服、备份后同时移走 V3 SQLite 与 DATA_DIR，以 V4 initial migration 启动；明确不迁移历史。列出全部必需/默认 env，并明确 `OWNER_ARTIFACT_LIMIT`、`OWNER_STORAGE_LIMIT_BYTES`、`AGENT_MAX_OWNER_RUNS` 均已分别改为 `USER_ARTIFACT_LIMIT`、`USER_STORAGE_LIMIT_BYTES`、`AGENT_MAX_USER_RUNS` 且旧名不接受；说明重启 recovery 行为和 engine snapshot 生命周期。

- [ ] **Step 5: 扩展真实依赖门禁。** 检出 eig-rs 当前默认分支、构建当前 tensoreval、使用当前 `SPARROWZZ_DIR`；先验证全部 DataFrame facade 的传递 schema 完整、`enabled_count == catalog_entry_count` 且零诊断，再验证静态导入仍完整返回 21 个 adapter 输出但 zjudss 只发布 20 张基础模型表并把 `para` 发布为普通 Data resource；验证目录额外 model schema 可上传、发布和绑定；验证参数卡覆盖后 alias 指向新 artifact 且旧运行仍可读；至少对 catalog 当前存在的每个 study family 运行代表合同。用真实 DeepSeek 完成至少一个“确认→自动执行”和一个“失败→修正→成功”旅程。注入一个仅供隔离测试的坏门面时，应只记录该门面诊断且不禁用其他项；真实当前目录存在任何诊断则候选发布失败。

- [ ] **Step 6: 运行最终候选门禁。**

  ```bash
  cd backend
  cargo fmt --all -- --check
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --features e2e-test
  cd ../frontend
  npm ci
  npm test -- --run
  npm run build
  npm run test:e2e
  cd ..
  ./scripts/check-functional-coverage --require-pr
  ./scripts/test-release-integration
  ./scripts/check-functional-coverage --require-release
  git diff --check
  ```

  真实 secret/binary/resource 不可用时发布门禁必须失败，不能 skip 或回退 stub；确定性 PR 门禁仍应独立通过。

- [ ] **Step 7: 复核仓库不存在 V3 兼容残留。** `rg` 检查旧 model-import API、`OWNER_*`、`plan_study`、聊天 `create_script`、静态 script validation 和 binary hash 拦截；只允许历史 plan/spec 中出现。检查所有新增公共合同均在 manifest 和 feature catalog 中恰好一次。

- [ ] **Step 8: 提交。**

  ```bash
  git add README.md AGENTS.md docs scripts integration-tests/release deploy/zjudss.service
  git commit -m "docs: publish v4 deployment contract"
  ```

## Final Verification

- [ ] 从全新 SQLite 和空 DATA_DIR 启动一次确定性 e2e server，确认 migration、用户 seed、engine snapshot 和 startup recovery 均成功。
- [ ] 手工审阅 `git diff --stat` 和 `git status --short`，确认没有暂存用户原有无关文件，也没有生成数据库、DATA_DIR、secret、trace 或构建产物。
- [ ] 对照规格逐节勾选：关系/V3 基线、资源身份、存储生命周期、运行绑定、引擎权威、输入确认、受管执行、前端、API/Agent、全部验收要求均有对应 task 和测试。
- [ ] 只有以上门禁全部通过后，才把 `docs/development-status.md` 的 current release 改为 V4 complete。
