# 结果读取、分析、比较与图表可维护性设计

日期：2026-11-05  
阶段：仓库可维护性重构 8.6  
状态：已授权实施

## 1. 背景

V4 已经形成以 Snappy Parquet 为事实来源的结果链路，并具有受控分析、运行比较和六类受控图表配置。当前公共行为完整，但内部边界仍不均衡：

- `ParquetDataFrameReader` 已统一分页、列投影、下载和 Arrow 值转换；数据集、分析和图表仍分别解析“当前用户可读的结果来源”。
- 分析服务同时负责输入快照、工作树准备、引擎执行、输出校验、artifact 提交、数据集发布和消息投影。
- 比较服务同时负责运行校验、数据集配对、兼容性推导、脚本生成、分析调度和比较记录发布。
- 图表编译器把公共候选值处理、聚合/排序和六类图表算法放在同一文件；图表服务同时负责来源、编译、物化和已保存视图生命周期。
- 前端 `ChartBuilder` 把初始化、字段映射、配置生成和全部控件放在同一组件。

本阶段是结构性重构，不改变用户功能或数据契约。

## 2. 目标

1. 为结果数据建立唯一的、用户作用域内的只读来源描述，供表格、分析输入和图表读取复用。
2. 保留 `ParquetDataFrameReader` 作为分页、投影、下载和批次扫描的唯一应用层读取内核。
3. 分离分析与比较的策略、执行和发布职责，同时保持既有状态、错误码和事务边界。
4. 将图表编译公共流水线与六类图表算法分离，不合并语义不同的算法。
5. 将前端图表构建器拆成纯配置状态、字段映射、模板控件和壳组件；ECharts 继续只消费服务端 `chart-model-v1`。
6. 用特征测试锁定来源删除、血缘快照、幂等、用户隔离、分页和图表模型行为。

## 3. 非目标和硬边界

- 不新增或修改公共路由、DTO、消息类型、稳定错误码、数据库 schema 或 migration。
- 不让前端或 LLM 提交任意 ECharts option、服务器路径、命令、SQL 或脚本之外的非受控执行参数。
- 不恢复 CSV 链路，不引入固定 metrics；结果仍以 Snappy Parquet 为事实来源。
- 不把分析、比较和图表合并成一个通用状态机。
- 不改变图表种类、默认样式、预算、采样、聚合、排序或保存后的 Plot Data 语义。
- 不移动或复制 `tensoreval` 调用；所有执行仍经 `backend/src/adapters/`。

## 4. 方案比较

### 4.1 仅做文件切分

把大文件拆小，但不建立共享来源边界。改动较机械，却会让 `ResultRepo::find_live`、artifact 所有权校验、schema 解析和 `DataFrameReadSource` 构造继续散落，后续仍可能出现读取语义漂移。

### 4.2 通用派生结果流水线

把分析、比较和图表统一为同一执行/发布抽象。表面重复更少，但三者有不同的执行介质、幂等语义、事务边界、删除行为和错误模型，会形成条件分支密集的错误抽象。

### 4.3 分层收敛（采用）

只共享稳定事实和无副作用读取能力：用户作用域内的来源描述、schema、artifact 身份、内容 hash 与 DataFrame 读取。分析、比较、图表各自保留编排和发布边界。该方案能消除真正的规则重复，同时避免把相似但不同的算法和生命周期强行统一。

## 5. 后端设计

### 5.1 只读结果来源边界

新增应用层结果来源模块，提供内部 `ResultSource` 值对象与 `ResultSourceService`：

- 输入：`CurrentUser` 和稳定 dataset ID。
- 输出：dataset 身份、生产者、名称、schema JSON/已解析列、schema hash、行数、artifact ID、artifact 内容 hash，以及可直接交给 `ParquetDataFrameReader` 的 `DataFrameReadSource`。
- 查找顺序：先通过 `ResultRepo` 做显式 `UserId` 和 live 状态校验，再通过 `ArtifactService` 做 storage scope 所有权校验。
- 对外只暴露应用层错误，不泄漏路径；不存在、跨用户和已删除仍统一映射为既有 404/业务错误。

