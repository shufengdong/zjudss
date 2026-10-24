# 配电网仿真智能体 V3 架构

## Scope

V3 是空库直接切换版本。系统由 React/TypeScript 工作台、Rust/Axum 应用、SQLite 元数据与
内容寻址 artifact 存储、DeepSeek agent，以及只能经 `backend/src/adapters/` 调用的 tensoreval 组成。
不存在旧模型/任务协议的迁移、双读或兼容路由。

## Runtime flow

```text
固定用户登录 → 会话 → 模型导入 → 可变草稿 → 用户生成版本并自动激活
                                           ↓
上传/Agent 创建 RSS 脚本 → 用户点击运行 → SimulationRun → Snappy Parquet 结果
                                                        ↓
                                         分析 / 视图 / 比较（独立物化）
```

- `.dss`、DSS 目录、`.dssbundle`、`.rssbundle`、ZIP、CSV、Excel 和 Parquet 进入模型导入流水线。
- adapter 把模型来源转换成规范命名的 Parquet DataFrame；只有这些 DataFrame 进入草稿和版本。
- `.rss` 是会话脚本资源，不属于模型版本。创建脚本不会运行；只有脚本卡片“运行”按钮能创建运行。
- 内置案例只来自 `TENSOREVAL_CWD/sparrowzz/dss/rssbundle`。普通目录形成分类，直接包含
  `manifest.json` 的 `.rssbundle` 目录形成案例；`sparrowzz/dss` 其他位置的 `.rss` 不进入案例库。
- 运行冻结模型内容 hash、脚本 artifact/hash、engine contract 和 Agent 来源信息。
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

模型导入在最终确认事务前不改草稿。草稿写入比较 `{id, revision}`；发布和运行使用 client id 幂等。
服务重启把处理中导入和 queued/running 运行收敛为带 `service_restarted` 诊断的失败态，已提交消息、版本和
结果保持不变。SSE 先持久化消息再广播，支持 `Last-Event-ID` replay 与 resync。

引擎资源在启动时从当前 `TENSOREVAL_CWD` 读取并按字节生成 snapshot ID；发布阶段再次校验源 hash，并原子复制
到 `DATA_DIR/system/engine-resources/<snapshotId>`。案例导入只读取这个不可变发布树。源文件在扫描与发布之间变化、
已发布目录损坏或 catalog 不兼容都会失败关闭。更新文档、catalog 或案例资源后必须重启以发布新 snapshot；
单独替换 `TENSOREVAL_BIN` 无需重启，下一次 adapter 调用直接执行路径上的当前普通文件。

案例扫描显式解析 manifest 的 schema/artifact、入口、study、source、files 和 inputs。单个无效 manifest 仍形成
带脱敏诊断的不可导入叶节点，不阻断其他案例。应用案例时先验证不可变快照中 `manifest.json` 和
`manifest.files` 全部哈希，再按该清单进入普通模型导入确认；bundle 内未声明文件不会被读取或暂存。HTTP、前端与
Agent 工具共享同一规范目录和 manifest 解释结果，并且都不返回物理路径。

## Capacity and trust

Agent 调度默认实例 8、每用户 2、每会话 1、等待 32；引擎进程默认 4。artifact 配额按固定用户的可信
storage scope 计费。当前固定用户密码与 ID 相同是已明确接受的第三版试用约束，不代表安全身份方案。
