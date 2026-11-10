# 后端维护性整理与 API 源码文档设计

## 背景与意图

当前 V4 后端的公共合同、分层边界和领域所有权已经稳定，但源码可读性不均衡：

- `backend/src/api/` 当前以 80 个公开异步 handler 加私有健康检查承载 81 条唯一 method/path，只有少量 handler 带 `///` 文档注释；
- 部分模块缺少 `//!` 级职责说明，读者需要跨文件追踪认证、领域调用和错误映射；
- 后端约 5.98 万行，已有若干超过 700 行的模块，其中一部分是合理的领域聚合，另一部分仍可通过职责提取、命名和重复逻辑收敛提升可维护性；
- `docs/api-design.md`、`docs/architecture.md` 和 `docs/development-status.md` 已是当前权威文档，但源码入口、文档导航和最近维护性基线仍可进一步对齐。

本轮工作面向后端维护者。目标是在不改变任何产品行为或公共合同的前提下，让维护者能从 API handler 直接理解接口语义，并让后端各层的职责、关键不变量和权威文档入口更容易发现。

## 目标

1. 为所有实际注册的 Axum HTTP/SSE handler 补齐准确、结构一致的 Rust `///` 文档注释。
2. 为后端重要模块、公共 facade、跨层边界和难以从类型直接看出的不变量补充 `//!` 或 `///` 文档。
3. 在整个 `backend/` 内进行证据驱动、行为保持的维护性整理：改善命名、缩小职责、收敛重复逻辑、删除确认无消费者的死代码。
4. 同步当前设计文档与开发状态，使源码、路由注册表、设计说明和验证证据相互一致。
5. 保持每一批改动可独立审查、可独立验证，并避免一次性机械改写整个后端。

## 非目标

- 不新增、删除或修改 HTTP 路由、方法、DTO、状态码、稳定错误码或 SSE 事件格式。
- 不修改数据库 schema、migration、持久化语义或空库初始化合同。
- 不修改 Agent bundle、工具权限、Prompt、skill、LLM provider 或引擎合同。
- 不修改 tensoreval 调用方式、命令参数、Parquet 事实来源或资源 snapshot 规则。
- 不改变前端行为，不要求前端配套修改。
- 不引入 OpenAPI、Swagger、`utoipa` 等新依赖或生成流程。
- 不以文件行数为唯一理由拆文件，不为每个内部字段生成低价值注释，也不启用全 crate 的 `missing_docs` 门禁。
- 不顺带修复与维护性无关的产品需求或领域行为问题。

## 已确认约束

- 工作范围覆盖整个 `backend/`，而非仅 `backend/src/api/`。
- 所有改动必须严格保持现有 HTTP、数据库、Agent、引擎和前端行为。
- 所有 tensoreval 调用继续只通过 `backend/src/adapters/`。
- repository 查询继续显式接收 `UserId`，跨用户与不存在资源继续统一为 404。
- 浏览器和 LLM 继续只接触稳定 ID 与受控 spec，不暴露路径、命令、SQL 或任意 ECharts option。
- 当前工作区已有的 Agent manifest/skill index hash 修复不属于本轮设计内容；实施时必须保留并与本轮提交隔离。

## 考虑过的方案

### 方案一：注释与文档优先

只为 API handler、模块和少量 facade 补注释，并更新设计文档。优点是风险最低、审查容易；缺点是不能解决后端内部仍然存在的职责模糊、命名不清和安全重复问题，无法完整满足“整理代码”。

### 方案二：分阶段维护性整理（采用）

先建立 API 注释和模块文档基线，再按 API、application、adapter/engine resources、repository 顺序逐批整理。每批只处理有明确所有权收益的提取或重复收敛，并运行分层验证。该方案兼顾完整性、风险和可审查性。

### 方案三：全量公共项文档化

为约 787 个 `pub`/`pub(crate)` 项补文档并启用 `missing_docs`。优点是形式覆盖率高；缺点是会产生大量重复类型签名的低价值注释，显著增加审查噪声，也会迫使测试支撑和内部组合类型承担不必要的公共文档承诺，因此不采用。

## API 源码文档规范

### 覆盖范围

覆盖 `backend/src/api/mod.rs` 实际注册的所有 handler，包括：

