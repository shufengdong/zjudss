# 自然语言发布与 144 案例手动全链路长测设计

## 意图

在案例候选已由用户确认进入模型草稿和数据资源后，允许用户用明确的自然语言请求发布当前草稿并自动激活版本；随后使用
`docs/v4-tester-guide.md` 中按 `projection.target_function` 分组的提示词准备研究，在用户保留最终确认权的前提下生成脚本、
执行并验证结果。为当前 snapshot 的全部 144 个可导入案例提供可显式选择执行的真实依赖长测，但不把它加入日常自动回归、
PR 门禁或候选发布门禁。

## 目标

- 新增受控 Agent 工具 `publish_model_version`，只发布当前会话中与参数精确匹配的非空草稿，并沿用现有发布后自动激活、
  内容去重、描述校验和用户隔离语义。
- 用户必须明确表达“发布/生成当前版本”；研究意图本身、案例导入完成或普通模型修改均不得触发隐式发布。
- 案例导入确认与“确认并生成脚本”继续由界面确认卡完成；自然语言不能绕过这两个门禁。
- 手动长测逐个发现当前 snapshot 的可导入案例，不把 144、案例路径或案例数量写死为产品合同；每个案例使用 manifest 的
  `projection.target_function` 选择测试手册中的对应提示词。
- 每个案例完成应用、候选确认、自然语言发布、手册提示词研究配置、显式最终确认、脚本生成/修正、运行终态和至少一个结果
  数据集验证。
- 长测保存可恢复、脱敏的逐案例报告；提示词失败时保留失败证据，允许用户修改提示词映射后只重跑失败案例，并把最终成功
  使用的提示词同步回测试手册。

## 非目标与反目标

- 不允许 Agent 应用案例、确认上传候选、确认研究配置、激活任意历史版本、删除版本、创建任意脚本或直接创建运行。
- 不恢复路径型 DSS 输入、CSV 结果链路、固定模型表范围或任意 RSS/命令/SQL 参数。
- 不把长测登记为 PR/release 功能覆盖证据，也不让普通 `npm run test:e2e`、`scripts/test-release-integration` 或
  `scripts/check-functional-coverage` 自动执行它。
- 不用“跳过失败案例”得到绿色总结果；过滤执行只用于调试或续跑，完整验收仍要求本次发现的全部可导入案例通过。
- 不自动反复改写生产提示词。提示词调整是显式、可审阅的测试资料变更，并在报告中记录原提示词与最终提示词的 hash。

## 已选方案与取舍

### 方案 A：独立手动 Playwright 长测套件（采用）

新增独立配置、测试文件和根目录脚本。它复用真实 UI、API 观察辅助函数和真实依赖启动方式，但使用独立输出目录、超时和报告。
命令必须由用户显式运行，可通过案例 ID/门面过滤器及失败报告续跑。优点是最接近测试人员实际操作，确认卡、Agent 工具轨迹、
脚本工作流和结果展示都被覆盖；代价是执行时间和 LLM/引擎成本较高。

### 方案 B：给现有候选发布门禁增加环境变量开关（未采用）

复用较多，但容易因 CI 环境变量或脚本调用方式变化而意外执行 144 次完整研究，也会把“发布门禁代表案例覆盖”和“人工选择的
全案例长测”混为一谈。

### 方案 C：后端/API 专用批处理器（未采用）

速度更快，但绕过真实对话、确认卡和浏览器行为，不能证明用户看到的自然语言链路可用，也容易形成仅为测试开放的高权限接口。

## 自然语言发布合同

### 工具参数与执行

`publish_model_version` 归入 `model-intake` skill，参数为：

- `expectedDraft: { id, revision }`：必须来自同一轮 `inspect_workspace`；
- `description?: string | null`：用户给出时透传现有 200 Unicode 字符规范化规则，否则由现有服务生成描述。

工具不接受 session ID、版本 ID、路径或客户端幂等键。服务端使用当前 `request_message_id` 与精确 draft ref 生成稳定的内部
`clientPublishId`，因此同一消息重放幂等，不允许 LLM 选择或碰撞幂等域。工具调用现有 `VersionService::publish`，返回稳定
`versionId`、描述、内容 hash、`deduplicated`、`replayed` 和 active 状态，不返回物理 artifact 信息。

调用前必须加载 `model-intake`、调用 `inspect_workspace` 并确认存在非空 draft。工具再次由 repository 校验用户、session、
draft ID/revision。并发改变草稿时返回现有 `draft_revision_conflict`，Agent 必须重新检查工作区，不能猜测或自动改发新 revision。

### Agent 策略

system prompt 与 `model-intake` 明确区分：

- “修改、导入、准备研究”等请求不构成发布授权；
- 只有用户当前消息明确要求发布/生成当前模型版本时才可调用；
- 发布成功后可在同一用户消息还包含研究意图时继续进入 `study-planning`；
- 案例导入确认和研究最终确认仍提示用户操作对应卡片。

Agent bundle 内容和工具清单变化，bundle 版本从 `v4.0.0` 提升为 `v4.1.0`，同步默认配置、部署文档、运行时断言、报告断言
及 bundle hash。

## 手动长测流程

### 发现与隔离

套件默认像 release integration 一样启动隔离的临时 SQLite 与 `DATA_DIR`，结束后只保留脱敏报告；不会向已有部署写入 144 个
session，也不负责删除已有环境的数据。若后续增加部署环境黑盒模式，必须由用户显式提供 base URL，并明确说明不自动清理。

