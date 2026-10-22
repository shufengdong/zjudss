# 开发状态与设计速览

> 新开的 agent 先读本文。详细机制见 `architecture.md`、`api-design.md`、`db-design.md`、
> `frontend-design.md`、`model-workspace-design.md`、`agent-prompt-skills-design.md` 和
> `v2-deployment.md`。自动化全链路验收见 `integration-testing-design.md`。

## 当前结论

配电网仿真智能体 v2 已完成实现，并通过现有组件、服务集成和真实 tensoreval smoke test：

| 模块 | 状态 | 说明 |
|---|---|---|
| 身份与隔离 | ✅ | instance/trusted_proxy、scope repository、跨 owner 统一 404 |
| Agent 调度 | ✅ | 实例 8、owner 2、session 1、等待 32；agent 按消息短生命周期创建 |
| 消息与恢复 | ✅ | 八类消息、持久事件、Last-Event-ID、resync、最近页/older/around |
| Artifact/配额 | ✅ | SHA-256 CAS、原子配额、20,000/20 GiB、80% 告警、GC |
| 模型工作区 | ✅ | active/draft、不可变版本、替换/移除/确认/激活/回退 |
| 多格式摄取 | ✅ | 单 RSS、ZIP/目录 rssbundle、CSV/Excel/Parquet、分批追加、表头推断 |
| 引擎资源 | ✅ | TENSOREVAL_CWD 文档/catalog/案例 hash snapshot、内置案例导入 |
| Agent prompt/skills | ✅ | v2 bundle、按需知识、skill-gated 工具、run 固化技能引用 |
| 仿真 | ✅ | 任务私有输入树、适配器边界、Parquet input/result-dir/output-format |
| 结果分析 | ✅ | RSS 脚本、受控 alias、派生 Parquet、血缘、六类视图 |
| 任务比较 | ✅ | 结果名/稳定键/dtype/单位兼容性和差值 DataFrame |
| 历史检索 | ✅ | FTS5 trigram、短查询有界 LIKE、任务/时间过滤、agent 历史工具 |
| 前端工作台 | ✅ | scope 恢复、历史搜索、目录上传、案例、工作区、任务、表格、ECharts |
| v2 schema | ✅ | 单一 initial migration，无原型结果表或兼容分支 |

## 不可违反的边界

1. 所有 tensoreval/仿真内核调用只能通过 `backend/src/adapters/`。
2. LLM 和前端只传稳定资源 ID 与受控 spec，不能传服务器路径、命令、SQL 或 ECharts option。
3. 模型版本、artifact、结果数据集和分析脚本发布后不可修改；变化产生新版本/资源。
4. 所有资源读取先校验 access scope；跨 scope 与不存在资源都返回 404。
5. 公开部署必须使用 trusted proxy。instance 只用于可信单用户或单租户。
6. 仿真和分析结果统一为 Snappy Parquet；数据库只存 schema、行数、血缘和 artifact 引用。
7. 领域写操作以自然语言为主；固定控件只用于上传、历史、案例浏览、结果读取和下载。

## 运行时链路

`Artifact → Ingestion → ModelVersion → SimulationTask → ResultDataset → ResultAnalysisRun/Comparison`

上传可以分多批进行。DataFrame 类型根据列名推断，文件名和 sheet 名只作展示；模糊、未知、dtype 冲突
或同类型重复时保持 collecting，等待用户在对话中确认。错传文件重新上传使用新 batch/file ID，再由
自然语言说明替换或移除，不覆盖历史版本。

任务绑定 model version、study type/spec、实际 RSS、agent bundle/skill refs 和 engine contract。
`tensoreval` 由适配器显式传 `--result-dir ... --output-format parquet`。用户查询结果时，agent
读取 dataset schema；需要计算则生成 RSS，读取既有 Parquet 并产出派生 DataFrame。

## 前后端契约

`frontend/src/types/index.ts` 汇总 session/message/workspace/task/dataset 类型。ID 出网为字符串，
时间戳为 ISO 8601。后端 REST/SSE 直接输出 camelCase。

前端按 session 键控消息，先接 SSE 再读取快照。localStorage 键
`zjudss:last-session:<scopeKey>` 只保存导航偏好。DataFrame 大表数据保留在结果 hook 中，不写入聊天
全局 state。已有视图由本地六类 builder 编译，禁止执行 formatter。

## Schema 与数据

v2 仅支持空库初始化。部署前删除旧开发 SQLite 和 DATA_DIR；不做回填或双读。唯一 migration 为
`backend/migrations/00000000000000_initial`。成功结果只进入 `result_datasets`，没有
`task_files/task_results/curve_data/params_json`。

## 验证状态

- 后端全量测试覆盖身份、调度、SSE、配额、四类摄取来源、案例、版本工具、Parquet 合同、仿真失败
  原子性、结果分析、六类视图、比较、历史和完整 DataFrame e2e。
- 前端测试覆盖消息归并、session state、workspace null、SSE、分页表格、null/重复行和六类图表。
- 已用当前 eig-rs 源码重建 release binary，并通过真实 tensoreval 命名 Parquet 输入、expr_df 执行和
  Snappy Parquet 输出 smoke test。该测试因 CI 不一定安装外部 binary，仍保留为显式 ignored 并在发布时单独运行。
- 当前尚未覆盖浏览器 → Vite → Axum → SQLite/CAS 的自动化全链路，也没有真实 DeepSeek 发布验收。
  已在 `integration-testing-design.md` 定义机器可读功能目录、Playwright、可编排 LLM/engine、故障注入和
  四级门禁；完成落地并达到目录 100% 前，不宣称全链路功能验收完成。
- 自动化测试设计审查发现工具合同有一项实现缺口：manifest 中的 `get_storage_usage` 尚未注册，已注册的
  `list_result_datasets` 尚未进入 manifest。集成测试实施时必须先统一两份清单并加入双向枚举测试。

发布命令和配置见 `docs/v2-deployment.md`。
