# 运行私有结果选择覆盖设计

**日期：** 2026-11-03
**状态：** 已确认

## 背景与意图

研究门面的结果选择通过目录约束写入 Struct 字段，例如把用户选择的结果集合写入 `para.result`。当前确认流程把这次
服务端注入视为普通 Struct 修改；即使用户没有修改求解参数，也会默认执行 `create_new`，发布一个新的普通数据资源。
下一次准备同一研究时，原始 `para` 与新 `para_*` 同时成为兼容候选，于是本来唯一的参数资源变成多资源歧义。

用户想要的是：结果仍由结果选择卡控制，确认后本次运行实际使用的 `para.result` 自动变为所选结果，但资源库中不因此
增加或覆盖长期 `para`。运行记录仍必须精确保存实际输入，支持审计、重试和再次运行。

## 目标

- 只改变结果选择时，不创建、替换、改名或退出任何普通数据资源。
- 确认后的执行输入包含服务端权威的 `para.result`，并以不可变 artifact 冻结到本次 workflow/run。
- 下一次研究继续看到原始唯一 `para`，可以自动绑定；只有多个真正不同的长期资源时才要求选择。
- 结果选择器字段不再作为可编辑 Struct 参数要求用户重复确认。
- 失败重试、成功脚本再次运行和历史审计都使用与结果合同一致的有效 Struct 输入。

## 非目标

- 不允许原地修改已发布资源或历史 workflow 输入。
- 不改变用户主动修改普通 Struct 字段时的 Replace/SaveAsNew 语义。
- 不把任意 Struct 字段改成临时字段；只处理当前门面 `result_constraints` 明确声明的结果选择器。
- 不自动删除既有的 `para_*` 资源，也不改写已有运行记录。

## 反目标

- 不能为了消除界面歧义而猜选两个真正不同的参数资源。
- 不能让前端或大模型自行拼接 `result`；权威覆盖必须由后端确认和运行准备逻辑完成。
- 不能让工作流私有 artifact 出现在普通数据资源列表、alias 冲突检查或后续兼容资源候选中。

## 方案比较

### 方案一：原地更新当前 `para`

数据库行和 artifact 直接改写，表面上最接近“覆盖”。这违反已发布资源不可变、历史引用稳定和内容 hash 可审计的硬边界，
不采用。

### 方案二：自动 Replace 当前 `para`

底层发布新资源并把 alias 转移给新版本，用户只看到一个 live `para`。它仍会产生资源版本，还会失效引用旧资源的其他待确认
配置；并发配置和重复运行时副作用过大，不采用。

### 方案三：运行私有覆盖

长期 `para` 保持不变。确认时以绑定资源为基线，注入本次结果选择，物化一个不进入 `data_resources` 的 Snappy Parquet
artifact，并只由 `workflow_input_snapshots` 和 run 输入快照引用。该方案满足不可变和审计约束，同时消除新增候选资源，采用。

## 领域合同

### 服务端管理字段

后端从当前门面的 `OrderedUniqueSelection.selector` 解析结果选择器；例如 `para.result` 表示输入 alias 为 `para`、字段为
`result`。只有解析成功并指向当前 Struct 输入的字段才是服务端管理字段。

研究配置 DTO 的 Struct 卡增加 `resultSelectorFields: string[]`。前端不为这些字段显示普通编辑控件，而是在参数区显示
“结果字段由结果选择自动写入”的说明。结果集合只在结果选择卡编辑。

Struct 更新仍提交完整 values 以保持当前协议兼容，但后端忽略客户端对服务端管理字段的改动，并保留基线值。差异计算、
Replace/SaveAsNew 判断和确认摘要中的“用户参数变更”均排除这些字段。最终确认始终根据服务端已持久化的结果选择重新生成
有效值，因此伪造请求也不能覆盖结果合同。

### 确认时分离长期值与执行值

准备确认时，每个含结果选择器的 Struct 同时形成两组值：

- `resourceValues`：长期资源值。它保留绑定资源中服务端管理字段的基线，只包含用户对普通 Struct 字段的合法修改。
- `effectiveValues`：本次执行值。它以 `resourceValues` 为基线，再以有序、去重且已经通过门面约束校验的结果名覆盖选择器。

仅当用户修改普通字段、明确选择 Replace/SaveAsNew，或配置没有任何来源资源而必须首次发布时，才生成现有
`PreparedResourceReplacement`。结果注入本身永远不触发 replacement。

如果普通字段确实需要发布新资源，事务先按现有语义发布并取得最终来源资源；随后再基于该资源与本次结果合同生成执行值。
长期资源中的选择器字段不随本次结果选择变化。

### 工作流私有输入 artifact

