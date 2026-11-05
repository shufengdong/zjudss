# 研究配置、工作流、脚本与运行维护性重构设计

**日期：** 2026-11-05  
**阶段：** 仓库整理路线 8.5  
**状态：** 设计草案，待审批

## 1. 背景与意图

路线 8.1–8.4 已经收敛应用装配、公共错误、测试夹具、engine adapter、资源快照、artifact 引用、上传确认、模型版本和数据资源生命周期。下一阶段处理研究从“配置”进入“冻结、生成、执行、修正、重试和再次运行”的完整链路。

当前产品行为完整，但所有权分散：

- `study_configurations.rs` 同时负责配置状态机、资源选择、Struct 校验、结果 selector、确认快照和视图映射；
- `DataResourceService` 暂时拥有研究及 retry/rerun 的私有 Struct overlay 物化，业务上却不是资源 CRUD；
- `ManagedWorkflowRepo` 同时承担确认、输入冻结、retry 差异、attempt、终态发布、恢复和聚合读取，文件超过 2300 行；
- `RunRepo::struct_overlay_requests` 与 workflow retry 对冻结 snapshot、replacement 血缘和历史 overrides 做相近推断；
- `ManagedWorkflowService`、`WorkflowWorker` 和 `RunService` 分别重复临时输入树、artifact 交接、失败诊断和执行收敛的一部分规则；
- retry token 仍直接生成并哈希，没有复用已经建立的 `opaque_token` 权威实现；
- repository 直接依赖 application 的 `PreparedStudyConfirmation`，使配置编排与持久化边界倒置。

本阶段的目的不是改变用户流程，而是让每条规则只有一个清晰所有者，使后续修改研究输入、修正预算或运行恢复时不必同时理解多个大文件中的隐式约定。

## 2. 目标

1. 把研究配置编辑、最终确认准备和 workflow 启动分成明确边界；
2. 把私有 Struct overlay 从 `DataResourceService` 移交给专用 workflow input/overlay 服务；
3. 统一首次确认、workflow retry 和成功脚本 rerun 使用的冻结输入、replacement 血缘和 overrides 解释规则；
4. 让 retry token 复用统一 opaque token 生成、哈希和格式校验；
5. 拆分 `ManagedWorkflowRepo` 的确认、retry、attempt/终态、恢复和查询职责，同时保留一个对外 facade；
6. 让普通运行和受管 workflow 共享相同的安全输入树、engine 调用结果收集与失败诊断原语；
7. 保留普通运行与受管 workflow 各自的状态机、结果合同、修正预算、事务和权限边界；
8. 保持现有 HTTP/SSE、JSON DTO、稳定错误码、数据库 schema、Agent 权限和前端交互兼容。

## 3. 非目标与反目标

### 3.1 非目标

- 不新增、删除或改名公共路由；
- 不改变研究配置卡、最终确认按钮、retry 差异确认或普通“运行”按钮；
- 不修改 initial migration 或持久化字段；
- 不改变 `ScriptAuthor`、`FailureAnalyzer`、对话 Agent 和 engine worker 的权限隔离；
- 不改变最多 3 次修正、4 次执行的受管预算；
- 不改变不可变 script、run、input snapshot、result contract 或历史读取语义；
- 不处理结果读取、分析、比较和图表内部结构；这些属于路线 8.6；
- 不修改 Agent bundle、Prompt、工具或 skill；这些属于路线 8.7；
- 不因内部拆分而重新设计 UI；前端仅在现有行为测试或类型引用需要迁移时调整。

### 3.2 反目标

- 不创建一个同时解释研究配置、workflow、普通运行和结果分析的万能 manager；
- 不把普通运行和受管 workflow 合并成一个通用状态机；
- 不让普通运行获得自动修正能力，也不让受管 workflow 绕过最终确认门禁；
- 不把 application 规则下沉为难以测试的 SQL 条件；
- 不以移动行数作为成果；每个新模块必须对应单一业务所有权或至少两个同语义消费者。

## 4. 约束与必须保持的合同