套件启动后登录固定测试用户，通过 `/api/examples` 读取当前 snapshot，并只选 `importable` 案例；摘要不含目标门面时再按稳定
example ID 调用详情接口读取 manifest。每个案例建立独立 session，
避免模型、Data/Struct alias、工作流和结果互相污染。报告记录 snapshot ID、example ID、相对目录、projection profile、目标门面
和阶段耗时，不记录账号、Cookie、服务器路径、完整脚本、原始 stderr 或 LLM secret。

### 单案例状态机

1. 在案例库选择稳定 example ID 并点击“使用此案例”。
2. 在模型工作区等待候选就绪，保留 catalog 建议用途，显式确认可发布候选；diagnostic 不发布。
3. 发送“请发布当前模型草稿并自动激活，版本描述为……”，断言轨迹含
   `load_agent_skill -> inspect_workspace -> publish_model_version`，并等待 active 存在且 draft 清空。
4. 根据目标门面读取测试手册提示词映射并原样发送；断言 Agent 使用 `study-planning`、目录检索与 `prepare_study`，没有换成
   其他门面。
5. 对配置卡按 catalog/案例导入资源完成唯一绑定、Struct 默认值确认和推荐结果选择；若出现真实歧义则把案例标记为需要人工
   决策并失败，不由测试猜选。
6. 点击“确认并生成脚本”，等待 workflow 终态。允许产品既有的最多 3 次脚本修正；成功时校验脚本为 agent 来源、运行成功、
   结果名称/数量符合冻结合同并至少一个结果可以读取 schema/首屏数据。
7. 写入该案例的原子 checkpoint 后再进入下一案例。进程中断时，可用报告中的未通过 example ID 显式续跑。

### 提示词来源与修订

测试代码不复制一份容易漂移的 144 路径清单。仓库新增一个机器可读的、以目标门面为键的提示词资料文件，内容初始逐字来自
`v4-tester-guide.md` 的提示词 2、6、14、16、17、18；测试手册引用同一资料或由一致性测试验证两者文本完全相同。

当前手册的提示词 16、17 把 `projection_diagnostic` 列为资源，但现有 projection 合同将 diagnostic 排除在发布和运行输入之外。
实现和真实长测必须以本次 snapshot 的 catalog 为准；若该冲突导致配置失败，应从提示词中删除 diagnostic 输入要求并同步手册，
不得改变 diagnostic disposition 或把它伪装成 Data resource。

提示词导致未选择目标门面、缺失案例实际提供的资源、无法达到 `ready_to_confirm` 或脚本工作流失败时，该案例失败并保存脱敏阶段
和稳定错误码。用户可修改资料文件中的对应提示词，再用门面/example 过滤器重跑失败项；提示词一旦验证成功，同步更新测试手册，
一致性测试阻止二者再次漂移。提示词调整必须由人审阅，不让测试或 LLM 自动改写后静默重试。引擎、投影、资源合同或脚本错误
不能通过粉饰提示词判为成功。

## 手动入口与自动化隔离

- 新增类似 `scripts/test-all-case-agent-journeys` 的显式入口，校验真实 LLM、`TENSOREVAL_BIN`、`SPARROWZZ_DIR` 后启动专用
  Playwright config。
- 入口支持 `CASE_JOURNEY_EXAMPLE_ID`、`CASE_JOURNEY_FACADE` 和 `CASE_JOURNEY_RESUME_REPORT` 等窄范围选择；默认仍是当前
  snapshot 的全部可导入案例。
- 专用测试文件放在与 `integration-tests/e2e`、`integration-tests/release` 分离的目录，普通 Playwright config 与 release config
  均不匹配它；`frontend/package.json` 不把它串入既有 `test:e2e`。
- 报告落在 `target/manual-case-journeys/`，失败返回非零；每个案例增量原子写入，最终汇总 discovered/passed/failed、按门面统计、
  prompt hash、bundle/hash、engine snapshot 和耗时。
- 本次实现完成只声明“手动入口已就绪”；聚焦测试和单个代表案例可用于验证 harness。完整 144 条不会在实现过程中自动启动，
  只有用户显式运行长测入口后，才能根据报告声明全案例链路通过。

## 验证与文档

- Rust 工具测试：skill gate、明确 draft/revision 发布、幂等重放、deduplicate、stale draft、跨用户/跨 session 404 等价语义，
  工具输出无路径。
- scripted-agent 行为测试：明确发布请求会调用新工具；普通修改/研究请求不会隐式发布；发布后组合研究请求继续准备配置卡。
- manifest/registry/bundle hash 和公共合同枚举测试更新为 23 个工具；功能目录新增该工具的 PR 行为证据，但不登记手动长测为
  自动覆盖替代品。
- 手动套件的纯解析、提示词映射、一致性、过滤/续跑和报告脱敏逻辑用快速 Vitest 覆盖；真实 144 案例执行不进入回归。
- 同步 `development-status.md`、`api-design.md`（工具语义而非新路由）、`agent-prompt-skills-design.md`、
  `integration-testing-design.md`、`v4-deployment.md`、`frontend-design.md` 和 `v4-tester-guide.md`。

## 实施者仍可决定的细节

- 机器可读提示词资料采用 JSON 或 TypeScript 常量，以能被手册一致性测试稳定解析且不引入生成文档步骤为准。
- 报告 checkpoint 的临时文件名和原子 rename 方式。
- UI 定位器与已有 release helper 的提取边界，只要专用套件不改变生产 UI 或形成测试专用后门。