- 公共健康检查与认证接口；
- 会话、消息和 SSE；
- 上传、案例、模型工作区和版本；
- 独立数据资源；
- 研究配置与受管工作流；
- 脚本、运行、结果、分析、比较和图表；
- 下载、分页和删除影响预览接口。

不要求给每个私有解析 helper 写路由式注释；私有 helper 只在存在安全边界、所有权转换或非显然不变量时补普通注释。

### 注释结构

每个 handler 的 `///` 至少回答以下问题：

1. **路由与目的**：首句给出 method/path 和用户可观察的核心语义。
2. **鉴权/作用域**：说明是公开接口还是要求当前用户，以及跨用户资源如何处理。
3. **请求约束**：只记录无法从 extractor/DTO 类型直接看出的 revision、幂等、分页、token 或上传限制。
4. **成功结果**：说明主要状态码、响应对象以及 200/202/204 等差异。
5. **关键副作用**：说明持久化、后台任务、artifact、SSE 或 tombstone 行为；纯读接口明确只读即可。
6. **关键错误语义**：记录该 handler 特有且维护者容易破坏的稳定错误；公共错误映射继续由 `api/error.rs` 和 `docs/api-design.md` 统一说明。

建议形式：

```rust
/// `POST /api/study-configurations/{id}/confirm`：确认当前 revision，并原子创建
/// 受管 workflow 与 provisional run。
///
/// 要求当前登录用户拥有配置；请求必须携带精确 `expectedRevision` 和幂等
/// `clientOperationId`。成功返回 202，幂等重放保持现有合同。跨用户或不存在
/// 资源统一为 404，过期 revision 映射为稳定的配置冲突错误。
pub async fn confirm(/* ... */) { /* ... */ }
```

具体注释必须以当前路由注册、handler 实现、领域 service 和 `docs/api-design.md` 为证据，不能凭名称推断。

### 模块文档

每个 `backend/src/api/*.rs` 文件补充或校准 `//!`，说明：

- 本模块拥有的路由族；
- handler 与哪个 application service/repository 边界协作；
- 本模块不拥有的领域规则；
- 如适用，认证、分页、下载、SSE 或后台任务的特殊边界。

`api/mod.rs` 继续是路由注册和合同枚举的唯一入口，不再维护第二份源码路由表。

## 后端维护性整理原则

### 处理条件

代码整理项至少满足以下一项才实施：

- 同一业务规则或机械转换存在两个及以上真实实现；
- 一个模块同时拥有可独立命名、可独立测试的多个职责；
- 公共 facade 被内部细节淹没，提取后能明显展示业务编排顺序；
- 参数或返回值的命名掩盖用户作用域、不可变快照、artifact 所有权或失败原子性；
- 已确认没有消费者的死代码或兼容入口仍保留；
- 注释审计发现实现与权威文档之间存在不改变行为即可修正的表达偏差。

不满足上述条件的“看起来可以抽象”不进入本轮。

### 分层顺序

#### 1. API 层

- 补齐 handler 和模块文档；
- 统一重复的 extractor 到领域请求映射、附件响应和错误响应机械代码，但保留各领域 DTO 与错误类型；
- 确保 handler 只做 HTTP/SSE、认证提取、DTO 映射和 application 调用；
- 不把业务校验搬回 handler，也不引入通用动态路由抽象。

#### 2. Application 层

- 优先整理大型 facade 中已具备独立职责的内部阶段，使 facade 保持业务流程可读；
- 收敛重复的不可变 artifact 元数据、冻结输入、结果事件映射和受控 DataFrame 读取调用；
- 保持研究配置、工作流、普通运行、上传、分析、比较和图表的状态机相互独立；
- 不重新合并已经在 2026-11-04/05 拆开的所有权边界。

#### 3. Adapter 与 engine resources

- 校准进程控制、结果扫描、导入、临时任务文件、资源安全树、loader 和 publisher 的模块文档；
- 只提取能强化“唯一进程边界”和“不可变 snapshot 发布”的重复逻辑；
- 不改变子进程参数、超时/取消、输出捕获、路径限制或 capability probe 顺序。

#### 4. Repository 与数据库边界

- 改善事务 helper、用户作用域查询和 artifact retain/release 调用点的命名与局部结构；
- 仅收敛机械 SQL 映射或事务骨架，不合并不同领域的状态机与错误语义；
- 保持所有查询显式 `UserId`、404 隐藏、immediate transaction、幂等和失败原子性。

