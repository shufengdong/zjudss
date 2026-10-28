# V4 DataFrame 模型、资源、脚本与运行设计

## Model lifecycle

模型的唯一规范表示是一组 `{tableName, Snappy Parquet artifact, schema, rowCount, contentHash}`。DSS、目录、
bundle、ZIP 和表格文件只是统一上传来源；adapter 解析后返回 DataFrame 候选。单个 `.rss` 永远归为脚本。
候选必须选择 `model_table|data_resource|skip`；只有模型表进入草稿和版本，`para`、负荷预测等 Data/Struct
作为普通本地资源按稳定 ID、alias、schema/content hash 和 replacement 血缘管理。

上传首先持久化可恢复批次，然后异步检查/规范化。用户选择候选并确认后，模型子事务比较 expected draft、创建或
复制 draft、按 `tableName` 整表覆盖并推进 revision；资源子事务发布普通资源。两个子事务独立结算，部分成功只
保留失败侧所需的 staging 引用。放弃、失败和应用完成都幂等释放不再需要的来源。

DSS `Redirect`/`Compile` 依赖相对当前文件进行词法规范化。允许使用 `.`，也允许规范化后仍位于本次私有
输入树内的 `..`；绝对路径、平台前缀和任何越过输入树根的父目录引用返回 `input_path_invalid`。规范化后的
目标仍必须存在于本次导入树中，否则返回 `missing_dependency`。调用引擎 import profile 时，adapter 在私有
输入树根创建并在调用后清理每次导入唯一的 DSS wrapper 与 RSS 脚本，使 tensoreval 的 acquisition root 与
上述已验证边界一致；无法安全表示在 wrapper 中的双引号或控制字符入口返回 `input_path_invalid`。

draft 可随时变化，不是版本。只有用户点击“生成版本”才能冻结内容；发布计算按表名排序的规范 content hash，
写 publish receipt，清空 draft 并自动激活。相同存活内容去重复用；删除后的相同内容可获得新版本 ID。

## Model DataFrame reads

active、历史版本和草稿中的模型表都可以按稳定 DataFrame ID 只读查看元数据、分页行、列投影并下载原始
Snappy Parquet。版本表不可变，因此打开后即使 active 指针变化仍指向同一内容；历史版本无需激活即可读取。
草稿读取必须携带准确的 `draftId + expectedDraftRevision`，cursor 也绑定资源种类、ID、content hash 和 revision，
禁止把变化前后的行合并为一页。跨用户、已删除和不存在资源统一 404。

结果数据集与模型表共用内部 `ParquetDataFrameReader` 的 artifact 所有权检查、投影、分页、cursor 和
Arrow-to-JSON 解码，但使用独立 DTO 与权限。ArtifactService 用进程内读写门闩把所有权校验和文件打开与
release/GC 串行化，reader 向上只传已打开句柄；并发删除不能在校验与打开之间移除 blob，读取也不会持久增加
artifact 引用。模型表读取不创建 `result_datasets`，不增加 artifact 引用，也不
暴露结果删除、分析、视图或图表能力；数据库 schema 无需变化。

## Case library

案例列表只展示可导入来源。“使用案例”固定按钮以 engine resource snapshot 和稳定 example ID 创建统一
upload batch；随后展示 20 张静态模型表和普通 `para` 资源候选，必须确认用途后才发布。案例不会自动生成版本、
创建脚本或运行。

## Model mutation

Agent `modify_model` 只对 draft 中的一张 DataFrame 按唯一 `rowKey` 修改。请求必须包含 table name、旧值、
新值、单位和 expected draft；服务端校验单位维度、类型、唯一行与旧值，写新 Parquet artifact 并推进 revision。
若当前没有 draft，会从 active copy-on-reference 建立。修改成功后仍需用户点击“生成版本”。

Agent 在修改前先用 `inspect_workspace` 确定 active/draft 范围，再用 `read_model_dataframe` 按表名、投影列和最多
8 个标量等值条件读取。工具最多返回 32 列、100 行和 64 KiB JSON，并扫描整表给出准确 `matchCount`；只有
唯一目标行及旧值明确后才调用 `modify_model`。字符串、布尔、整数和有限浮点按 Arrow 类型严格比较，null、
数组、对象和复杂列不能作为等值谓词。

## Script lifecycle

脚本是 session 下可随时新增的不可编辑资源，可由上传、手工文本或确认后的专用脚本 author 创建；没有版本历史和单独删除。
V4 创建时只验证名称、UTF-8 和大小，RSS 语法与运行可行性完全以 tensoreval 为准；受管研究另由冻结配置做结构
一致性检查。无 active 时也允许保存，全文通过详情 API 读取。用户可复制、修改后以新 `.rss` 再上传。

## Run lifecycle

只有 `POST /api/scripts/{id}/runs`（脚本卡片按钮）创建 SimulationRun。创建事务验证脚本/session，并按脚本类型冻结当前输入：
成功 Agent 脚本只解析门面实际需要的最新 active 表和资源 alias，manual/upload 脚本使用存在时的全部 active 表及全部 live
Data resource；没有 active 也允许执行。worker 在私有输入树中按表名物化 DataFrame，调用 adapter
`RunMode::Dataframe`；adapter 在运行中和退出后验证全部 Snappy Parquet 输出并一次性返回，失败清理整个未发布目录。

终态运行在 impact token 校验后与全部直接结果原子 tombstone；非终态或任一直接结果被分析消费时拒绝整批删除。历史版本删除
继续按版本影响规则处理，已完成的派生和提升资源通过冻结血缘保持可读。

## Results and derived resources

用户可删除单个结果。若它正被 pending/running 分析使用则返回 `result_busy`；否则立即 tombstone、隐藏
metadata/rows/download，并释放 artifact，但运行继续存在。分析、视图和比较在开始时保存输入 schema/hash/
producer 快照，输出独立物化。来源删除后，新派生被禁止，已完成派生、视图和比较仍可读并显示 tombstone 血缘。