它是只读事实解析器，不持有删除、发布或事务职责。数据资源读取继续使用其现有独立来源类型，因为资源 ID、生命周期和可用 artifact 规则不同。

`DataFrameReadSource.content_hash` 统一使用 artifact SHA-256，而不是 schema hash。cursor 仍由读取器绑定用户、资源键、投影和内容身份。

### 5.2 数据集读取与生命周期

`DatasetService` 保持公共门面：

- metadata/list/list-analysis 仍组合结果行、视图摘要和血缘快照。
- page/download 改为复用 `ResultSourceService`，不再自行构造来源。
- delete 仍由 `ResultRepo` 的事务实现活动分析保护和 tombstone，再释放 artifact、执行回收并投递消息。

元数据投影、血缘投影和 schema hash 生成可移动到专用内部模块，但返回 DTO 和 JSON 形状不变。派生数据集在来源删除后继续展示冻结血缘；被删除的来源本身不可再次读取或作为新分析输入。

### 5.3 分析服务

保留 `AnalysisService` 门面与既有 `run/find` 行为，内部拆为四个职责：

1. **输入快照**：解析 dataset/data-resource 输入，验证会话，产生不可变 `ResultAnalysisInput`，并同时生成执行所需的 artifact 引用。dataset 输入使用共享结果来源；data-resource 使用现有资源服务。
2. **执行准备**：创建受控临时目录、复制已验证 artifact、生成 `NamedInput`；不接受外部路径。
3. **输出收集**：校验声明输出、Snappy Parquet、schema/大小，暂存并提交 artifact，失败时释放本次已提交引用。
4. **发布与投影**：调用 repository 原子完成分析与结果数据集发布，再发出结果和进度消息。

输入快照和执行准备必须消费同一轮解析结果，避免先快照、后重新查询来源造成重复规则或竞态窗口。现有 `pending/script/run/collect/present` 阶段、bundle/skill/engine contract 记录和错误码保持不变。

### 5.4 运行比较

比较继续通过受控分析流水线实现，但拆分为：

- 运行资格校验；
- 结果集配对与 schema 兼容性推导；
- 受控 RSS 生成；
- 分析调度；
- 比较快照、记录和消息发布。

兼容性模块只处理纯值：稳定键、同名数值列、dtype、单位和维度。RSS 生成只接受该受控兼容性模型，并继续校验合法标识符。它不读取文件、不访问数据库，也不成为通用用户脚本生成器。

### 5.5 图表服务与 Plot Data 生命周期

`ChartService` 保持公共门面，内部依赖共享结果来源：

- capability、preview、save 使用同一种来源/schema/content hash。
- preview 只返回临时 `chart-model-v1`。
- save 仍先处理 client operation 幂等，再编译、写入 Snappy Plot Data，并通过现有 artifact/repository 原子发布边界提交视图。
- 已保存视图读取只依赖冻结配置、schema、诊断和 Plot Data artifact；来源删除后仍可读取，`sourceDeleted` 继续反映 tombstone 状态。
- 首次编译或发布前来源被删除仍返回 `chart_source_deleted`；已保存 Plot Data 丢失仍是物化错误。

来源解析、materialization、视图 DTO/cursor/lifecycle 分别成为内部模块，但不改变 repository 的事务职责。

### 5.6 图表编译器

保留 `ChartCompiler::compile` 入口和 `CompiledChart` 输出，内部拆为：

- `compiler/source`：列索引、字段/路径解析和候选值读取；
- `compiler/common`：公共缺失值策略、series key、聚合、排序、预算与单位规则；
- `compiler/line`、`bar`、`scatter`、`heatmap`、`boxplot`、`histogram`：各自算法；
- `compiler/schema`：Plot Data schema 构造；
- `compiler/tests` 或相邻模块测试：锁定原有模型输出和诊断。

只有完全同义、共享不变量的逻辑进入 `common`。例如 line/bar 可共享分类聚合与排序原语，但仍保留独立输出算法；boxplot 四分位和 histogram 分箱不得为了减少文件数而合并。

## 6. Repository 边界

