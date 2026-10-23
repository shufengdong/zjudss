# Agent Prompt 与领域技能设计

> 状态：v2 已实现资源合同、启动 preflight、文档/案例快照、领域技能 bundle、工具门控、表头推断，
> 并在仿真和分析 run 中固化技能引用。
> 适用范围：zjudss 后端通过 rig 0.45 构建的配电网仿真 agent。
> 引擎知识源：`${TENSOREVAL_CWD}/sparrowzz/dss/docs/`，当前核对版本固定到 eig-rs superproject commit
> `3698864f0856e16ee77c56be89ef3d1c1424007b` 及其 sparrowzz submodule commit
> `d2b6d88a740293074273fd5df4237898ac66657a`。

## 1. 实现与术语

`backend/resources/agent/v2` 保存经过 hash 校验的 manifest、system prompt、skill index 和领域
Markdown。每轮 agent 注入紧凑 system prompt、技能索引和当前 engine resource snapshot 摘要；领域
文档、案例详情和技能正文通过稳定 ID 按需读取。工具名称、说明与参数 schema 由 Rust `Tool` 实现
提供，脚本生成类工具在缺少 required skill 时返回 `skill_required`。

rig 的 `AgentBuilder` 提供 `preamble/context/dynamic_context/tool` 等组装能力，但 rig 0.45 中名为
`Skill` 的类型属于 Anthropic container 扩展。当前项目使用 DeepSeek，不能把该 provider 专属能力当成
项目的技能系统。本文中的 **领域技能（agent skill）** 是 zjudss 自己管理、按需注入模型上下文的版本化
Markdown 知识模块。

## 2. 四类内容的边界

| 层 | 载体 | 放什么 | 不放什么 |
|---|---|---|---|
| 系统规则 | `agent/system.md` | 角色、自然语言交互原则、资源 ID 规则、工具调用顺序、安全边界、遇到歧义时的行为 | 具体函数全集、完整 schema、用户数据 |
| 领域技能 | `agent/skills/*.md` | 完成某类任务所需的步骤、选择条件、最小示例、禁止推断和验收清单 | 密钥、服务器路径、任意用户文件内容 |
| 机器合同 | `engine/<commit>/catalog/*.json` | 函数签名、参数顺序、DataFrame 列、dtype、单位、稳定键和结果名 | 面向模型的长篇说明 |
| 工具合同 | Rust `Tool` 实现 | 可执行动作、参数 schema、稳定 ID、错误码和输出上限 | 引擎命令行、存储路径、绕过 application/adapters 的能力 |

系统 prompt 只负责稳定策略。会频繁变化或只与某一任务有关的引擎知识进入领域技能；必须由程序精确
判断的名称、类型和单位进入 JSON catalog；真正产生副作用的能力只能由工具提供。Prompt 不能代替参数
校验、权限检查或脚本验证器。

## 3. 知识来源与优先级

领域技能从 eig-rs 文档提炼，而不是凭模型经验编写。来源映射如下：

| eig-rs 来源 | zjudss 用途 |
|---|---|
| `agent_quickstart.md` | 分析类型、求解器和结果选择的主流程 |
| `catalog/functions.json` | RSS/DataFrame/DSA 函数的规范名称、参数个数与顺序 |
| `catalog/dataframes.json` | 输入与结果表的列、dtype、单位、稳定业务键 |
| `rss_language_reference.md`、`rsdss_function_reference.md` | 脚本语法、表达式、筛选、聚合、连接和多返回值规则 |
| `dsa_function_reference.md` | 各分析入口的输入、输出和功能要求 |
| `simulation_recipes.md` | 已验证脚本结构和任务配方 |
| `validation_and_troubleshooting.md` | 结果验收、诊断和常见失败处理 |
| `dss_to_rss.md`、`opendss_compatibility_matrix.md` | DSS/RSS 导入边界和不支持项 |

发生冲突时采用 eig-rs 自身声明的可信顺序：发布版实现 → 解析器/分析模块 → 随版本测试 → catalog →
Markdown。zjudss 不硬编码 `../eig-rs` 相对路径；统一从配置后的
`${TENSOREVAL_CWD}/sparrowzz/dss` 建立只读快照，并记录来源 commit、相对路径与 SHA-256。

当前固定 catalog 为 schema version 2；外层 eig-rs commit 通过 Git tree 指向上述 sparrowzz commit。
资源固化时 `dss_to_rss.md` 和 `opendss_compatibility_matrix.md` 含未提交修改，因此 manifest 显式记录
`sourceDirty: true` 并锁定整个 docs tree hash；运行目录只要再发生变化，启动 preflight 就会失败：

