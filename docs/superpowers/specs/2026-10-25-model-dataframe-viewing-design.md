# V3 模型 DataFrame 查看与受控读取设计

> 状态：已实现。本文扩展当前 V3 DataFrame 模型合同，不引入旧版本兼容层。

## 1. 意图

模型工作区目前只展示当前模型和可变草稿的表名、schema、行数与内容 hash。用户无法核对一张模型表的实际
数据；Agent 的 `inspect_workspace` 也只能读取表级摘要，但 `modify_model` 又要求准确的唯一 `rowKey` 和旧值
`from`。这使人工检查和自然语言修改都缺少可靠的数据读取入口。

本设计让用户在模型工作区直接查看当前模型、可变草稿和历史版本中的 DataFrame，并让 Agent 在修改前通过
受控只读工具查询实际行。模型修改仍由自然语言触发，查看是一项高频、无歧义的固定界面操作。

```text
模型工作区表卡片 ──查看──> 只读 ModelTableWorkbench ──分页/选择列/下载
                                             ▲
用户自然语言 ──Agent──> inspect_workspace ──┴──> read_model_dataframe
                                                   │
                                                   └──> modify_model ──> 新草稿 revision
```

## 2. 目标、非目标和约束

### 目标

- 当前 active 模型的每张 DataFrame 都可以查看实际行、选择列、分页和下载 Parquet。
- 可变草稿的每张 DataFrame 提供相同查看能力，并且一次查看只对应一个明确的 draft revision。
- 历史版本复用相同的只读查看能力，用户无需先激活版本才能核对内容。
- Agent 能按 active 或准确 draft revision、表名、投影列和等值条件读取少量模型行，再安全调用
  `modify_model`。
- 所有读取显式校验当前用户所有权；跨用户 ID 与不存在 ID 继续统一返回 404。
- 模型表与结果数据集共用 Parquet schema、分页和 Arrow-to-JSON 解码基础设施，但保持领域 DTO 和操作权限分离。

### 非目标

- 不在表格中直接编辑、批量粘贴或保存单元格；领域修改继续通过自然语言和 Agent 工具完成。
- 不把模型 DataFrame 注册为 `result_datasets`，不制造运行、结果、视图或分析血缘。
- 不给模型表增加结果图表、物化视图、删除结果或结果分析按钮。
- 不允许 Agent 提交 SQL、脚本、路径、任意谓词或无限制扫描。
- 不改变发布、激活、导入、运行和版本删除语义。

### 约束

- 模型正文继续只保存在 Snappy Parquet artifact 中，数据库 schema 无需变化。
- 浏览器与 Agent 只使用公开的模型 DataFrame ID、版本/草稿 ID、revision、表名和受控查询参数，不接触
  artifact 路径。
- `ModelVersion` 和其中的 `ModelDataFrame` 不可变；草稿每次成功写入都递增 revision。
- 公共路由、DTO、Agent 工具和功能目录必须同步，并有行为测试，不能只增加注册表枚举测试。

## 3. 方案选择

### 方案 A：独立模型表读取接口，复用底层 Parquet 读取能力（已选）

版本表和草稿表使用各自稳定、用户隔离的读取路由。前端使用专门的只读模型表工作台；后端抽取结果数据集与
模型表共用的 schema、投影、分页、cursor 和 Arrow-to-JSON 逻辑。Agent 工具调用同一应用服务，但只暴露
有界等值查询。

该方案保持模型与结果的领域语义清晰，同时避免重复实现复杂 Arrow 类型读取。代价是增加一组公共路由、一个
工具和专用前端 hook/component。

### 方案 B：把模型表临时包装为结果数据集（已排除）

该方案可以直接复用完整 `DatasetWorkbench`，但会污染结果列表、存储引用和血缘，也会错误暴露删除结果、
视图和分析能力。

### 方案 C：把预览行嵌入 workspace 快照（已排除）

该方案接口数量少，但每次工作区刷新都会读取多张 Parquet，响应随表数和列数膨胀，无法支持完整分页，也无法
清楚处理草稿 revision 变化。

## 4. 资源和一致性语义

### 4.1 当前模型

“当前模型”指 workspace 的 active `ModelVersion`。用户从当前模型卡片打开表时，前端保存打开瞬间的
`modelDataframeId` 和 `versionId`。版本不可变，因此后续切换 active 不会改变已经打开的数据；界面在工作区
刷新后可以标记“此版本已不再是当前模型”，但不强制关闭。

### 4.2 可变草稿

草稿查看必须同时绑定：

- `draftDataframeId`；
- `draftId`；
- `expectedDraftRevision`。