1. 所有 tensoreval 调用继续只经过 `backend/src/adapters/`；共享执行原语只组装受控 spec，不启动进程。
2. 所有 repository 查询继续显式接收 `UserId`，跨用户与不存在资源统一 404。
3. 研究最终确认仍以精确 configuration revision、engine snapshot、facade hash、active model 和资源 hash 为门禁。
4. 同一会话仍只允许一个非终态 managed workflow；确认和 retry 幂等必须在读取可变输入前快速重放，并在事务内二次校验。
5. Struct 用户字段与结果 selector 仍分别冻结；历史 snapshot 缺少分类字段时只允许既有的 merged overrides 回退。
6. replacement 只沿显式 lineage，拒绝同 alias 冒充、循环和超过 64 层的链。
7. source-less Struct 不查询 alias 或虚构长期资源，继续复用冻结的有效 artifact。
8. 新生成的私有 overlay 必须先只进入 staging；artifact/link 创建与 workflow/run 输入冻结在同一个数据库事务中完成，不保留“事务提交后再释放临时逻辑引用”的窗口。
9. 受管终态发布仍在一个事务内绑定最终 attempt、script、message、run 和全部结果；普通 run 继续使用自己的 publication 独占事务。
10. 取消继续中断等待中的 author/engine future；已有候选与无候选两类终态恢复语义不变。

## 5. 现状证据与处理决策

| 现状 | 决策 | 原因 |
|---|---|---|
| `DataResourceService::materialize_*_struct_overlays` | 迁移到 `StructOverlayService` | 私有运行输入不是长期资源生命周期 |
| 首次确认、retry、rerun 使用同一 material 类型但由不同 repo 查询构造 | 引入统一的冻结输入解析边界和显式 overlay plan | 避免三条路径对 snapshot/lineage 做不同解释 |
| `resource_lineage.rs` 已集中 successor 与 frozen overrides 基础规则 | 保留为 repository 内核，并由单一 workflow input repository 使用 | 这些规则需要数据库查询，不能复制到 application |
| `RunRepo::struct_overlay_requests` 重复 workflow retry 查询 | 改由共享 `WorkflowInputRepo`/内部 helper 返回同一 request DTO | retry 与 rerun 必须得到一致结论 |
| repository 接收 `PreparedStudyConfirmation` | application 转成显式 `ManagedWorkflowStartCommand` | repository 不应依赖 application service DTO |
| `ManagedWorkflowRepo` 超过 2300 行 | 按确认、retry、attempt/终态、恢复/读取拆实现文件 | 每块有独立事务和测试边界 |
| `WorkflowWorker` 与 `RunService` 都构造私有输入树并调用 engine | 共享安全执行原语，保留各自 orchestration | 相同文件安全规则应统一，状态机不能混合 |
| managed 与 ordinary failure analysis 请求构造相近 | 共享脱敏/有界诊断与 analyzer 请求构造 | 同一出网安全合同只保留一份 |
| managed strict contract 与 ordinary open/frozen contract 不同 | 使用显式结果策略，共享物理校验原语 | 防止为了去重放宽 managed 合同或限制 manual script |
| retry token 自行 `getrandom` + SHA-256 | 使用 `opaque_token::{generate_token,hash_token,is_generated_token}` | 统一安全实现，同时保持只接受新 64 hex token 的公共行为 |
| `StudyConfigurationService` 1711 行 | 保留 facade，按准备、输入编辑、确认快照和纯校验拆子模块 | API 和调用者不需要感知内部拆分 |

## 6. 方案比较

### 方案 A：只拆大文件

把 study、workflow 和 run 文件按方法区段移动到子模块，不改变依赖方向。改动小，但 overlay 仍属于 DataResource，retry/rerun 仍重复解释 snapshot，repository 仍依赖 application DTO。只能降低单文件长度，不能降低业务回归风险。

### 方案 B：按领域所有权逐层整理（采用）

建立专用 Struct overlay 服务和共享冻结输入 repository；将 workflow repository 按事务职责拆分；普通运行和 workflow 只共享输入树、结果物理校验与失败诊断原语，各自保留状态机及发布事务。该方案解决真实重复点，又不会把不同产品语义抽象成一个通用执行器。

