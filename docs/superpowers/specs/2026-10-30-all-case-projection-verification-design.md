# 全案例真实投影验证设计

## 目的

当前真实引擎门禁已证明 144/144 个 rssbundle 案例满足 manifest、文件 hash、projection profile 与门面合同，
但只对每个被引用 profile 选择一个代表案例执行真实 `project_case`。本改动要求每个已发布案例都通过当前
tensoreval 的真实投影，确保其模型表和普通本地资源确实能够导入，而不只是静态合同有效。

## 范围

- 对当前不可变 engine snapshot 中每个 `importable` 案例逐一调用 `ProcessEngine::project_case`。
- 案例运行树必须保留 `.rssbundle` acquisition root，使 DSS redirect 按 bundle 根解析。
- 严格校验 profile `projectorOutputs` 声明的全部有序 Snappy Parquet 输出：名称、数量、顺序、schema kind
  和 diagnostic disposition 必须一致。
- `model` 输出计入模型表；`data` 与 `struct` 输出计入普通本地资源；diagnostic 只计入诊断，不计入可发布输入。
- 每个案例保存脱敏的投影结果摘要；任一案例投影失败都使真实引擎门禁失败。
- 继续对每个被案例引用的 profile 选择一个投影成功的代表案例，注入 manifest 派生的 `resultSelector` 后执行
  `managed.rss`，验证最终研究门面。

## 非目标

- 不要求 144 个案例全部执行 `managed.rss` 或生成最终仿真结果。
- 不改变产品运行时的案例导入、确认和发布行为。
- 不恢复固定 20 张模型表假设；输出分类继续完全由当前 projection catalog 决定。
- 不把服务器路径、脚本正文或原始 stderr 写入发布报告。

## 方案选择

采用逐案例真实投影。继续按 profile 抽样无法证明具体 bundle 的 DSS redirect 和资源文件都可读取；逐案例完整仿真
又超出“确保模型和本地资源能导入”的目标，并显著增加门禁耗时。逐案例投影加 profile 级代表仿真能够分别覆盖
案例输入完整性与研究函数可执行性。

## 报告与失败语义

真实引擎报告增加全案例投影统计和逐案例摘要，至少包含案例稳定 ID、projection profile、成功状态、Model/Data/Struct/
diagnostic 数量以及公开错误码。成功条件为：

1. `projectedExamples == importableExamples == bundleExamples`；
2. `projectionFailures == 0`；
3. 每个案例的可发布输出集合与 profile 声明完全一致；
4. 每个被引用 profile 仍有一个成功的代表 `managed.rss` 运行。

失败信息只保留稳定错误码和经现有 adapter 清洗的公开摘要；报告不得包含物理路径、secret 或未经处理的 stderr。

## 自动化验证

- 先修改发布测试，使其要求每个 importable 案例都有真实投影摘要；旧实现应因只投影代表案例而失败。
- 真实 eig-rs 门禁逐一投影 144 个案例，并验证全部成功。
- release semantic assertions 和机器功能目录验证新增统计，防止门禁退回 profile 抽样。
- 回归运行后端全量测试、前端相关发布断言、真实引擎门禁、功能覆盖门禁与格式检查。