元数据、首个分页、后续分页和下载都检查三者仍匹配当前 workspace 草稿。只要任一草稿表被导入或修改，整个
draft revision 就会变化；旧请求返回 `409 draft_revision_conflict`，不能静默切换到新 artifact 或把两个
revision 的页面合并。前端保留已加载行，显示“草稿已更新”，并提供“重新加载最新修订”按钮；重新加载后清空
旧页和 cursor。

发布或放弃草稿后，旧草稿资源返回 404。草稿 artifact 是不可变文件；草稿只替换关系。读取服务解析并打开
通过所有权校验的 artifact，不缓存服务器路径。现有 artifact 引用和 GC 规则继续保证被草稿或版本引用的文件
可读；实现时需用共享的 artifact 打开方法消除“校验后按路径再次查找”的重复逻辑。

### 4.3 历史版本

版本历史中的表与 active 表使用同一 `model_dataframes` 读取接口。删除版本后，其表关系与 artifact 引用被
释放，原 ID 返回 404；已经打开的前端页面在下一请求时显示资源已删除。

## 5. HTTP API

新增六条只读路由：

| Method | Path | 语义 |
|---|---|---|
| GET | `/api/model-dataframes/{id}` | 当前用户拥有的存活版本表元数据 |
| GET | `/api/model-dataframes/{id}/rows` | 不可变版本表分页投影 |
| GET | `/api/model-dataframes/{id}/download` | 下载版本表的 Snappy Parquet |
| GET | `/api/draft-dataframes/{id}` | 指定 revision 的草稿表元数据 |
| GET | `/api/draft-dataframes/{id}/rows` | 指定 revision 的草稿表分页投影 |
| GET | `/api/draft-dataframes/{id}/download` | 下载指定 revision 的草稿 Parquet |

版本表路由使用 URL 中的 DataFrame ID。草稿三条路由额外要求 query 参数：

```text
draftId=<stable id>&expectedDraftRevision=<positive integer>
```

分页路由继续接受：

```text
cursor=<opaque>&limit=1..500&columns=comma,separated,names
```

元数据响应使用独立 DTO：

```json
{
  "id": "42",
  "scope": "draft",
  "ownerId": "7",
  "revision": 5,
  "tableName": "load",
  "rowCount": 120,
  "sizeBytes": 16384,
  "columns": [
    {"name": "id", "dtype": "Utf8", "nullable": false, "unit": null}
  ],
  "schemaHash": "...",
  "contentHash": "..."
}
```

`revision` 仅在 scope 为 `draft` 时存在。响应不包含物理路径、结果 views、lineage、run ID 或删除能力。

模型表分页响应使用新的 `ModelDataFramePage`：

```json
{
  "resourceId": "draft-dataframe:42:revision:5",
  "columns": ["id", "p"],
  "rows": [["load1", 100.0]],
  "nextCursor": null,
  "diagnostics": {}
}
```

现有结果接口及其 `DatasetPage.datasetId` 保持不变，不发生公共 API 破坏性修改。两类分页 DTO 只在后端共用
内部 page core 和 Arrow 解码器；模型 DTO 使用 `resourceId`，结果 DTO 继续使用 `datasetId`。

cursor 绑定资源类型、DataFrame ID、content hash 和草稿 revision；不能在结果数据集、另一个模型表或另一个
草稿 revision 上复用。列投影必须存在且去重，空投影表示全部列。Arrow 值编码与结果数据集一致，包括
`Utf8View`、List/LargeList/ListView、定长 List 和非有限浮点诊断。

下载文件名分别为 `model-v{versionId}-{tableName}.parquet` 和
`draft-{draftId}-r{revision}-{tableName}.parquet`，表名经过现有安全文件名规则处理。

稳定错误：

- `404 not_found`：资源不存在、已删除、已发布草稿或不属于当前用户；
- `409 draft_revision_conflict`：draft ID、revision 或 DataFrame 与当前草稿不一致；
- `400 invalid_page`：limit、列投影或 cursor 不合法；
- `500 dataframe_read_failed`：已登记 artifact 无法读取或 schema 与登记信息不一致。

## 6. 后端设计

### 6.1 Repository

`VersionRepo` 增加两个显式接收 `UserId` 的查询：

- 按 `model_dataframes.id` 连接 `model_versions -> workspaces -> sessions`，只返回存活且属于用户的版本表；
- 按 `draft_dataframes.id` 连接 `workspace_drafts -> workspaces -> sessions`，在同一查询中校验 draft ID 和
  expected revision。

查询返回表记录、所属版本/草稿摘要和 artifact ID。artifact ID 只在应用层内部使用，不作为工具参数。

### 6.2 共享 Parquet 读取器

从 `DatasetService` 抽取内部 `ParquetDataFrameReader`，负责：

