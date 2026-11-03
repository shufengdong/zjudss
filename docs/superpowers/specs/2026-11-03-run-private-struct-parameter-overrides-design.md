# 研究参数运行私有覆盖设计

**日期：** 2026-11-03
**状态：** 已确认

## 意图

研究配置卡中的 `para` 等 Struct 输入是一次研究的运行参数，不应因为用户把 `max_iter`、`tolerance` 等字段改成了本次需要的值，就强迫用户覆盖或另存一个长期数据资源。长期 Struct 资源继续作为可复用参数模板；研究卡中的修改只影响本次配置及其工作流。

用户明确在“数据文件”工作台修改参数资源时，仍沿用现有结构化资源预览与 Replace/SaveAsNew 流程。研究配置确认不再隐式发布、替换或另存 Struct 资源。

## 目标

- 所有用户可编辑的 Struct 字段默认成为运行私有覆盖，不按 `max_iter`、`tolerance` 等字段名硬编码。
- 结果目录控制的 selector 字段继续由服务端管理，用户不能在 Struct 更新请求中伪造。
- 执行输入按“基础模板 + 用户覆盖 + 结果 selector 覆盖”生成不可变的单行 Snappy Parquet。
- 研究确认不创建 `data_resources` 行、不占用资源 alias，也不要求 Replace/SaveAsNew。
- 重试和生成脚本重跑保留原用户覆盖和 selector 覆盖；基础资源变化时将这些覆盖重新应用到最新模板。
- 没有基础 Struct 资源时也能用目录默认值和用户输入生成运行私有 artifact。
- 工作流快照能审计来源、用户覆盖、selector 覆盖和最终有效值，并兼容已有快照。

## 非目标

- 不改变“数据文件”工作台对长期 Struct 资源的显式修改、替换和另存能力。
- 不在研究配置卡新增“保存为参数模板”按钮；需要长期保存时使用现有数据资源参数编辑入口。
- 不改变 Data 输入列映射的发布语义，也不改变模型版本生命周期。
- 不让浏览器或 LLM 接触服务器路径、Parquet 内容或任意脚本参数。
- 不修改已经确认的历史工作流或运行 artifact。

## 方案比较

### 方案 A：所有研究 Struct 修改均为运行私有覆盖（采用）

资源只提供基线，研究配置保存字段值和相对基线的用户覆盖；确认时生成私有 artifact。规则按 Struct 输入类型生效，因此新增加的普通参数字段自动适用。

优点是用户心智一致、不会产生资源膨胀，也不要求引擎目录逐字段标注。代价是研究卡不再顺便承担长期模板编辑；长期保存必须去数据文件工作台显式完成。

### 方案 B：由目录逐字段声明持久化策略

在 catalog schema 中为每个 Struct 字段增加 `run_private` 或 `resource_persistent`。它最细粒度，但需要升级引擎目录、snapshot hash、schema 校验和全部案例资源，且用户仍要理解同一张参数卡中两种保存语义。本次需求没有必须长期写回的研究字段，因此不采用。

### 方案 C：保留当前 Replace/SaveAsNew，只让更多特定字段进入 selector overlay

可以把 `max_iter`、`tolerance` 加入一个硬编码或配置字段列表，但每新增算法参数都要继续维护例外，且“哪些字段会产生资源”不透明。这会把结果 selector 的特殊机制扩展成另一套隐式规则，不采用。

用户已确认采用方案 A：参数资源是模板，研究配置修改是本次运行覆盖，只有显式资源编辑才发布长期资源。

## 领域模型

### 基础值和覆盖值

每个 Struct 输入形成四层可审计状态：

1. `source`：可空的长期资源身份、artifact、schema hash 和 content hash。
2. `userOverrides`：用户可编辑字段相对基础值发生变化的最小对象。
3. `selectorOverrides`：服务端根据有序结果选择生成的字段对象。
4. `effective`：依次应用两类覆盖并完成 schema 校验后的完整 Struct 值。

为兼容现有重试与重跑读取器，冻结快照继续保留合并后的 `effective.overrides`，其内容为 `userOverrides` 后再应用 `selectorOverrides` 的结果；同时新增分类字段用于审计。selector 与普通字段集合必须互斥，冲突时视为配置或目录已过期。

### 基础来源

- 唯一兼容资源继续自动绑定；多个兼容资源仍要求用户选择，因为不同模板会改变未覆盖字段。
- 没有兼容资源时使用 Struct schema 的目录默认值建立配置。必填且无默认值的字段仍必须由用户填写。
- 有来源时，配置保存来源的 baseline；用户值只与该 baseline 比较生成 `userOverrides`。
- 用户把字段恢复为 baseline 后，该字段从 `userOverrides` 删除。
- JSON 数值继续按语义比较，`50` 与 `50.0` 不形成覆盖。

### 更新研究参数

`PUT /api/study-configurations/{id}/structs/{inputId}` 的当前产品请求收敛为：

```json
{
  "expectedRevision": 7,
  "values": {
    "max_iter": 30,
    "tolerance": 0.000001
  }
}
```

研究更新不再接收或保存 `replacementAction`。服务端忽略客户端对 selector 字段的值，恢复服务端基线/当前受控值后校验完整 Struct，保存配置值并递增 revision。已有未确认配置中残留的 replacement metadata 在下一次读取或更新时不再产生资源命令；不需要数据库迁移。

公共 DTO、API 文档和行为合同同步删除研究 Struct 更新中的 replacement action；上传批次和数据资源替换 DTO 不变。