### 方案 C：合并 managed workflow 与普通 run

把两者统一为一个可配置状态机，以策略决定是否生成、修正和严格验证结果。表面去重最多，但会把最终确认、author 权限、修正预算、retry 血缘和普通按钮运行混成大量策略组合，测试矩阵和误授权风险都显著上升，因此不采用。

## 7. 目标模块所有权

### 7.1 研究配置 application

保留 `StudyConfigurationService` 作为 API、Agent 工具和 workflow 的稳定 facade，内部拆分为：

- `study_configurations.rs`：公共 DTO、错误、service 装配、稳定 facade；
- `study_configuration_prepare.rs`：门面选择、动态输入绑定、初始卡片构造；
- `study_configuration_inputs.rs`：资源绑定、Struct 编辑、列映射和结果选择；
- `study_confirmation.rs`：精确 revision 的最终确认准备、locked contract 和 overlay plan；
- `study_configuration_policy.rs`：Struct schema 校验、selector 字段、数值等价、状态计算等纯规则。

纯规则只依赖 catalog/JSON 类型，必须有直接单元测试。需要用户作用域读取的逻辑留在 service/repository，不放进 policy。

### 7.2 Workflow input 与 Struct overlay

新增 application 内部 `StructOverlayService`，依赖：

- `ArtifactService`；
- 当前 engine schema snapshot；
- 一个只读、用户作用域的 workflow input/resource repository。

它接收显式 `StructOverlayPlan`，输出 `StagedOverlaySet`：

- 可复用来源只记录已有 artifact ID/hash，不增加引用；
- 新有效值只产生 `StagedArtifact`、`ArtifactDraft` 和预期 content hash，不提前创建 artifact/link；
- 每个 material 同时包含 input ID、可空来源资源、来源 hash/schema、分类 overrides，以及“复用已有”或“待提交 staging”的封闭状态；
- 集合拥有全部 staging 文件；物化或后续事务失败时由 staging RAII 清理，不能形成逻辑配额泄漏。

计划来源只有三种显式用途：

- `InitialConfirmation`：使用配置准备出的 source/effective 值；
- `WorkflowRetry`：使用共享 resolver 解析出的最新显式 replacement 后代；
- `ScriptRerun`：与 retry 使用同一 resolver，但由 run 创建事务消费。

用途只决定调用上下文和错误映射，不改变 lineage、overrides 或 Parquet 物化算法。

`DataResourceService` 删除上述两个隐藏 materialize 方法，继续只拥有长期资源读取、更新、replacement、deletion、promotion 和下载。

### 7.3 冻结输入 repository

在 repository 层建立一个窄的 `WorkflowInputRepo`，或在不暴露新公共类型的情况下建立同名内部模块。它唯一拥有：

- 从 workflow input snapshot 读取 source/effective/分类 overrides；
- 解析 live replacement successor；
- 比较来源 ID、artifact、content hash 和 schema hash；
- 为 workflow retry 与 script rerun 生成同一 `StructOverlayRequest`；
- source-less 输入复用判定；
- legacy merged overrides 回退；
- 冻结输入行的插入与 artifact retain/release 基础 helper。

对于 staged overlay，workflow/run repository 复用已经用于结果原子发布的 `ArtifactRepo::commit_batch_with_publish` 模式：在同一个 immediate transaction 内创建或复用 artifact/link，并冻结 workflow/run input。新 artifact 的首个 link 直接归冻结 input 所有，不再额外 retain；复用已有 artifact 才按逻辑输入增加引用。事务回滚可能留下无数据库记录的 CAS 文件，但现有 GC 已负责清理这类孤儿。

`ManagedWorkflowRepo` 和 `RunRepo` 不再各自解析 snapshot JSON 或遍历 replacement 链。`resource_lineage.rs` 保留底层、数据库内的通用 lineage/override 解析，不复制到 application。

### 7.4 Managed workflow repository

保留 `ManagedWorkflowRepo` facade 和现有公共 aggregate/error 类型，拆分实现为：