- `dataframes.json` SHA-256：`ead67bd4354e0acb99a957c4f96fd80f85e96268a7a190fb36ac65cce98cd471`
- `functions.json` SHA-256：`d84b715c62b79ba95931b04e2df1c351928c02c8124b73f18b709f2430331496`

案例 allowlist 从 sparrowzz commit 的 Git 对象生成，固定为 16 个根 RSS 和 57 个 bundle。工作树中
额外的未提交 bundle 不进入快照。开发工作区里的 eig-rs 可能含未提交修改，因此 manifest 同时记录
HEAD、submodule commit、dirty 状态和逐类内容 hash，不能默认为当前工作树就是发布事实。

`sourceDirty: true` 表示生成当前合同的受控来源快照包含两份已知文档修改，并且这些内容已由完整 tree
hash 固定；它不表示可以接受任意 dirty 工作树。该字段用于来源审计，启动 preflight 实际校验合同范围
内的 catalog、文档和案例内容 hash，而不读取 Git 工作树状态；相关内容发生变化就按资源漂移处理。
若这些修改需要成为新的基线，应按第 11 节更新合同、bundle 和测试。

## 4. v2 资源布局

```text
backend/resources/agent/v2/
├── manifest.json
├── system.md
├── skill-index.json
└── skills/
    ├── model-intake.md
    ├── study-planning.md
    ├── rss-generation.md
    ├── result-analysis.md
    ├── comparison.md
    └── troubleshooting.md

${DATA_DIR}/system/engine-resources/<snapshot-id>/
├── manifest.json
├── docs-index.json
├── examples.json
├── docs/                  # 经过允许列表和 hash 校验的 Markdown 快照
└── catalog/
    ├── dataframes.json
    └── functions.json
```

Agent 资源用 `include_str!`/`include_bytes!` 编译进后端。引擎资源在启动 preflight 时从
`TENSOREVAL_CWD` 读取并原子写入只读快照，运行期间只使用该快照的 docs/catalog 索引；案例文件在导入
时从已校验的源目录复制到用户 artifact 存储。manifest 至少保存 `agentBundleVersion`、eig-rs
superproject commit、sparrowzz submodule commit、catalog schema/version/hash、docs tree hash、examples
catalog hash、每个 prompt/skill 文件 hash、允许的工具名称和生成时间。版本是内容标识，不使用 `latest`。

## 5. 启动前知识加载

“让 agent 提前学习”在本系统中定义为 **服务启动时预检、切分和索引，执行某一请求时按需注入**，不在
启动时微调模型，也不把全部文档永久塞进每轮上下文。当前目录包含 11 份 Markdown、2 份 JSON catalog、
16 个根目录 RSS 示例和 57 个已发布 `.rssbundle`；完整文本约数百 KiB，全部常驻 prompt 会挤占用户模型
和结果 schema 的上下文空间。

启动时 `EngineResourceRegistry` 执行：

1. canonicalize `TENSOREVAL_CWD`；文档和案例入口只从 `sparrowzz/dss` 发现，根 RSS 的显式依赖可以
   解析到该工作目录内其他允许文件，拒绝任何逃逸 `TENSOREVAL_CWD` 的路径或符号链接；
2. 校验 `docs/README.md`、允许列表内 Markdown、两个 catalog 及其版本/hash；
3. 按“文档 → 标题 → 二级/三级标题”切成稳定 section，生成 `docId/sectionId/hash/title/keywords`；
4. 建立本地只读全文索引，提供 `search_engine_docs` 和 `read_engine_doc_section`；
5. 扫描根目录 `.rss`、`models/` 依赖和 `rssbundle/**/*.rssbundle/manifest.json`，生成案例目录；
6. 完成后原子发布 `snapshot-id`；任一关键合同失败则 readiness 失败，禁止脚本生成和案例导入。

`system.md` 与技能索引始终进入 agent 上下文；`agent_quickstart` 的核心选择表提炼进
`study-planning`。其余原始文档由 agent 在生成脚本或处理陌生诊断前通过只读工具查询。工具返回命中的
标题、短片段、doc/section ID 与 hash，并限制总字符数；agent 再读取必要 section，不能请求任意文件路径。

## 6. 内置案例库与导入

案例库收录两类来源：

