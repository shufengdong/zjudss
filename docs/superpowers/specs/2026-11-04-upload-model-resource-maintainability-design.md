# 上传、模型版本与数据资源可维护性重构设计

**日期：** 2026-11-04  
**阶段：** 仓库整理路线 8.4  
**状态：** 已实施并完成全仓验证；候选发布仅受已记录的真实 eig-rs 外部资源断言阻塞

## 1. 背景

V4 已经具备上传批次解析、候选用途确认、模型草稿发布、数据资源发布与替换、案例整体切换、结构化资源预览替换和删除影响确认等完整行为。当前问题不是缺少功能，而是这些行为的所有权逐渐混在一起：

- `DataResourceService` 同时负责资源读取/更新、研究参数 overlay、上传确认幂等、模型发布、资源发布、案例切换、receipt 构造和 staged 文件清理；
- `DataResourceRepo` 除资源持久化外，还直接拥有上传确认锁和上传批次收尾；
- `UploadBatchService` 为了恢复和过期处理反向依赖 `DataResourceRepo`；
- 普通上传和案例整体切换共享大量流程，但策略差异通过多处 `source_kind == "case"` 条件表达；
- alias 规范化在上传与数据资源模块重复实现；
- 数据资源删除/替换预览 token 使用可预测哈希，而模型版本删除使用操作系统随机 token；
- 大文件中的解析、确认、资源生命周期和 Parquet 编解码边界不清晰。

这些问题会增加并发确认、失败恢复和后续修改 DTO 时的回归风险，也让测试不得不绕过应用服务直接操作错误归属的 repository。

## 2. 目标

本阶段完成以下目标：

1. 为上传确认建立单一应用服务和单一 repository 所有者；
2. 把普通上传与案例切换的差异收敛为显式发布策略；
3. 让幂等 claim、执行锁、失败恢复、子发布和 receipt 构造只有一条实现路径；
4. 让上传候选用途与状态转换由集中策略校验，不在应用服务和 repository 间重复猜测；
5. 统一 Rust 后端的资源 alias 规范化基础规则；
6. 统一模型版本与数据资源预览确认使用的安全 opaque token；
7. 按职责拆分上传处理、数据资源 replacement/deletion 与 repository 事务代码，降低单文件复杂度；
8. 保持现有 HTTP 路由、JSON DTO、稳定错误码、数据库 schema 和前端交互兼容。

## 3. 非目标

- 不改变上传入口、轮询方式、确认按钮或数据资源工作台交互；
- 不重新设计模型草稿/版本的业务语义；
- 不修改研究配置 overlay 和 workflow 重试语义，这属于路线 8.5；
- 不修改结果、分析、图表链路，这属于路线 8.6；
- 不引入任意 ECharts option、服务器路径、SQL 或命令参数；
- 不新增数据库迁移，除非实施时发现无法通过现有字段保持并发语义；若发生这种情况必须先回到设计审查；
- 不为文件数量而创建通用 `utils`、万能 repository 或无业务含义的 manager。

## 4. 现状证据与处理决策

| 现状 | 决策 | 原因 |
|---|---|---|
| `DataResourceService::confirm_upload` 约 600 行，编排两个子发布和案例切换 | 删除该职责并迁移到 `UploadConfirmationService` | 上传确认是跨聚合用例，不是数据资源 CRUD |
| `DataResourceRepo::{claim,begin,end}_confirmation` 和 `finish_batch` | 迁移到 `UploadConfirmationRepo` | 这些方法操作 `upload_batches`，不属于数据资源 repository |
| `DataResourceRepo::{publish_models,publish_resources}` 与 `case_switch.rs` | 作为 `UploadConfirmationRepo` 的事务策略实现复用/迁移 | 保留已验证 SQL 语义，只调整所有权 |
| `UploadBatchService` 依赖名为 `confirmation_repo` 的 `DataResourceRepo` | 改为依赖 `UploadConfirmationRepo` | 消除反向领域依赖 |
| 普通上传与案例上传通过散落的 `source_kind` 分支区分 | 抽象为封闭的 `PublicationStrategy` | 差异需要显式、可穷举、可单测 |
| 上传与数据资源各有一份 `normalize_alias` | 提取 `resource_alias` 领域策略 | 同一后端规则只保留一个实现 |
| 前端 `rssIdentifier` | 保留 | 前端即时建议与后端权威校验是不同信任边界 |
| 数据资源 token 为时间/PID 哈希 | 删除并改用 OS 随机 opaque token | token 是确认凭据，不应可预测 |
| 模型版本 token 已使用 OS 随机字节 | 复用统一 token 模块 | 消除格式与哈希重复 |
| `DataResourceService` 中的研究 overlay | 保留到 8.5 | 避免跨阶段扩大修改面 |
| 现有 REST DTO 与前端组件 | 复用 | 当前行为测试完整，没有证据要求公共契约变化 |
| `DataResourceRepo::list` 的 session 约束 | 保留一次资源筛选，并保留独立的用户会话所有权检查 | 当前实现不存在重复 query predicate；两处检查职责不同 |