## 确认和物化

确认准备对每个 Struct 输入执行：

1. 重新验证配置 revision、engine snapshot、schema 和可空来源资源的当前身份/hash。
2. 从冻结 baseline 计算 `userOverrides`，并拒绝未知字段或 selector 字段冲突。
3. 从结果选择计算 `selectorOverrides`。
4. 生成并验证完整 `effective` 值。
5. 若存在来源且 effective Parquet 与来源字节相同，则复用来源 artifact；否则提交工作流私有 artifact。
6. 原子确认配置、创建 workflow/run、冻结输入和建立 artifact 引用；不执行任何 Struct 资源 replacement。

没有来源时，私有 artifact 的 `source` 为 `null`，由 workflow/run 引用计数持有。事务失败、幂等冲突或准备失败时，所有新建私有 artifact 都必须释放。相同 `clientOperationId` 的重放必须在物化前返回既有操作，不能产生额外 artifact。

## 重试和生成脚本重跑

有来源的 Struct 输入继续比较资源 ID、schema 和 source content hash：

- 来源未变：可直接复用冻结的 effective artifact。
- 原资源被显式 Replace：只接受 `lineage.replacesResourceId` 可追溯到冻结资源的同 session、同 schema replacement 后代，读取该后代的完整模板，依次重放冻结的 `userOverrides` 和 `selectorOverrides`，重新校验并生成 effective artifact。
- 来源被删除、schema 不兼容或覆盖字段不再存在：返回稳定的 `configuration_stale`，不猜测替代资源。

“同 alias”本身不构成合法继承关系。案例切换发布的同名资源代表新的案例模板，删除后重新上传的同名资源也没有 replacement 血缘；两者都不能自动接收旧工作流覆盖，必须返回 `configuration_stale`。这比当前按 alias 查找任意 live 资源更严格，防止旧案例的 `max_iter`、`tolerance` 等设置被静默套用到另一个案例。显式 Replace 是唯一允许自动跟随的资源变化。

没有来源的 Struct 输入不寻找新资源，也不因后来出现同 schema 资源而改变含义；重试和重跑复用冻结 effective artifact 与覆盖快照。

旧工作流只有合并的 `effective.overrides` 时，按现有语义把它作为不可分的受控覆盖重放。新代码必须继续读取该格式，不迁移历史 JSON。

## 前端体验

Struct 参数卡继续显示当前基础资源身份或“使用目录默认值”，但移除“参数资源保存方式”选择。文案明确：

- 未修改：`当前值来自参数模板；本次研究不会修改模板`。
- 已修改：`本次运行覆盖：max_iter、tolerance；不会修改参数模板`。
- 无来源：`使用目录默认值；保存后仅用于本次研究`。

按钮使用“应用参数”，只提交 `expectedRevision` 和 `values`。配置摘要将普通字段差异标为“运行覆盖”，不再显示“覆盖/另存资源”。结果 selector 字段继续隐藏，并显示由结果选择自动写入的说明。

数据文件工作台的“修改参数”入口保持原有资源预览、Replace/SaveAsNew 和血缘语义，作为长期模板管理入口。

## 错误与并发

- 过期 revision、engine snapshot 或来源 hash 继续返回 `configuration_stale`。
- Struct 类型、范围、枚举、必填和条件规则继续由当前 schema 校验。
- 用户提交 selector 字段不会改变服务端值，也不会进入 `userOverrides`。
- 多个资源的歧义必须在参数编辑前解决，不能用上次资源或新出现的资源静默替代。
- artifact 配额、存储和事务错误不能留下孤立引用或半确认 workflow。
- API 幂等重放必须早于任何私有 artifact 生成。

## 测试与追溯

后端行为测试至少覆盖：

- `max_iter`、`tolerance` 修改后确认不创建、替换或退出任何 `data_resources` 行。
- 有来源时生成包含普通字段和 selector 的 effective artifact，snapshot 分类正确。
- 值恢复为 baseline 时不生成用户覆盖；effective 与来源相同时复用 artifact。
- 无来源时只生成工作流私有 artifact，并在失败/删除时正确释放。
- 多资源歧义、selector 伪造、数值等价和 schema 校验保持正确。
- retry/rerun 在来源未变、来源变化、无来源及旧快照四种情况下保持预期。
- retry/rerun 只跟随显式 replacement 后代；案例切换和删除后同 alias 重传都返回 `configuration_stale`。
- `clientOperationId`、`clientRetryId`、`clientRunId` 重放不额外物化。

前端行为测试至少覆盖：

- 参数卡不显示 Replace/SaveAsNew，提交体不含 `replacementAction`。
- 基础模板、运行覆盖、无来源三种文案与变更恢复正确。
- selector 字段仍隐藏，不参与普通变更计算。
- 配置摘要不再把研究参数变化描述成资源发布。

同步更新 `docs/api-design.md`、`docs/db-design.md`、`docs/frontend-design.md`、`docs/development-status.md`、`frontend/src/types/` 和 `integration-tests/catalog/features.yaml`。公共路由不新增，但请求 DTO 和研究确认行为变化必须有自动化行为证据。

## 实施边界

- 所有 Parquet 生成继续复用应用层受控 Struct 编码和 artifact 服务，不直接调用 tensoreval。
- 长期资源仍不可变；本设计只是把研究参数变化从资源发布链路移到 workflow 私有 artifact。
- 不修改当前工作区中与图表能力相关的未提交文件。