- `sparrowzz/dss/*.rss`：当前 16 个经过回归验证的脚本；导入时必须解析 `import_dss/import_bmopf`、
  `ds_*_plan` 和 `#include` 的完整依赖闭包，不能只复制入口脚本。依赖可能位于 `models/`，也可能是
  `psa/tests/bmopf` 或 `sparrowzz/rspower/lib`；只要显式引用解析后仍位于 `TENSOREVAL_CWD` 内即可；
- `sparrowzz/dss/rssbundle/**/*.rssbundle`：当前 57 个有 `manifest.json`、文件 hash 和自执行验证证据的
  自包含 bundle，包含 execution plan、静态潮流、谐波和短路案例。

`examples.json` 的每项至少包含稳定 `exampleId`、标题、说明、标签、`sourceKind`、入口相对路径、分析
类型、solver、required features、文件数/字节数、tree hash、验证状态、来源与许可证说明、
`engineContractVersion`。只有 manifest/hash/依赖闭包验证通过的案例标记为 `importable`；需要
MUMPS/MKL 或运行时间较长的案例必须在导入前展示能力与资源提示。

`exampleId` 是不透明的 `ex_` 前缀标识，由 `sourceKind + 规范相对路径` 的哈希稳定生成并做全目录碰撞
检查；内容变化不改变 ID，而由 `engineResourceSnapshot + treeHash` 阻止把旧选择静默导入为新内容。

Agent 通过 `search_examples`、`inspect_example` 和 `import_example` 使用案例库。导入参数包含稳定
example ID、用户选择时看到的 engine resource snapshot，以及当前 draft/active 基线版本；空工作区
允许基线为 null。用户可以说“导入 IEEE13 短路案例”“找一个三时点 QSTS 例子”；唯一高置信命中可
直接导入，有多个合理候选时先列出差异并
让用户选择。用户或模型不能提交 `TENSOREVAL_CWD` 下的路径，只能提交稳定 `exampleId`。

导入不修改共享案例。后端重新校验源文件 hash，把入口和显式依赖按相对 `TENSOREVAL_CWD` 的虚拟路径
逐个提交到当前 owner 的 artifact 存储，走普通 ingestion/workspace 事务并创建不可变 ModelVersion。
运行前把版本文件物化到任务私有 input tree，以该 tree 根作为子进程 cwd；执行不再读取共享案例目录。
空工作区首个 ready 版本可自动 active，已有 active 时只成为 draft。版本 manifest 保存 example ID、
source tree hash、engine snapshot 和来源信息；此后修改参数产生子版本，已有副本不会因引擎目录升级而
变化。导入文件按普通历史文件计入该 owner 配额，底层内容寻址存储仍可去重。

前端可提供常用的只读“案例库”入口和筛选。点击“使用此案例”必须向对话写入一条可见用户消息，再由
agent 调用 `import_example`，保证自然语言历史、工具调用、版本创建和后续修改处在同一条审计链上。

## 7. 系统 prompt 的确定方法

`system.md` 只保留每轮都适用且不应由用户覆盖的规则：

1. agent 的职责是解释用户意图、读取当前工作区状态并通过已注册工具完成操作；
2. 文件、版本、任务、数据集和视图只使用稳定 ID，禁止向模型暴露或接受服务器路径；
3. 上传可以由固定控件完成，替换、移除、修改、运行、查询和比较由自然语言触发；
4. 对表类型、替换关系、单位、求解器或比较键存在歧义时，先展示证据并请求用户确认；
5. 脚本生成前加载对应领域技能，并在技能无法覆盖具体函数/诊断时检索当前引擎文档；函数、列、单位和
   稳定键必须以当前 catalog 为准；
6. 只调用已注册工具，不编造工具结果，不把示例值当成真实仿真结果；
7. 工具失败时依据稳定错误码解释下一步，不向用户输出原始路径、命令、密钥或未筛选 stderr；
8. 回答中明确区分已执行结果、准备执行的动作和需要用户确认的选择。

提供商、模型名、部署地址和业务数据不写进 system prompt。工具参数的细节也不重复写入 prompt，避免
工具 schema 修改后产生两份事实来源。

## 8. 首批领域技能

每个技能文件包含固定元数据：`id`、`version`、`description`、`triggers`、`requiredTools`、
`engineContractVersion` 和 `sources`。正文统一包含“何时使用、所需事实、执行步骤、禁止推断、验收、最小
示例”，不得复制整份上游参考文档。