- `ResultRepo` 继续拥有 live dataset、run/analysis 列表、视图摘要、血缘和删除事务。
- `AnalysisRepo` 继续拥有分析创建、阶段迁移、完成/失败和输入快照的事务。
- `ChartViewRepo` 继续拥有幂等操作、视图发布关联、列表、查找和删除事务。
- 重复的用户作用域 SQL 只有在能保持完全相同的 join、404 和事务语义时才下沉为同一 repository 内部查询辅助；不跨 repository 建立泛型 SQL 框架。
- 每个公开 repository 查询继续显式接收 `UserId`。

## 7. 前端设计

`ChartBuilder` 拆成以下层次：

1. `chartBuilder/fieldMapping.ts`：`ChartFieldRef` identity/key/name、capability 字段选项和保留 label/path 的纯函数。
2. `chartBuilder/state.ts`：编辑状态类型、默认值、从已有 `ChartConfig` 初始化、由状态生成受控 `ChartConfig` 的纯函数。
3. `chartBuilder/useChartBuilder.ts`：capability 变化、初次播种、preview freshness、保存幂等 operation ID 等副作用。
4. `chartBuilder/TemplateControls.tsx`：按图表种类显示字段、聚合、排序、采样、分箱和离群点控件。
5. `chartBuilder/PresentationControls.tsx`：标题、调色板、图例、网格和折线样式控件。
6. `ChartBuilder.tsx`：布局、错误/加载状态和预览/保存动作。

字段映射和状态转换通过纯单元测试验证；组件测试继续覆盖 capability 禁用、默认选择、已有配置复制、预览变旧、保存幂等和样式设置。前端 ECharts adapters 保持按图表类型分离，继续只把受控 `chart-model-v1` 确定性转换为 ECharts option。

## 8. 数据流与失败语义

```text
dataset ID + CurrentUser
        |
        v
ResultSourceService -----> ResultRepo (UserId + live)
        |                  ArtifactService (storage scope)
        v
ResultSource + DataFrameReadSource
   |              |                 |
 table read    analysis input    chart capability/compile
                                      |
                                      v
                              frozen Plot Data view
```

- 任一 live 来源解析失败：沿用调用方既有 not-found 或专用错误映射。
- 分析输出 artifact 部分提交失败：释放本轮已提交引用，不发布结果行。
- 图表保存发布失败：沿用 artifact 原子发布边界，不留下孤立 live view。
- 来源删除：阻止新读取/新派生；冻结血缘和已保存 Plot Data 继续可见。

## 9. 验证策略

先补特征测试，再重构：

- 同一 dataset 来源在 page/download/chart/analysis 中使用 artifact content hash 和一致的用户作用域。
- 跨用户、不存在和已删除来源继续不可见。
- 活动分析阻止来源删除；完成的派生结果保留 tombstone 血缘。
- 比较继续选择稳定键、拒绝单位不兼容、只生成受控函数。
- 六类图表的模型、预算、采样、聚合、排序、缺失值和诊断保持不变。
- 保存图表的幂等、名称冲突、来源删除竞态和删除后读取保持不变。
- 前端配置状态与字段路径/label 往返一致，preview freshness 和保存按钮行为不变。

阶段完成时执行：

```bash
cd backend
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --features e2e-test

cd ../frontend
npm test -- --run
npm run build

cd ..
./scripts/check-functional-coverage --require-pr
```

真实 eig-rs 候选门禁继续按 `docs/v4-deployment.md` 执行；若仍只有已记录的外部 `case_projection_invalid`，必须明确记录为外部资源兼容性阻塞，不能把它表述为本阶段通过。

## 10. 文档与完成标准

完成后更新：

- `docs/architecture.md`：结果来源与图表编译边界；
- `docs/development-status.md`：8.6 完成内容和验证证据；
- 仅当公共语义意外需要调整时才更新 API/DB/前端设计和功能目录；本设计默认不触发这些变更。

完成标准：公共契约零变化；共享来源规则只有一个实现；大编译器和构建器按职责拆分；所有仓库门禁通过，外部门禁状态被准确报告；工作树不包含与本阶段无关的修改。