- 从登记 schema 生成列元数据和 schema hash；
- 校验列投影和 `1..=500` 页大小；
- 编解码绑定资源身份与内容 hash 的 cursor；
- 调用 `ParquetInspector::projection`；
- 将 Arrow 标量、字符串、列表和 null 统一转换为 JSON；
- 汇总 missing/nonfinite 等诊断；
- 生成只读下载描述。

`DatasetService` 和新增 `ModelDataFrameReadService` 组合该读取器。结果数据集仍负责 views、lineage、删除和结果
事件；模型读取服务不依赖 `ResultRepo`，也不伪造 `DatasetMetadata`。

### 6.3 校验顺序

一次请求按以下顺序处理：

1. repository 用 `UserId` 解析公开 DataFrame ID，并检查存活状态；
2. 草稿请求在同一查询中校验 draft ID 和 expected revision；
3. ArtifactService 按当前用户 storage scope 校验 artifact 所有权并打开文件；
4. 共享读取器校验登记 schema/content hash 对应的 cursor 和列投影；
5. 返回元数据、页或下载流。

跨用户和猜测 ID 在第一步统一收敛为 404。草稿确实属于当前用户但 revision 已变化时才返回 409，避免通过
错误差异泄漏其他用户资源。

## 7. Agent 受控读取与修改闭环

新增只读工具 `read_model_dataframe`。它只读取当前会话 workspace，不接受 session ID、路径、SQL 或脚本。

参数：

```json
{
  "scope": "draft",
  "tableName": "load",
  "columns": ["id", "p"],
  "equals": {"id": "load1"},
  "limit": 20,
  "expectedDraft": {"id": 7, "revision": 5}
}
```

规则：

- `scope` 只允许 `active | draft`；读取 draft 必须给出与 `inspect_workspace` 一致的 `expectedDraft`；读取
  active 时 `expectedDraft` 必须为空。
- `columns` 必须是登记列名且最多 32 列；`equals` 是最多 8 个列的 AND 等值匹配，不支持表达式、范围、
  正则、排序、聚合或用户提供代码。过滤列不要求出现在输出 `columns` 中，后端读取两者并集后再投影输出。
- `equals` 只接受非 null 的 JSON string、boolean 或有限 number。字符串可匹配 `Utf8`、`LargeUtf8` 和
  `Utf8View`；boolean 只匹配 Boolean；整数必须无损落入目标有符号/无符号整数宽度；浮点值转换为目标
  Float32/Float64 后按确切值比较。禁止整数/浮点列之间隐式换类，禁止 null、数组、对象和复杂列条件。
- `limit` 默认为 20，范围 `1..=100`。查询以流式 batch 扫描完整 DataFrame 并计算准确 `matchCount`，只保留
  前 `limit` 行；`truncated` 当且仅当 `matchCount > returnedRows.length`。因此 Agent 只有在
  `matchCount == 1` 时才能把目标视为唯一，不能根据首个返回行猜测唯一性。扫描仍受现有 artifact 大小、工具
  调用超时和并发门禁约束，不新增一个会让表尾行永远无法修改的行数截断值。
- 返回列名、匹配行数组、准确 `matchCount`、实际 scope/version/draft/revision、content hash 与
  `truncated`；不返回 artifact ID 或路径。序列化工具输出最多 64 KiB，超过时返回稳定错误并要求缩小列投影，
  不截断单元格后继续声称结果完整。
- 查询使用与公共模型读取相同的 Arrow-to-JSON 实现，但工具侧增加过滤、扫描和输出上限。

Agent 的模型修改顺序更新为：

1. 加载 `model-intake` skill；
2. `inspect_workspace` 确定 active/draft 和 revision；
3. `read_model_dataframe` 查询目标行和实际旧值；
4. 只有结果唯一且用户意图明确时调用 `modify_model`；
5. `modify_model` 继续由服务端复核唯一 row、`from`、类型、单位和 expected draft；
6. 回复用户草稿已更新及新 revision，并提示可查看草稿、确认后点击“生成版本”。

读取工具不削弱 `modify_model` 的旧值校验。读取与写入之间发生并发变化时，expected draft 或 `from` 校验仍会
拒绝旧请求，Agent 重新检查工作区和目标行后再处理。

本功能同时修正文档中“Agent 可读取模型”的含糊表述：`inspect_workspace` 只负责目录和摘要，
`read_model_dataframe` 才负责有界行读取。

工具稳定错误码为：

- `model_scope_unavailable`：请求的 active 或 draft 不存在；
- `table_not_found`：当前 scope 中没有该表；
- `invalid_model_query`：列、limit、equals 数量、值类型或列类型不合法；
- `draft_revision_conflict`：expected draft 与当前草稿不一致；
- `model_query_output_too_large`：选择列产生的序列化输出超过 64 KiB；
- `dataframe_read_failed`：登记的 Parquet 无法完成读取或 schema 校验。