| skill ID | 解决的问题 | 主要来源 | 必须在何时加载 |
|---|---|---|---|
| `model-intake` | 识别 RSS/bundle/DataFrame、按表头推断、处理重传和替换关系 | schema catalog、`dss_to_rss.md` | 需要解释或修正模型输入时 |
| `study-planning` | 从自然语言选择潮流、QSTS、短路等分析入口和所需结果 | `agent_quickstart.md`、DSA reference | 创建或重跑仿真前 |
| `rss-generation` | 按规范参数顺序生成可验证 RSS | functions catalog、RSS/RSDss reference、recipes | 任何生成仿真脚本的工具前 |
| `result-analysis` | 读取 dataset schema，生成筛选、聚合、连接和可视化输入表 | 两个 catalog、RSDss reference | 生成结果查询脚本前 |
| `comparison` | 选择稳定键、时间维度、单位和差值列，不按行号比较 | schema catalog、validation docs | 比较两个任务前 |
| `troubleshooting` | 根据稳定错误码、诊断表和能力探测给出修复动作 | troubleshooting、compatibility matrix | 脚本/内核/校验失败后 |

示例必须来自目标 commit 下可运行案例或由测试固定的最小 fixture。示例中使用占位资源 ID，不写本机绝对
路径。函数列表和列列表不手抄到多个技能，技能通过 catalog 查询结果引用它们。

## 9. 技能发现、加载与执行约束

每轮 agent 固定注入 `system.md`、精简的 `skill-index.json` 和当前会话的资源摘要。完整技能按需加载：

1. 模型从技能索引判断当前任务需要哪些 skill；
2. 调用只读工具 `load_agent_skill(skillId)`，工具返回 `{id, version, hash, content}`；需要更具体的引擎
   事实时再调用 `search_engine_docs/read_engine_doc_section`；
3. 当前 agent run 的 `LoadedSkillSet` 记录已加载技能；
4. 会生成 RSS 的写工具在 application 层检查所需技能是否已加载，否则返回 `skill_required`；
5. 服务端仍用 catalog 和脚本验证器检查最终参数与脚本，不能因为模型加载过技能就跳过验证。

首版采用显式 `load_agent_skill`，不引入向量检索。技能数量少且边界明确，显式加载更容易测试、审计和
复现。只有技能数量增长到无法由索引选择时，才评估 `dynamic_context`；检索结果仍需携带资源版本和 hash。

普通聊天、列出现有任务、读取表格或切换已有视图不要求加载领域技能。需要脚本生成的操作每个 agent run
重新加载，不能依赖模型“记得”上一轮内容。

## 10. 版本、持久化与可复现性

`simulation_tasks` 和 `result_analysis_runs` 保存实际使用的 `agent_bundle_version`、
`agent_skill_refs_json`（本轮真正加载的 id/version/hash 有序列表）与 `engine_contract_version`。脚本
artifact、输入 hash、输出 dataset 和这些版本共同构成可复现记录。任务详情 API 返回这些字段，便于
诊断历史结果；前端默认折叠在“运行信息”中。

Agent bundle 与 engine contract 必须兼容：manifest 明确允许的 catalog schema 和 eig-rs commit。启动
preflight 校验所有文件 hash、工具列表和合同关系；不匹配时进程启动失败且不接流量。数据库和已有
artifact 保持不变，需恢复匹配的资源 artifact，或按第 11 节发布新合同后，才能重新启动服务读取历史。
禁止静默回退到另一版 prompt 或 catalog。

## 11. 变更流程与测试

引擎升级或修改技能时按以下顺序进行：

1. 选择明确的 eig-rs superproject commit 及其记录的 sparrowzz submodule commit，从对应 Git 对象读取
   catalog 和文档；
2. 更新 engine manifest、catalog 副本和受影响技能，生成新的 bundle version；
3. 运行 catalog 内部引用与 SHA-256 校验；
4. 对每个技能运行路由测试、必需工具测试、禁止推断测试和至少一个真实/fixture 脚本测试；
5. 运行 prompt snapshot，确认系统规则、技能索引和工具 schema 没有互相矛盾；
6. 用潮流、QSTS、短路、结果聚合和比较五类 golden 对话验证工具选择与参数；
7. 新版本通过后才能作为默认 bundle，历史任务仍显示原版本。

文档评审重点不是措辞是否自然，而是每条领域规则能否追溯到来源、是否能由 catalog/代码校验、是否只在
需要时占用上下文，以及失败时是否会阻止错误脚本执行。