## 5. 方案比较

### 方案 A：只做机械拆文件

把大文件按行数切成若干模块，但继续让 `DataResourceService` 和 `DataResourceRepo` 拥有上传确认。风险低，但错误所有权、循环认知和重复推断全部保留，不能达到本阶段目标。

### 方案 B：内部所有权优先，保持公共契约（采用）

新增上传确认应用服务与 repository，提取显式发布策略、alias policy 和 opaque token；按业务职责拆分大模块。HTTP、JSON 和 schema 不变。它能直接解决当前维护问题，同时把迁移风险限制在后端内部和测试装配层。

### 方案 C：同时重做 REST DTO、前端交互和数据库状态模型

可以得到更“整齐”的新接口，但当前协议已被前端、集成测试和历史恢复依赖。没有功能缺口支撑这种迁移成本，且会把重构与产品变化混在一起，因此本阶段不采用。

## 6. 目标架构

### 6.1 应用层所有权

新增 `backend/src/application/upload_confirmation.rs`：

- `UploadConfirmationService`：上传确认用例的唯一入口；
- `UploadConfirmationRequest`：应用层内部命令，包含 batch、decisions、replacement action、expected draft 与 client operation id；
- `UploadConfirmation`：保持现有序列化字段不变；
- `UploadConfirmationError`：映射到现有稳定错误码；
- `PublicationStrategy`：封闭枚举，仅含 `OrdinaryUpload` 与 `CaseSwitch`。

服务按固定管线执行：

1. 规范化并哈希请求；
2. 认领幂等 operation；
3. 通过上传候选策略校验并持久化 decisions；
4. 获取执行锁，或等待同 operation 的执行者；
5. 根据 replacement action 处理取消或继续；
6. 提交所需原始文件 artifact；
7. 由 `PublicationStrategy` 执行模型/资源子事务；
8. 记录失败诊断，但保留已经成功的普通上传子事务；案例策略保持整体切换语义；
9. 完成 batch、释放 reservation 和 staged 文件；
10. 从持久化状态构造唯一 receipt。

API handler 改为调用 `state.upload_confirmation_service.confirm(...)`，随后仍发布同一条 upload message。响应仍为：

```json
{"uploadConfirmation": {"uploadBatchId": "..."}}
```

`DataResourceService` 只保留资源查询、下载、更新、promotion、replacement/deletion 与暂时保留到 8.5 的 struct overlay 能力。

### 6.2 普通上传与案例策略

`PublicationStrategy` 不做动态 trait 注入，使用封闭枚举和穷举 `match`，因为策略数量固定且共享同一事务模型。

- `OrdinaryUpload`
  - 保持现有执行顺序：先把模型候选写入草稿，再发布数据资源；
  - 模型与资源是可独立完成的子发布；
  - 一个子发布失败时保留另一个已发布结果；
  - 支持 replace/save-as-new/cancel；
  - retry 只补做未发布子项。
