# 引擎 Adapter、资源目录与导入维护性重构设计

## 1. 意图

这是全仓库维护性路线图的第三阶段，范围限定为 `backend/src/adapters/`、
`backend/src/engine_resources/` 及其直接测试和启动装配。目标是在不改变 HTTP/SSE、数据库 schema、Agent 工具、
上传协议或用户行为的前提下，让进程执行、结果目录安全、临时导入脚本、资源扫描和不可变 snapshot 发布各有一个
明确所有者。

本阶段只整理引擎边界和启动资源。上传批次、模型版本、数据资源、研究工作流、结果分析和图表只作为消费者回归；
它们分别留给后续 8.4–8.6 阶段。

## 2. 当前证据

- `backend/src/adapters/process/mod.rs` 为 617 行，同时负责配置、普通执行、模型 import wrapper、案例 projector
  临时脚本、退出诊断、结果目录准备和环境变量解析。
- 模型导入和案例投影各自实现 nonce、`create_new`、写入、显式清理、取消时 Drop 清理及清理失败错误拼接；其生命周期
  相同，仅文件内容、数量和稳定错误码不同。
- `monitor::scan_result_limits` 已校验目录、文件类型、路径和容量，`output::validate_outputs` 随后又重新读取目录并重复
  canonicalize、类型、路径、单文件大小和总大小检查。最终解析必须保留退出后的独立复检，但一次最终复检内不应再遍历两遍。
- `backend/src/engine_resources/mod.rs` 为 1002 行，同时读取 catalog、发现文档/案例、计算 snapshot identity、复制源文件、
  写 manifest、校验已发布目录和实现安全路径/有界读取。
- `EngineResourceRegistry::load` 接收的 `resource_features` 已完全未使用；`EngineCapabilities::resource_features` 仅用于构造
  该参数和一条枚举测试，形成“binary capability 会影响资源目录”的错误耦合。
- `STATIC_MODEL_TABLES` 明确标注为 V3 过渡别名，当前生产代码已使用 `STATIC_IMPORT_OUTPUTS` / `MODEL_TABLE_OUTPUTS`，
  只有测试仍消费旧名。
- adapter 的进程组终止、双流有界捕获、运行中监控、最终 Snappy Parquet 校验和 engine resource 的 fail-closed 规则已有
  完整行为测试，不需要重写算法。

## 3. 目标

- 最终结果目录只由一个扫描函数拥有类型、路径、文件数和容量规则，并把已验证文件清单交给 Parquet 解析。
- 运行中监控与退出后最终解析仍是两个独立时间点；退出后最终扫描不得因复用监控状态而省略。
- 临时 RSS/DSS 文件由一个 RAII 原语以 `create_new` 创建，在成功、失败、timeout 和 future cancellation 下都清理；调用方
  保留各自稳定错误码和业务脚本文本。
- `ProcessEngine` 的 trait 入口保持不变，但 import/projector 实现与普通执行编排分属聚焦模块。
- engine resource 的加载/身份计算、树安全原语和不可变发布分属聚焦模块；`mod.rs` 只保留公共导出与 registry 门面。
- 删除未使用的 resource feature 参数和 V3 常量别名，不留下兼容空壳。
- 本阶段不改变任何公共 route/message/tool/skill/upload/view/error 合同，因此不修改功能目录枚举。

## 4. 非目标

- 不改变 `SimEngine` 的行为入口、`RunSpec`/`ImportSpec`/`CaseProjectionSpec`、稳定 adapter 错误码或输出 DTO。
- 不把同步启动 capability probe 改成异步运行 worker，也不改变启动时 fail-closed 顺序。
- 不统一 capability probe 的临时目录与运行时任务树；前者是启动自检，后者是受控用户执行，生命周期和错误公开语义不同。
- 不建立 adapter 与 engine resource 共用的万能 safe-path、walker、hash 或 error abstraction。
- 不合并普通 DSS import、案例 projector 和运行结果合同；三者只复用文件/目录安全原语。
- 不改变 catalog schema、rssbundle manifest schema、snapshot ID 算法、目录限制或案例可导入判定。
- 不修改外部 eig-rs 资源树，也不放宽当前 7 个 `case_projection_invalid` 的真实资源门禁。
- 不拆上传、案例应用、研究配置、workflow、分析、比较或图表业务模块。

## 5. 方案

### 方案 A：只移动大文件，不抽取重复

可以迅速缩短 `mod.rs`，但结果目录仍重复遍历，import/projector 仍分别维护临时文件算法，所有权没有真正改善。

### 方案 B：按职责拆分并抽取最小安全原语（采用）

adapter 内新增私有结果扫描和临时任务文件所有者；engine resource 内拆出 tree、loader 和 publisher。只共享当前至少两个
消费者语义完全一致的实现，错误转换仍留在领域入口。同时移除已经没有运行时意义的 capability 参数和 V3 别名。