#### 5. 支撑模块与测试

- 为 auth、ingestion、storage、catalog、testing 的关键入口补充职责和安全边界文档；
- 删除确认无消费者的测试构造变体和死代码；
- 如果完整测试暴露可稳定复现的测试全局状态竞争，可只在测试支撑层修复隔离，不修改生产行为。

### 大文件策略

行数只用于发现候选，不作为拆分理由。以下类型的大文件通常保留：

- 由大量稳定 DTO/事件枚举构成且有单一所有权的文件；
- 自动/半自动 schema 映射；
- 大量紧邻实现的单元测试。

只有当可以给提取模块一个稳定、领域化名称，并让原 facade 的业务步骤更清晰时才拆分。新模块保持私有，除非已有外部消费者要求公开。

## 文档完善范围

### `docs/api-design.md`

- 校准路由表与 `api/mod.rs` 注册表；
- 增加源码导航，指向各 API 模块和统一错误映射；
- 明确 handler 注释、路由注册表、公共 DTO 和领域事实各自的权威边界；
- 只记录当前行为，不添加未来规划。

### `docs/architecture.md`

- 更新后端模块职责图与本轮确认后的唯一所有权；
- 记录新增的共享 helper 或模块拆分；
- 明确哪些大文件经审计后有意保留。

### `docs/development-status.md`

- 在实现和验证完成后记录维护性整理范围、提交和实际验证结果；
- 不提前把未运行的测试写成已通过；
- 若存在外部 eig-rs 或环境门禁失败，精确区分代码失败与外部状态。

### 其他文档

只有代码整理实际触及对应边界时才更新 `db-design.md`、`agent-prompt-skills-design.md`、`v4-deployment.md` 或 `integration-testing-design.md`。由于本轮不改变公共行为，默认不修改功能目录；如果实施中发现公共合同必须变化，应停止并重新确认范围，而不是顺手更新目录。

## 实施与提交策略

工作拆成可独立审查的批次，建议顺序为：

1. API handler/module 文档与 API 层小型整理；
2. application facade 和共享机械逻辑；
3. adapter、engine resources、auth/ingestion/storage/catalog；
4. repository 与测试支撑；
5. 权威文档、全局验证和开发状态收敛。

每批只包含同一层或同一所有权主题，避免把纯注释、重命名和跨层提取混在一个难以审查的提交中。实施时直接在 `master` 工作，但只暂存本轮文件；已有未提交的 Agent hash 文件必须保留且不纳入本轮提交。

## 验证策略

### 每批验证

- `cargo fmt --all -- --check`
- 对应模块的聚焦单元/集成测试；
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo doc --no-deps --all-features`，确保 Rustdoc 可生成且链接有效。

如 Clippy 或 Rustdoc 暴露本轮前已存在的问题，必须区分基线问题和新增回归；不能通过宽泛 `allow` 隐藏新增问题。

### 最终验证

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --features e2e-test`
- 从仓库根运行 `./scripts/check-functional-coverage --require-pr`

API 注释本身以以下事实交叉检查：

- `backend/src/api/mod.rs` 的 method/path；
- handler extractor 和响应；
- application service 的副作用；
- `api/error.rs` 的公开错误映射；
- `docs/api-design.md` 的当前合同。

若完整测试因共享测试状态出现并行竞争，先以可复现证据定位；允许在测试支撑层修复隔离，不能用串行测试替代默认全量门禁后宣称全绿。

## 验收标准

- 81 条实际注册的唯一 method/path 所映射的 handler 均有准确的 `///` 注释，API 模块均有职责明确的 `//!`。
- 后端关键公共 facade 和安全/所有权边界从源码即可发现，且未产生覆盖所有内部字段的噪声注释。
- 每个代码提取或重复收敛都有明确所有权收益，并有聚焦测试或现有行为测试保护。
- 公共路由、DTO、错误码、数据库 schema、Agent bundle/工具、引擎合同和前端行为无变化。
- `docs/api-design.md`、`docs/architecture.md`、`docs/development-status.md` 与最终代码和验证事实一致。
- 最终验证结果按实际输出报告；任何外部门禁或既有失败都被明确列出，不被误报为成功。