- `managed_workflow.rs`：facade、聚合类型、公共错误和 owner-scoped loader；
- `managed_workflow_confirmation.rs`：确认幂等、配置 revision 复核、provisional run 和初始输入冻结；
- `managed_workflow_retry.rs`：retry replay、preview/token/impact、确认和派生 workflow；
- `managed_workflow_attempt.rs`：candidate、attempt 状态、终态 script/result 原子发布；
- `managed_workflow_recovery.rs`：取消、启动恢复和无/有候选终态收敛；
- `workflow_input.rs`：7.3 的共享冻结输入内核。

application 将 `PreparedStudyConfirmation` 转为 repository-owned `ManagedWorkflowStartCommand`。该 command 只含事务所需的稳定 ID、hash、JSON snapshot 和 material，不携带 service 或 catalog 对象。这样 repository 不再导入 application DTO。

拆文件不得拆开一个 immediate transaction。共享 helper 只接受现有连接并返回 `RepoResult`，不能在事务内重新取得连接。

### 7.5 Managed workflow application 与 worker

保留 `ManagedWorkflowService` 作为 confirm/retry/get/cancel facade，并将 retry 编排移入 `workflow_retry.rs`。构造函数改为具名依赖结构，避免当前长位置参数链。

`WorkflowWorker` 只拥有：

- author/repair 循环；
- 修正预算；
- managed attempt 状态和候选父链；
- managed strict result contract；
- managed 终态发布调用。

以下能力迁入共享执行模块：

- `execution_input_tree.rs`：从冻结 artifact 构造私有命名输入树，使用 RAII 清理根目录；
- `execution_results.rs`：结果名称、普通文件、Parquet metadata、非空值和 dtype/unit 的公共物理检查；
- `execution_failure.rs`：adapter stdout/stderr 脱敏、有界摘要、hash 元数据和 FailureAnalyzer 请求构造。

共享模块不得写 workflow/run 状态，也不得直接发布数据库结果。

### 7.6 普通运行

`RunService` 保持显式按钮创建、调度、执行一次和删除 facade。内部拆为：

- `runs.rs`：公共 DTO、错误、service facade；
- `run_execution.rs`：普通 run 的一次性 orchestration，调用共享输入树和失败原语；
- `run_results.rs`：open/frozen result policy 与普通 run 发布；
- `run_deletion.rs`：影响预览和原子 tombstone 编排；
- repository 的 run deletion 可按同样职责拆内部文件，但继续使用 `RunRepo` facade。

普通运行结果策略保持：

- manual/upload script 使用开放结果合同，只要求安全、唯一、合法的 Snappy Parquet；
- 成功 Agent script rerun 使用冻结名称/schema 合同；
- 普通运行永不进入 repair loop。

`WorkflowWorker` 的 managed strict policy 继续验证 facade 声明的精确输出顺序、dtype、unit 和实际非空值。两者共享物理读取，不共享业务判定。

### 7.7 脚本边界

`ScriptRepo` 当前较小且职责集中，不为文件数量强拆。`ScriptService` 保持文本/上传创建与读取。只有以下共享关系允许调整：

- managed 终态 script 发布仍由 workflow 事务独占；
- rerun/retry 只读取不可变 script provenance 和 frozen contract；
- 脚本名、message anchor 和 source workflow/script 血缘继续在现有事务中一次绑定。

## 8. 关键流程

### 8.1 首次研究确认

1. application 先以 operation ID 查询不可变 replay；命中立即返回，不读取当前可变资源。
2. `StudyConfigurationService` 以用户、configuration ID 和 expected revision 准备 locked contract 与 overlay plans。
3. `StructOverlayService` 解析 plans，得到只含已有引用或 staged Parquet 的 `StagedOverlaySet`。
4. application 构造 `ManagedWorkflowStartCommand`，连同 staged overlays 交给 repository 的 artifact batch transaction。
5. transaction 再次复核 configuration、active model、resource hashes、单 live workflow 和 operation ID；随后原子提交新 artifact/link、冻结输入、创建 workflow、progress message 与 provisional run，并确认配置。
6. transaction 成功后 staging 已发布且引用直接归冻结 input；失败或重放未消费的 staging 由 RAII 清理。只有新建 workflow 才启动 worker。

### 8.2 Retry preview 与确认

