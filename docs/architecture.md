# 配电网仿真智能体 V4 架构

> V4 使用与模型版本分离的数据资源库和受管研究工作流，并以空数据库作为新基线；需求见
> [`2026-10-26-independent-data-resources-design.md`](superpowers/specs/2026-10-26-independent-data-resources-design.md)。

## Scope

V4 是空库直接切换版本。系统由 React/TypeScript 工作台、Rust/Axum 应用、SQLite 元数据与
内容寻址 artifact 存储、可选择 DeepSeek 或百炼的 LLM agent，以及只能经 `backend/src/adapters/` 调用的 tensoreval 组成。
不存在旧模型/任务协议的迁移、双读或兼容路由。

## Runtime flow

```text
固定用户登录 → 会话 → 统一上传批次 → 用途确认 ┬→ 模型表 → 可变草稿 → 生成并激活版本
                                                └→ 普通 Data/Struct 资源
自然语言研究需求 → 配置卡确认 → 受管脚本生成/执行/修正 → Snappy Parquet 结果
已成功/手写/上传 RSS 脚本 → 用户点击运行 → 使用最新所需资源 → 新结果
```

- `.dss`、DSS 目录、`.dssbundle`、`.rssbundle`、ZIP、CSV、Excel 和 Parquet 进入统一上传批次；确认前不发布。
- 每个候选显式选择模型表、普通数据资源或跳过。模型候选进入草稿，Data/Struct 候选成为可替换的独立资源；静态导入的 `para` 属于普通资源。
- `.rss` 是会话脚本资源，不属于模型版本。创建脚本不会运行；只有脚本卡片“运行”按钮能创建运行。
- 内置案例只来自 `SPARROWZZ_DIR/dss/rssbundle`。普通目录形成分类，直接包含
  `manifest.json` 的 `.rssbundle` 目录形成案例；`SPARROWZZ_DIR/dss` 其他位置的 `.rss` 不进入案例库。
- 受管工作流冻结确认时的模型、资源、门面、结果合同和 engine snapshot；脚本再次“运行”时按其输入合同解析最新资源。
- 分析、视图和比较保存输入快照与独立 artifact；来源结果删除后，派生结果通过 tombstone 血缘继续解释。

## Boundaries

1. 所有引擎进程调用都在 adapter 层，参数使用结构化 spec；handler、tool 和 application service 不拼命令。
2. 浏览器和 LLM 只提交稳定 ID、草稿 revision 与受控 spec，不提交服务器路径、SQL、命令或任意图表配置。
3. repository 查询显式接收 `UserId`；跨用户和不存在资源统一 404。
4. 发布后的模型版本、脚本 artifact、运行快照、结果和分析记录不可修改。
5. 生产启动必须通过当前 binary 的功能性 capability 探测、动态 engine resource snapshot、Agent bundle 和用户
   种子校验；服务不修改 eig-rs 工作树。
6. 结果事实来源仅为 Snappy Parquet；数据库保存索引、schema、行数、hash、状态和血缘。

## Consistency and recovery

统一上传在最终确认前不改草稿或资源库；确认使用相互独立的模型和资源子事务，允许部分成功后只重试失败侧。
草稿写入比较 `{id, revision}`，发布、确认和运行使用 client id 幂等。服务重启恢复 upload batch 和受管工作流，
无法继续的执行收敛为带稳定诊断的终态，已提交消息、版本、资源和结果保持不变。SSE 先持久化消息再广播。

引擎资源在启动时从当前 `SPARROWZZ_DIR` 读取并按字节生成 snapshot ID；发布阶段再次校验源 hash，并原子复制
到 `DATA_DIR/system/engine-resources/<snapshotId>`。案例导入只读取这个不可变发布树。源文件在扫描与发布之间变化、
已发布目录损坏或 catalog 不兼容都会失败关闭。更新文档、catalog 或案例资源后必须重启以发布新 snapshot；
单独替换 `TENSOREVAL_BIN` 无需重启，下一次 adapter 调用直接执行路径上的当前普通文件。

案例扫描显式解析 manifest schema v2 的 schema/artifact、入口、study、source、projection、files 和 inputs，并把
projection 逐字段绑定到同一 snapshot 的 `case_projections.json` schema v2。单个无效 manifest 仍形成
带脱敏诊断的不可导入叶节点，不阻断其他案例。应用案例时先验证不可变快照中 `manifest.json` 和
`manifest.files` 全部哈希，再由 adapter 在私有树内执行已发布 projector；有序输出按目录角色成为 Model/Data/Struct
候选，diagnostic 只进入批次诊断，然后进入统一上传用途确认。bundle 内未声明文件不会被读取或暂存。HTTP、前端与
Agent 工具共享同一规范目录和 manifest 解释结果，并且都不返回物理路径。

## Backend code organization

`api` 只处理 HTTP/SSE、认证提取与 DTO；`application` 编排用例；`db/repositories` 封装带用户作用域的
持久化；`adapters` 是 tensoreval 唯一边界。跨用例重复出现的数据形状在所属层集中构造：Parquet artifact
元数据属于 application artifact 服务，结果事件引用属于事件模型，运行/分析状态迁移属于 repository 内部。
服务依赖通过具名结构在 `AppState` 组合根装配，避免构造参数顺序承载隐含语义。

错误类型按边界保留：adapter 错误携带稳定 code、category、用户可读 message 和可选诊断；repository 错误
表达存储与并发语义；application service 再转换为业务错误；API 最后映射为公开状态码和错误体。可选的大型诊断
在 adapter 错误内使用间接存储，使常见 `Result<T, AdapterError>` 保持紧凑，同时保持 JSON 合同不变。

repository 的异步请求路径把 Diesel 同步 SQLite 操作封装在 `spawn_blocking` 内；启动迁移、恢复与用户 seed
在开始监听 HTTP 前同步完成。历史搜索的 LIKE/FTS 查询和索引重建语句集中定义，运行与分析仓储共用状态迁移
值对象，但各自保留独立事务与表级 SQL。

## Capacity and trust

Agent 调度默认实例 8、每用户 2、每会话 1、等待 32；引擎进程默认 4。artifact 配额按固定用户的可信
storage scope 计费。当前固定用户密码与 ID 相同是已明确接受的试用约束，不代表安全身份方案。
