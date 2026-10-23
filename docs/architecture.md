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
- 运行冻结模型内容 hash、脚本 artifact/hash、engine contract 和 Agent 来源信息。
- 分析、视图和比较保存输入快照与独立 artifact；来源结果删除后，派生结果通过 tombstone 血缘继续解释。

## Boundaries

1. 所有引擎进程调用都在 adapter 层，参数使用结构化 spec；handler、tool 和 application service 不拼命令。
2. 浏览器和 LLM 只提交稳定 ID、草稿 revision 与受控 spec，不提交服务器路径、SQL、命令或任意图表配置。
3. repository 查询显式接收 `UserId`；跨用户和不存在资源统一 404。
4. 发布后的模型版本、脚本 artifact、运行快照、结果和分析记录不可修改。
5. 生产启动必须通过 binary capability、engine resource snapshot、Agent bundle 和用户种子校验。
6. 结果事实来源仅为 Snappy Parquet；数据库保存索引、schema、行数、hash、状态和血缘。

## Consistency and recovery

模型导入在最终确认事务前不改草稿。草稿写入比较 `{id, revision}`；发布和运行使用 client id 幂等。
服务重启把处理中导入和 queued/running 运行收敛为带 `service_restarted` 诊断的失败态，已提交消息、版本和
结果保持不变。SSE 先持久化消息再广播，支持 `Last-Event-ID` replay 与 resync。

## Capacity and trust

Agent 调度默认实例 8、每用户 2、每会话 1、等待 32；引擎进程默认 4。artifact 配额按固定用户的可信
storage scope 计费。当前固定用户密码与 ID 相同是已明确接受的第三版试用约束，不代表安全身份方案。