### 方案 C：建立跨子系统通用文件系统安全层

可以复用 canonicalize、walker、hash 和限额代码，但 adapter 任务树、resource 只读树、CAS 与 upload staging 的信任来源、
错误码和清理策略不同。统一会隐藏边界并扩大未来阶段改动，因此不采用。

## 6. Adapter 模块设计

### 6.1 结果扫描

`backend/src/adapters/process/monitor.rs` 继续拥有运行中定时器；新增的私有扫描结果类型由同目录的结果模块拥有：

- `scan_result_dir(path, limits, phase) -> Result<ScannedResultDir, AdapterError>`：一次读取并验证根目录、普通文件、无符号链接、
  canonical containment、文件数、单文件和总大小。
- `ScanPhase::Monitoring` 只把根目录/读取竞态映射为 `result_monitor_failed`；具体发现的符号链接、非普通文件和限额仍保持
  原稳定 code。
- `ScanPhase::Final` 返回按文件名排序的 canonical 文件清单和大小；`validate_outputs` 直接在这份清单上校验 `.parquet`、
  变量名、大小写不敏感唯一性、Parquet 可读性和 Snappy，不再次遍历目录或重新累计容量。

监控每个 tick 都重新扫描，进程退出后仍无条件执行一次 `Final` 扫描。成功输出不依赖最后一次 monitor tick 的缓存。

### 6.2 临时任务文件

`backend/src/adapters/process/task_file.rs` 提供私有 `TemporaryTaskFile`：

- `create(root, prefix, extension, bytes, error_code) -> Result<Self, AdapterError>` 使用随机 128-bit nonce 和 `create_new`；
- `relative_path()` 返回只含生成文件名的 `Path`，供受控 `RunSpec` 使用；
- `remove(self)` 显式删除并报告 IO 错误；Drop 在 future cancellation 或提前返回时 best-effort 删除；
- error code 由调用方提供，模型导入仍使用 `model_import_script_failed`，案例投影仍使用
  `case_projection_script_failed`。

模型 import 的 wrapper DSS 和 RSS、案例 projection 的 RSS 都使用该原语。脚本文本和 wrapper 语义仍由对应 import/projector
函数生成，临时文件原语不理解 RSS/DSS。

### 6.3 进程与导入职责

- `process/mod.rs`：`EngineConfig`、`ProcessEngine`、`SimEngine` 委托和公共导出。
- `process/execution.rs`：普通命令执行编排、结果目录准备/清理、退出诊断和 stderr 附加。
- `process/import.rs`：`ProcessEngine` 的模型 import 与案例 projection 实现，消费 `TemporaryTaskFile` 和通用 execute。
- `process/control.rs`：只拥有 spawn、双流捕获、timeout/cancel、进程组终止和运行中 monitor 竞争。
- `process/output.rs`：只拥有退出后最终结果扫描后的 Parquet/命名解析。

这不是新增公开层；`SimEngine` 的方法和所有 public re-export 保持不变。

## 7. Engine Resource 模块设计

### 7.1 `tree.rs`

拥有只读资源树的安全原语：规范相对路径、拒绝 symlink、受限读取/流式 hash、文档发现和 rssbundle manifest 发现。
它返回 `EngineResourceError`，不复用 adapter 的任务输入错误。`examples.rs` 使用这些原语读取 manifest 声明文件。

### 7.2 `loader.rs`

拥有 `load_snapshot(sparrowzz_dir, limits) -> EngineResourceSnapshot`：读取四个 catalog、校验独立 schema version、构造
schema/facade/projection registry、加载文档与案例、建立 source inventory，并按现有字段顺序计算 snapshot identity。

`EngineResourceRegistry::load` / `load_with_limits` 仅委托该函数。删除 capabilities 参数，因为资源兼容性已由 catalog/
manifest 和固定 engine contract 表达，binary capability 在 `EngineConfig::probe` 独立 fail closed。

### 7.3 `publisher.rs`

拥有生成文件清单、源 hash 复验、staging 写入、原子 rename、已存在 snapshot 完整性校验和 snapshot 路径 rebase。
发布仍先校验源 hash，再写 manifest，失败 best-effort 清理 staging；已发布目录损坏继续返回
`engine_resource_hash_mismatch`。

`EngineResourceRegistry::publish` 只委托 publisher，公共返回值不变。

## 8. 删除与保留决策