1. 先按 script/clientRetryId 查询 replay；命中直接返回。
2. 共享 resolver 比较 frozen source 与当前显式 replacement successor。
3. 无变化时在一个事务内直接创建派生 workflow，并复用有效 artifact。
4. 有变化时保存带完整 impact hash 的短期 preview；token 由 `opaque_token` 生成。
5. confirm 校验新格式 token、snapshot/facade/impact，读取同一 resolver 的 overlay plans。
6. staged overlay 由 retry artifact batch transaction 二次复核、原子提交并创建派生 workflow；并发重放不提交 staging。

### 8.3 成功 Agent 脚本 rerun

1. 先按 script/clientRunId 查询 replay。
2. 共享 resolver 为所有来源已变化的 Struct 构造与 retry 相同的 overlay plans；无来源输入复用冻结 artifact。
3. `StructOverlayService` 生成 staged overlay，`RunRepo::create_with_overlay_artifacts` 在 artifact batch transaction 内复核、提交并冻结完整 run input snapshot。
4. 成功后新 artifact 引用直接归 run input；失败或重放未消费 staging 自动清理，然后普通 run executor 只执行一次。

### 8.4 Worker/Run 执行

1. 各自状态机取得冻结 context。
2. 共享输入树模块复制只读 artifact，生成受控 relative paths 与 named inputs。
3. 各自通过 adapter 调用 engine；取消由现有 scheduler/session token 中断 future。
4. 共享结果模块读取物理 metadata，各自结果 policy 决定接受、修正或失败。
5. 共享失败模块生成脱敏诊断；managed workflow 可进入 repair，普通 run 直接终态失败并异步补充分析。
6. 各自 repository 保持独立终态发布事务。

## 9. 错误、恢复与兼容性

### 9.1 稳定错误

以下 code 与 HTTP 状态不得改变：

- `not_found`
- `configuration_stale`
- `workflow_in_progress`
- `idempotency_conflict`
- `study_contract_changed`
- `retry_input_unavailable`
- `invalid_retry_preview`
- `retry_impact_changed`
- `script_generation_model_unavailable`
- `repair_model_unavailable`
- `result_contract_invalid`
- `run_queue_full`
- `run_busy`
- `invalid_deletion_impact_token`

内部 error 类型可以按所有权拆分，但 API 映射必须继续显式、可穷举。

### 9.2 Artifact 引用

- overlay 计算阶段只创建 staging，不创建逻辑引用；
- artifact/link 创建与冻结 input 插入使用同一个 `commit_batch_with_publish` immediate transaction；
- 新 artifact 的首个 link 直接归冻结 input，已有 artifact 按每个新冻结输入 retain 一次；
- replay 在物化前的快速路径不创建 staging，事务内并发 replay 也不得提交 staging；
- 物化或事务失败由 staging RAII 清理临时文件；事务回滚后的孤立 CAS 文件由现有 GC 清理；
- 不存在“业务事务已经提交，但随后释放临时逻辑引用失败”的成功后清理步骤；
- 事务失败不得留下 workflow/run/input snapshot 或永久增加配额。

### 9.3 恢复

- 启动恢复继续把非终态 workflow/attempt/run 收敛为已有稳定状态；
- 已有候选时发布最后候选并绑定 script/message，未产生候选时保持脚本字段为空；
- pending ordinary failure analysis 继续单向收敛为 unavailable；
- 本阶段不修改 recovery 扫描入口和启动顺序，只调整内部模块归属。

### 9.4 历史兼容

- frozen snapshot 的 current 分类格式保持不变；
- legacy snapshot 缺少 `user`/`selector` 分类时继续读取 merged overrides；
- retry preview 仍只接受当前 64 位小写 hex token，不扩大为数据资源 legacy token；
- 不重写已发布 workflow、script、run、input snapshot 或 artifact。

## 10. 测试策略

### 10.1 先补行为刻画

实施前新增或补强以下行为矩阵：