- `CaseSwitch`
  - 模型激活、旧案例资源退休和新案例资源发布由一个数据库事务完成；
  - `cancel` 与现状一致，在选择策略前直接丢弃批次；
  - 为兼容既有 API，误传的 `replace`/`save_as_new` 仍参与幂等 payload hash，但发布时显式忽略，不新增错误码；
  - alias 冲突时保持旧案例整体可用；
  - receipt 从同一持久化结果构造，不由 handler 推断。

策略选择只在读取 batch 后发生一次。后续代码不得再次比较 `source_kind == "case"` 来决定发布语义；允许在展示诊断或构造 lineage 时读取 source kind。

### 6.3 Repository 所有权

新增 `backend/src/db/repositories/upload_confirmation.rs`：

- `UploadConfirmationRepo` 持有 `DbPool`；
- 拥有 claim/begin/end confirmation、mark child failed、finish batch；
- 拥有普通模型发布、普通资源发布和案例整体切换的事务入口；
- 拥有这些事务专用的 input/result 类型；
- 每个入口显式接收 `UserId`，继续统一跨用户与不存在为 404；
- 复用现有 SQL 和事务断言，不改变表结构和并发语义。

`DataResourceRepo` 只拥有数据资源自身的查询、更新、promotion、replacement、deletion 和按 source batch 查询。`VersionRepo` 继续拥有独立的“草稿快照为不可变版本”、激活和版本删除。当前名为 `publish_models_on` 的上传步骤实际只是把已解析模型候选写入 workspace draft，并没有创建版本；它应改名为 `apply_model_candidates_to_draft` 并归 `UploadConfirmationRepo`。两条路径只复用已经存在的 artifact reference 和用户作用域查询 helper，不合并内容哈希、去重、激活、草稿删除或 `publish_receipts`，避免把不同幂等边界误抽象为同一命令。

`case_switch.rs` 改为 `UploadConfirmationRepo` 的实现模块。若直接迁移会导致循环引用，则保留独立文件，但类型所有者必须是 `UploadConfirmationRepo`。

### 6.4 上传批次与候选策略

新增 `backend/src/application/upload_candidate_policy.rs`，只包含纯规则：

- 哪些 batch 状态允许更新 parse settings、候选和确认；
- hidden/selected/purpose 的合法组合；
- model table/data resource/skip 的 alias 与 table name 要求；
- replacement batch 的 action 要求；
- 状态与两个 child publish status 如何形成最终 batch 状态。

应用层在写入前给出稳定业务错误；repository 仍保留 CAS/事务断言作为并发安全最后防线，但不再复制用户可读推断。repository 错误只报告状态冲突代码，不构造业务 receipt 或前端文案。

`uploads.rs` 保留 public DTO、错误、service facade 和生命周期入口。解析/投影实现迁移到 `upload_processing.rs` 的内部 `UploadProcessor`，依赖被显式打包，避免 service 文件继续同时承担状态机与文件格式解析。该拆分不改变 `tensoreval` 调用边界，所有引擎调用仍通过 `backend/src/adapters/`。

### 6.5 Alias 规则

新增 `backend/src/application/resource_alias.rs`：

- `normalize_alias`：保持现有后端语义（trim、ASCII 小写、空格和连字符转下划线）的唯一 Rust 实现；
- `default_alias`：由显示名产生默认 alias；
- `validate_identifier`：仅负责语法；
- `ResourceAliasPolicy`：结合当前 engine snapshot 校验 facade input 与保留名称。

上传候选、资源 rename、replacement 和启动时 revalidation 共享该策略。数据库继续存储用户原始 alias 与 normalized alias。前端的即时 normalization 保留，但后端结果始终为权威。

### 6.6 Opaque confirmation token

新增 `backend/src/application/opaque_token.rs`：

- 使用操作系统 CSPRNG 生成 32 字节、64 位小写 hex token；
- 只在响应中返回明文；数据库只存 SHA-256 hash；
- 提供生成、长度上限校验与 hash helper；新 token 固定为 64 位小写 hex；
- 数据资源删除、结构化 replacement 和模型版本删除共用；
- token 仍是 opaque 字符串，HTTP DTO 不变。