| 候选 | 决定 | 理由 |
|---|---|---|
| monitor/final 的目录类型、路径与容量校验 | 抽象 | 同一安全规则、两个时间点；phase 只影响 IO 错误映射 |
| final scan 后的第二次目录遍历 | 删除 | 同一最终时间点重复读取，已验证清单可直接解析 |
| import/projector 临时文件创建与清理 | 抽象 | `create_new`、nonce、RAII 和清理失败语义一致 |
| import wrapper 与 projector 脚本文本 | 保留 | 文件数量、函数、输出合同和稳定错误码不同 |
| runtime execute 与同步 capability probe | 保留 | 启动自检和用户任务的 timeout、诊断、目录生命周期不同 |
| adapter safe input 与 resource safe tree | 保留 | 不同信任根、错误类型和公开语义 |
| catalog facade/profile parser | 保留 | 单项禁用诊断、schema 绑定和重复 ID 政策不同 |
| `resource_features` 参数/方法 | 删除 | 加载器完全不读，制造虚假 binary/resource 耦合 |
| `STATIC_MODEL_TABLES` V3 别名 | 删除 | 无生产消费者；测试迁移到 V4 权威常量 |
| `EngineExecution` + `EngineRunOutput` 类型别名 | 删除别名层 | 直接把结构体命名为当前使用的 `EngineRunOutput` |

## 9. 错误、原子性与取消

- 所有 tensoreval 调用仍只在 `backend/src/adapters/`；命令参数继续由 `build_command` 或受控 probe 构造。
- timeout、monitor failure 和 future cancellation 必须先终止整个进程组，再清理结果目录和临时任务文件。
- 非零退出、最终扫描失败、Parquet/压缩失败和 projector 精确输出不匹配均不得留下可被上层发布的部分结果。
- 临时文件显式删除失败时，成功执行转为原稳定 script error；执行本已失败时保留主 error code，并把清理失败附加到 message。
- snapshot load 不写 `SPARROWZZ_DIR`；publish 只写 `DATA_DIR/system/engine-resources` staging，并保持 rename 原子边界。
- 扫描与发布之间源字节变化继续失败为 `engine_resource_changed_during_scan`；已有目的目录任何 inventory/hash 不一致继续失败关闭。

## 10. 测试与验证

行为测试至少覆盖：

- monitor 的 file count/file bytes/total bytes、symlink/目录、根目录消失与退出后 final recheck；
- final scan 只接受规范、大小写不重复、可读且 Snappy 的 Parquet，并返回稳定排序；
- import wrapper/RSS 与 projector RSS 在成功、非零退出、invalid path、output mismatch、timeout、cancellation 和并发执行后清理；
- capability probe 的两个 import profile 与命名 Parquet smoke 保持 fail closed；
- resource snapshot 对 catalog/document/example 字节敏感，schema 版本独立，缺失 study facade 只产生全局诊断；
- manifest 无效叶节点隔离、未声明文件不发布、source change、corrupt existing snapshot、symlink 和所有容量上限；
- 启动、release engine cases 与应用案例仍从已发布不可变 snapshot 工作。

本阶段无公共合同变化，不编辑 `integration-tests/catalog/features.yaml`。若行为测试符号重命名，仅在功能目录对应证据需要时
同步更新名称；不得用注册表枚举替代行为覆盖。

聚焦基线已经验证 `model_import_adapter` 10/10、`process_engine_v4` 17/17、`engine_resources` 除真实相邻 eig-rs 断言外
32/32 通过。阶段末运行 AGENTS.md 全量确定性门禁；精确后端全测若仍只报告已知 7 个外部案例，则跳过且仅跳过该断言
重跑其余套件。

## 11. 文档归属

- `docs/architecture.md`：记录 adapter 进程/结果/临时文件和 resource loader/publisher 的模块所有权。
- `docs/agent-prompt-skills-design.md`：仅在需要澄清 engine snapshot 与 binary capability 解耦时更新，不复制模块细节。
- `docs/v4-deployment.md`：保留启动 probe、资源只读、snapshot 发布和失败处理的运维合同。
- `docs/integration-testing-design.md`：记录结果扫描、临时文件取消清理和 snapshot 完整性的行为证据。
- `docs/development-status.md`：只记录本阶段完成项、删除的重复/陈旧别名和最近验证结果。

## 12. 完成标准

- 最终结果目录安全/容量规则只有一个扫描实现，`validate_outputs` 不再重复遍历。
- 模型 import 和案例 projector 共用一个临时任务文件原语，并维持全部成功/失败/取消清理证据。
- `adapters/process/mod.rs` 和 `engine_resources/mod.rs` 成为职责清晰的门面，不再承载全部实现。
- resource loader 不再接收未使用的 binary features；V3 常量别名和执行结果类型别名被删除。
- 无 route、DTO、schema、Agent、UI、catalog/manifest 或稳定错误码变化。
- 聚焦与全量门禁通过；唯一单独报告的失败仍是已知相邻 eig-rs 外部断言。