工具错误不暴露 artifact、服务器路径或其他会话是否存在。Prompt 对 revision 冲突必须重新
`inspect_workspace`；对无效查询或输出过大必须缩小列与条件；存储读取失败只报告诊断，不能继续调用
`modify_model`。

## 8. 前端交互

### 8.1 工作区入口

“当前模型”“可变草稿”和“版本历史”都把每张 DataFrame 渲染为可聚焦的表卡片，展示表名、行数和字段数，
并提供高频固定按钮“查看”。表名本身也可点击并具有相同语义；不能用不可访问的整行点击区域替代 button。

历史版本条目先展开表清单，再选择一张表查看。激活、删除和查看是独立按钮，点击查看不会改变 active，也不会
触发聊天消息。

### 8.2 ModelTableWorkbench

模型表在与结果数据集相同的居中遮罩层打开。专用工作台复用通用 `DataFrameTable`、列选择器、加载更多和下载
控件，标题明确显示来源：

```text
当前模型 · 版本 12 · load
可变草稿 · 修订 5 · load
历史版本 9 · load
```

模型工作台不显示结果视图标签、“删除结果”、分析或血缘。关闭后返回原工作区滚动位置。390px 窄屏占满可用
区域，并保持关闭、列选择和加载更多均可键盘访问。

加载状态分为元数据、首屏数据和后续分页；后续分页失败时保留已加载数据。草稿返回
`draft_revision_conflict` 时停止继续分页，显示“草稿已更新”，提供“重新加载最新修订”和“关闭”。重新加载先
刷新 workspace，再按同名表的新 ID/revision 打开；该表已不存在时关闭工作台并给出提示。

### 8.3 状态边界

前端打开模型表时使用判别联合，而不是把模型 ID 塞入 `datasetId`：

```ts
type OpenDataFrame =
  | { kind: 'result'; datasetId: string }
  | { kind: 'model-version'; dataframeId: string; versionId: string; tableName: string }
  | { kind: 'model-draft'; dataframeId: string; draftId: string; revision: number; tableName: string }
```

切换会话、退出登录或删除相应资源时关闭工作台。workspace SSE/重载只更新来源状态，不把已经打开的不可变版本
切换成新的 active。

## 9. 数据库与生命周期

本功能不新增表、列、触发器或索引。读取使用现有：

- `model_dataframes(version_id, table_name, artifact_id, schema_json, row_count, content_hash)`；
- `draft_dataframes(draft_id, table_name, artifact_id, schema_json, row_count, content_hash)`；
- `workspace_drafts.revision`；
- `storage_scope_artifacts` 所有权和引用。

读取不创建结果、artifact 引用、消息或审计资源。下载和分页不会延长版本/草稿生命周期。发布、激活时放弃草稿、
版本删除和会话删除继续按现有事务释放引用；后续请求按 404/409 规则收敛。

## 10. 验证和功能追溯

功能目录新增一项模型 DataFrame 读取能力，并把新增路由、工具和前端视图加入公共合同。行为覆盖至少包括：

- repository：版本表和草稿表按 `UserId` 隔离，错误用户与不存在 ID 同为 404；
- API：元数据、列投影、cursor 分页、下载 MIME/文件名、复杂 Arrow 类型和非法列/limit；
- 草稿一致性：正确 revision 可读，任意草稿变更后旧元数据、cursor 和下载均返回 409，发布后返回 404；
- 生命周期：历史版本不激活也可读，版本删除后不可读；查看不创建数据库记录或 artifact 引用；
- Agent 工具：只允许 active/draft、受控等值过滤和上限，跨会话/路径/SQL 无入口；旧 revision 被拒绝；
- 修改闭环：Agent 先读取唯一行和旧值，再修改产生新 revision，active 内容保持不变，用户发布后才激活；
- 前端：三类工作区入口、只读控件、分页错误保留已有行、草稿冲突重载、会话切换清理和窄屏/键盘访问；
- 回归：结果 `DatasetWorkbench` 保留图表、删除和血缘能力，模型工作台不会出现这些控件。

同步更新：

- `docs/api-design.md`；
- `docs/frontend-design.md`；
- `docs/model-workspace-design.md`；
- `docs/agent-prompt-skills-design.md`；
- `docs/db-design.md`（明确无 schema 变更和读取所用关系）；
- `docs/development-status.md`；
- `integration-tests/catalog/features.yaml` 及其 route/tool/view 合同计数；
- `backend/resources/agent/v3/manifest.json`、system/skill index/hash 和 `model-intake.md`。

候选实现必须通过后端 fmt、Clippy、`cargo test --features e2e-test`，前端 Vitest/build、Playwright，以及
`./scripts/check-functional-coverage --require-pr`。真实依赖门禁继续按 `docs/v3-deployment.md` 执行。