部署时数据库中可能仍有最长 10 分钟有效的旧 `dr-delete_...` / `dr-replace_...` token。确认入口在迁移期必须继续对非空且长度受限的旧 token 计算 hash，不能只因不是 64 位 hex 就拒绝；新生成 token 的格式测试与提交 token 的兼容验证分开。旧 preview 自然过期或被消费后无需保留双格式生成逻辑，也不在启动时强制清理未过期 preview。

运行删除和会话删除在各自阶段迁移到该 helper；本阶段不因代码去重而改变它们的业务流程。

### 6.7 数据资源生命周期模块

保持 `DataResourceService` 这个对外 facade，但将实现按职责放入内部模块：

- `data_resources.rs`：DTO、错误、service 装配、查询/更新 facade；
- `data_resource_replacement.rs`：structured preview/confirm/expiry；
- `data_resource_deletion.rs`：impact preview/token/confirm delete；
- `data_resource_parquet.rs`：Struct preview 的 Parquet 编解码；
- `study_struct_overlays.rs`：保持现状，8.5 再处理。

repository 同样拆为资源主体、replacement 和 deletion 实现文件，但继续使用同一个 `DataResourceRepo` 类型。拆分后的模块不得互相复制 owner check、impact hash 或 artifact release 逻辑。

### 6.8 模型版本模块

`VersionService` 的 publish/activate 行为保持不变。删除预览改用统一 opaque token。版本描述 normalization/default 仍属于版本应用层，不与资源 alias 规则混合。

`VersionRepo` 本阶段只在能保持事务 helper 单一来源时拆分 deletion 实现；不为了行数强行移动紧密耦合的 workspace/version 查询 helper。

## 7. 错误与恢复语义

以下稳定错误码和 HTTP 语义保持不变：

- `not_found`
- `upload_batch_expired`
- `upload_candidate_invalid`
- `upload_state_invalid`
- `idempotency_conflict`
- `draft_revision_conflict`
- `resource_alias_conflict`
- `replacement_preview_invalid`
- `invalid_deletion_impact_token`
- `deletion_impact_changed`
- `resource_stale`
- `storage_quota_exceeded`

恢复规则：

- 未开始执行的 claim 不可被恢复为 confirmed；
- 已开始且子发布已提交的 operation 由 startup recovery 收尾；
- active confirmation 阻止 discard/expiry/candidate mutation；
- 同 operation + 同 payload 返回同一持久化 receipt；
- 同 operation + 不同 payload 返回 idempotency conflict；
- staged 文件清理失败不回滚已提交数据库事务，后续 recovery/expiry 可再次清理；
- artifact 引用提交必须幂等，竞态下 ref count 不得增加两次。

## 8. 测试设计

### 8.1 新增/迁移的窄测试

- `resource_alias`：normalization、非法首字符、空值、保留名、snapshot 变化 revalidation；
- `opaque_token`：格式、随机性、hash 稳定性、非法 token 拒绝；
- `upload_candidate_policy`：每个可变状态、用途组合、replacement action、最终状态真值表；
- `UploadConfirmationRepo`：claim/begin/end 单执行者、payload 冲突、恢复和 finish；
- `UploadConfirmationService`：同 operation 并发、confirm/discard 竞态、普通双子事务部分成功、案例整体失败、receipt replay；
- 普通上传：锁定“先模型草稿、后数据资源”的顺序及 alias 冲突后的部分成功 receipt；
- 案例上传：`cancel` 丢弃，以及 `replace`/`save_as_new` 被显式兼容忽略且仍参与幂等 hash；
- data resource replacement/deletion：过期、stale、impact changed、artifact release；
- token 迁移：新 token 为 64 位 hex，部署前生成且尚未过期的前缀 token 仍可确认；
- version deletion：新 token helper 下原行为不变。

现有直接调用 `DataResourceRepo` confirmation 方法的测试迁移到 `UploadConfirmationRepo`；现有直接调用 `data_resource_service.confirm_upload` 的行为测试迁移到 `upload_confirmation_service.confirm`。测试只因所有权变化而改名，不降低断言。

### 8.2 回归门禁

按风险递增执行：