1. 首次确认、retry 和 rerun 对同一 replacement 链得出相同 source/effective 结论；
2. 同 alias、循环和超过 64 层 lineage 在 retry/rerun 中都拒绝；
3. source-less、source unchanged、source replaced 和 legacy merged overrides 四类输入在两条重放路径一致；
4. overlay 第 N 项 staging 失败清理前 N-1 项 staging，且数据库配额不变；
5. repository 失败、幂等 replay 和并发 replay 都不提交未消费 staging、不漂移引用计数；
6. retry token 格式、hash、过期和 impact changed 保持原 code；
7. managed 与 ordinary 执行共享物理安全校验，但 result policy 保持不同；
8. 取消在 author、engine 和 repair 三个阶段都停止后续动作；
9. 有候选/无候选启动恢复与 script message anchor 保持不变；
10. 失败分析器错误或超时不改写终态和已保存日志。

测试应优先通过公共 service/API 和真实 SQLite transaction 证明行为；纯函数 policy 使用单元测试。不得用“枚举存在”或私有模块路径断言替代行为证据。

### 10.2 聚焦回归

至少运行：

- `managed_workflows`
- `workflow_retries`
- `script_repair`
- `simulation_runs`
- `study_configurations`
- `scripts`
- `session_deletion`
- `failure_atomicity`
- `workspace_tools`
- 相关 application 单元测试

若前端 DTO 和行为无变化，不新增前端实现；仍运行现有 workflow、study card、run history Vitest 和相关 Playwright 旅程。

### 10.3 阶段门禁

遵循根 `AGENTS.md`：

```bash
cd backend
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --features e2e-test

cd ../frontend
npm test -- --run
npm run build
npm run test:e2e

cd ..
./scripts/check-functional-coverage --require-pr
```

当前真实 `SPARROWZZ_DIR` 的 7 个 `case_projection_invalid` 继续作为外部资源阻塞独立报告，不得通过放宽资源合同使门禁变绿。

## 11. 文档与功能目录

本阶段默认无公共合同变化，因此：

- 更新 `docs/development-status.md` 的模块所有权和实际验证证据；
- 如事务所有权描述变化，更新 `docs/db-design.md`；
- 如内部流程描述更清晰，更新 `docs/api-design.md` 和 `docs/integration-testing-design.md`，但不复制精确枚举；
- 公共行为不变时不修改 `docs/frontend-design.md` 和 `integration-tests/catalog/features.yaml`；
- 若实施发现必须改变路由、DTO、错误码、schema、消息、工具或 UI，停止当前计划，先修订本设计并补功能目录与行为测试。

## 12. 实施边界与提交顺序

详细步骤在设计获批后单独编写实施计划。计划应按以下独立提交边界展开：

1. 行为刻画和失败注入测试；
2. retry token 与纯配置策略收敛；
3. `StructOverlayService`、staged overlay 与 artifact/input 原子提交；
4. 共享 frozen input/lineage repository；
5. managed workflow repository 分层；
6. study configuration 与 workflow application 分层；
7. 共享执行输入、结果物理校验和失败诊断原语；
8. ordinary run/managed worker 迁移及恢复回归；
9. 文档、全仓门禁和独立审查。

每个提交必须保持可编译并运行对应聚焦测试。不得在一个提交中同时移动所有大文件，也不得让中间状态绕过用户作用域、事务或 adapter 边界。

## 13. 完成标准

1. `DataResourceService` 不再拥有 workflow 私有 overlay 物化；
2. retry 与 rerun 不再各自解析 frozen snapshot/replacement lineage；
3. repository 不再依赖 `PreparedStudyConfirmation` application DTO；
4. retry token 只使用统一 opaque token 实现；
5. `ManagedWorkflowRepo` 的确认、retry、attempt/终态和恢复事务有清晰文件所有权；
6. managed 与 ordinary execution 共享安全输入树、物理结果读取和失败诊断原语，但业务状态机与发布事务仍分离；
7. staged overlay 在成功、失败、重放、并发和部分 staging 失败中均有 artifact/link/配额行为测试；
8. 现有 HTTP/SSE、DTO、错误码、schema、Agent 权限和前端流程不变；
9. 聚焦测试、完整确定性门禁和独立代码审查通过；
10. 当前 eig-rs 外部阻塞仍被准确、独立报告。