当 `effectiveValues` 与最终来源资源的规范值不同时，确认服务生成规范单行 Struct Parquet，并作为工作流私有 artifact
冻结。它不创建 `data_resources` 行，也不占用 alias。

`workflow_input_snapshots` 使用现有列保存：

- `resource_id`：长期来源资源 ID；
- `artifact_id`、`content_hash`：本次有效输入 artifact；
- `input_alias`、`schema_ref`、`schema_hash`：实际引擎输入合同；
- `snapshot_json.source`：来源资源 ID、alias、schema hash、来源 content hash；
- `snapshot_json.effective`：有效 content hash、服务端覆盖字段和值。

如果有效值与来源资源完全相同，可直接引用来源 artifact，不重复物化。无论是否物化，locked contract 中的 Struct fields 都是
`effectiveValues`，脚本结构检查与执行看到同一份权威值。

artifact 的 CAS 提交、引用计数、确认幂等和失败释放沿用现有工作流输入机制。相同 `clientOperationId` 不得产生第二个私有
artifact 引用、资源或 workflow。

### 重试与再次运行

失败工作流的重试差异比较以快照中的 `source.contentHash` 与当前长期资源比较，不拿私有 overlay hash 与普通资源误比较。
输入未变化时可继续引用原私有 artifact；输入变化并经用户确认后，以当前长期资源和原冻结结果合同重新生成私有 overlay。

成功脚本再次运行继续采用“读取最新所需长期资源”的既有语义，但在启动引擎前必须用该脚本/run 保存的结果合同重新注入
选择器并冻结本次 run 输入。这样资源更新后仍不会丢失脚本要求的结果输出，也不会把覆盖写回资源库。

### 既有 selector-only 资源的兼容

历史上已经发布的 `source_kind=struct_edit` 资源，如果其血缘满足以下全部条件，则视为旧的 selector-only 派生候选：

- `replacesResourceId` 指向同会话、同 schema 的资源；
- `diff` 只包含当前门面声明的结果选择器字段；
- 来源资源仍为 live 且仍兼容当前输入。

研究配置的候选收集保留来源资源，并排除该 selector-only 派生资源，避免现有 `para_*` 继续形成假歧义。资源本身不删除、
不退役，仍可在数据文件页查看和用于历史审计。如果来源已不可用，该派生资源仍作为普通兼容候选，系统不得凭血缘猜测替代。

多个不满足上述条件的兼容资源仍必须由用户明确选择。

## 前端行为

- Struct 参数区隐藏 `resultSelectorFields` 的编辑器，并显示由结果选择卡管理的提示。
- 已经绑定且没有真实歧义的输入显示紧凑“已就绪”状态，不重复显示确认资源按钮。
- Struct 存在多个真实候选时，先显示资源选择；选定资源后才显示其可编辑普通字段。
- 结果选择提交后只更新配置 revision 和结果摘要，不产生“将创建新资源”提示。
- 最终摘要分别显示长期资源 alias/ID 和本次结果集合，不把运行私有 artifact 暴露成普通资源。

## 错误与并发

- 确认时继续复核 revision、snapshot、active 模型、来源资源 ID/hash/schema 和结果合同；任一变化返回现有
  `configuration_stale`。
- 私有 overlay 生成、artifact 引用或确认事务失败时，不确认配置、不启动 workflow，并释放本次新增 artifact 引用。
- 同一会话非终态 workflow 限制、取消和服务重启收敛逻辑不变。
- 不新增服务器路径、SQL、任意脚本或 ECharts option 出网面。

## 验证范围

后端行为测试至少覆盖：

- 只修改结果选择后确认，`data_resources` 数量、alias、状态均不变化；实际 Parquet 中 `para.result` 等于所选结果。
- 第二次准备同一研究时唯一长期 `para` 自动绑定，不再出现 selector-only 新候选。
- 既有 selector-only `para_*` 在来源仍 live 时被折叠；真正不同的多个资源仍要求选择。
- 用户修改普通 Struct 字段时，Replace/SaveAsNew 与失效保护保持原行为，执行 overlay 叠加在最终来源之上。
- 伪造 Struct 更新不能改变服务端管理字段。
- 重复确认幂等，stale revision/hash 被拒绝，失败不泄漏 artifact 引用。
- 失败重试与成功脚本再次运行都按冻结结果合同重新得到正确 `para.result`。

前端测试至少覆盖：

- `result` 不出现为可编辑参数，提示和结果选择卡可见。
- 已解析输入紧凑显示；真实 Struct 歧义先选择资源，其他已绑定资源不要求重新确认。
- 结果选择、刷新和 revision 更新后不出现“创建普通资源”提示。

实现同时更新 `docs/api-design.md`、`docs/db-design.md`、`docs/frontend-design.md`、
`docs/development-status.md`、公共 DTO 类型、功能目录和相应自动化行为追溯。