```bash
cd backend
cargo fmt --all -- --check
cargo test --features e2e-test --test upload_batches
cargo test --features e2e-test --test data_resources
cargo test --features e2e-test --test model_versions --test workspace_versions --test version_deletion
cargo test --features e2e-test --test case_import --test model_imports
cargo clippy --all-targets --all-features -- -D warnings
cargo test --features e2e-test

cd ../frontend
npm test -- --run
npm run build

cd ..
./scripts/check-functional-coverage --require-pr
```

真实依赖门禁继续遵循 `docs/v4-deployment.md`。本阶段不放宽当前已知的 eig-rs 相邻案例断言。

## 9. 文档与功能目录

因为公共路由、消息类型、工具、skill、上传格式、视图和稳定错误码均不改变，本阶段不新增功能目录条目。需要同步更新：

- `docs/development-status.md`：记录新所有权、验证命令和残余问题；
- `docs/api-design.md`：仅澄清上传确认由独立应用服务编排，外部协议不变；
- `docs/db-design.md`：澄清 upload confirmation transaction 的 repository owner；
- `docs/frontend-design.md`：若无行为变化，只记录兼容性验证，不改交互规范；
- 本设计与对应实施计划。

如果实施导致公共行为变化，必须先停止并补充设计、功能目录和行为测试，不能把变化伪装成内部重构。

## 10. 实施边界与完成标准

完成标准：

1. API、startup recovery 和测试均通过 `UploadConfirmationService` / `UploadConfirmationRepo` 的清晰边界工作；
2. `DataResourceService` 不再包含上传幂等、模型发布、案例切换或 batch finish；
3. `DataResourceRepo` 不再操作 confirmation claim/lock 或模型发布；
4. 普通上传/案例切换只有一处策略选择；
5. alias 与本阶段 token 无重复 Rust 实现；
6. 直接 repository 状态测试与应用行为测试均覆盖并发和恢复不变量；
7. public DTO、稳定错误码与 schema 兼容；
8. 格式、静态检查、后端测试、前端测试/构建与功能覆盖门禁给出可复现结果；
9. 未触碰 8.5 以后子系统的业务语义。

## 11. 实施记录与设计偏差

实现保持了本设计的公共兼容边界，并形成以下实际文件所有权：

- `UploadConfirmationService` 是唯一确认编排者；为避免复制既有错误映射，`UploadConfirmationError` 使用
  `DataResourceError` 的公开类型别名，稳定 code 和 HTTP 映射不变。
- `UploadConfirmationRepo` 拥有确认锁与事务入口；低层模型候选/资源发布函数位于其私有
  `upload_confirmation_transactions.rs` 子模块，案例切换复用同一函数来源。
- `UploadProcessor` 只持有上传 repository、artifact service、engine adapter、snapshot、数据目录和归档限制；案例切换基线通过
  延迟 future 在原处理时点读取，不把 confirmation repository 变成处理器依赖。
- `DataResourceService` 继续是唯一公共 facade，replacement/deletion/Parquet codec 通过物理子模块实现且不放宽 service 字段可见性；
  repository 仍是一个 `DataResourceRepo`。`VersionRepo` 继续独立拥有发布，删除实现位于 `version_deletion.rs`。
- 设计中提到的 `DataResourceRepo::list` 重复 session filter 在实施基线中不存在：当前只有一个资源 `session_id` 谓词，另一个
  `ensure_session` 是必要的用户所有权验证，因此没有删除任一检查。

定向验证已覆盖上传解析/确认、案例切换、数据资源替换/删除、研究 overlay、受管工作流和版本删除；新增特征测试明确区分
“不支持的解析设置保持旧候选”与“实际重新解析失败清理旧 artifact”，并覆盖版本 busy、无效/过期 token 和 artifact 释放。
本阶段没有公共路由、消息类型、工具、skill、上传格式、视图、稳定错误码或数据库 schema 变化，因此
`integration-tests/catalog/features.yaml` 和 `docs/frontend-design.md` 不需要修改。完整仓库验证及既有真实 eig-rs 外部门禁状态由最终验证阶段记录。
