# 用户配置式结果图表工作台设计

**日期：** 2026-11-03
**状态：** 已确认
**范围：** 仿真结果 `result_datasets` 的表格、图表配置、预览、保存与恢复

## 1. 背景

当前结果工作台默认展示 DataFrame 表格；持久图表由对话 Agent 加载 `result-analysis` skill 后调用
`create_dataset_view` 创建。工具接收宽松的 `spec: object`，应用层再把它解析成六类
`VisualizationSpec`。后端为视图复制整份源 Parquet，创建一条没有实际 RSS 计算的
`result_analysis_run` 和派生 `result_dataset`，再把 `dataset_views` 挂到源结果。前端读取
`spec + columns + rows`，按图形类型编译为 ECharts option。

这条链路存在四类结构性问题：

1. 图表配置依赖大模型正确生成稳定 ID 和内部 spec；工具的输入 schema 没有公开 `kind/x/y` 的完整结构。
2. 当前 `kind + x + y[]` 只覆盖标量宽表，无法可靠表达 `List` 分量、矩阵、长表分组和重复键聚合。
3. 后端把整份源结果复制为所谓“物化视图”，没有真正物化图表所需的窄投影和转换结果，并制造虚假的分析脚本审计记录。
4. 用户看不到图表配置，也无法通过界面选择列、修正不兼容组合或从已有图表复制配置。

产品决定把图表创建从对话 Agent 移到结果工作台。用户从有限图形模板中选择字段和展示方式，后端负责读取
Parquet、校验列形状、展开分量、聚合、排序和点数预算，返回稳定 `ChartModel`；前端只把该模型适配为
ECharts option。保存后的图表是持久资源，刷新后可恢复，来源结果删除后仍可读取。

## 2. 目标与非目标

### 2.1 目标

- 数据表仍是打开结果后的默认视图；成功运行不自动打开表格或图表。
- 用户点击“新建图表”后，从六种固定模板选择列和必要参数，不接触 ECharts option。
- 用户可在默认收起的“显示设置”中调整图例、固定调色板、标题显示、网格线和折线线型/数据点；设置随图表持久化。
- 支持标量字符串/布尔分类列、标量数值列以及最多四层嵌套数值 List 的固定分量选择。
- 后端生成经过校验和预算限制的标准图表数据，前端不再解释源 DataFrame 的 List/Matrix 结构。
- 预览不产生持久写入；只有用户明确点击“保存图表”才发布不可变图表资源。
- 保存图表记录冻结的配置、源 schema/content 快照和物化 Plot Data；来源结果 tombstone 后图表继续可读。
- 所有公共 ID 出网为字符串；跨用户和不存在资源统一 404。
- 图表创建、读取、删除和 artifact 引用遵循当前配额、会话删除与 GC 合同。
- 同一套 `ChartConfig`、兼容性规则和 `ChartModel` 成为后端与前端之间唯一图表合同。

### 2.2 非目标

- 不接收或返回任意 ECharts option、JavaScript formatter、函数、URL、SQL、RSS 或服务器路径。
- 不建设完整 BI 工具；首版不提供任意 transform pipeline、计算字段、join、pivot 或自由多轴布局。
- 不用大模型选择字段、创建图表或修复图表配置。
- 首版不为普通 `data_resources` 或模型表提供图表构建器，只面向仿真/派生结果 `result_datasets`。
- 首版不把变长二维 Matrix 自动解释成带领域轴标签的热力图；没有显式标量 X/Y/value 列时保持表格可读。
- 首版不提供堆叠、手动轴范围、缩放滑块、任意字体/像素坐标、平滑曲线或逐系列自由颜色。
- 保存后的图表不原地修改；“基于此图表新建”把冻结配置复制到构建器，保存为新的图表资源。
- 不静默降采样、聚合或猜测单位；需要这些行为时必须由固定配置显式选择。

## 3. 方案选择

### 3.1 已考虑方案

1. **继续由 Agent 生成受控 spec。** 改动最小，但正确性仍依赖模型理解内部类型，用户无法检查配置，拒绝。
2. **后端直接返回完整 ECharts option。** 前端简单，但后端耦合 ECharts 版本、主题和布局，重新引入任意 option
   安全边界，拒绝。
3. **默认展示完整自由图表构建器。** 能力最强，但多数用户不会高频使用，界面复杂度接近 BI 产品，拒绝。
4. **受控模板 + 后端 ChartModel + 前端 ECharts 适配。** 默认路径简单，复杂度受限且合同可验证，采用。

### 3.2 核心决策

- 图形类型固定为 `line|bar|scatter|heatmap|boxplot|histogram`。
- 后端返回业务化 `ChartModel`，不是 ECharts JSON。
- 图表配置由用户在 UI 中产生，不再注册 `create_dataset_view` Agent 工具。
- 图表数据以窄化、转换后的 Snappy Parquet 物化；SQLite 保存配置和快照。
- 数据表默认打开；图表必须由用户点击创建或打开已保存图表。
- 预览和保存使用相同编译器；源结果不可变，因此保存时可以确定性重算，不需要短期 preview artifact。

## 4. 用户体验

### 4.1 进入结果

用户从研究成果或运行记录点击指定数据集的“查看”，打开 `DatasetWorkbench`：

- 默认选中“数据表”；
- 保留选择列、cursor 分页、加载更多和 Parquet 下载；
- 顶部增加“新建图表”；
- 已保存图表继续作为“数据表”旁的标签展示；
- 不自动选择默认图表，也不在运行成功后自动打开工作台。

### 4.2 新建图表

点击“新建图表”打开同一工作台内的配置区，按以下顺序渐进显示：

1. 选择图形模板；
2. 选择模板要求的 X、Y、分类、值或分组字段；
3. 对 List 列选择具体分量；
4. 填写图表名称和可选标题；
5. 选择模板允许的排序、重复键聚合或分箱参数；
6. 点击“预览”；
7. 预览成功后点击“保存图表”。

排序、聚合和采样默认收在“高级设置”中；只有检测到重复键或点数超限时才主动展开并聚焦相关字段。常用路径只要求
图形、横轴和指标，不把高级能力作为默认界面负担。

“显示设置”与数据高级设置分开且默认收起，只包含：图例显示/位置/方向、四套固定调色板、标题显示、网格线显示，以及
line 的线宽、实线/虚线/点线和数据点显示。系列名称继续通过字段 `label` 修改。预览使用当前显示设置；修改显示设置同样使
预览过期。桌面选择右侧图例时，窄屏可以为避免溢出自动落到底部，保存的是布局意图而不是固定像素。

配置器只显示与当前槽位兼容的字段。不兼容字段可以隐藏，也可以禁用并显示确定性原因，例如：

- “`bus` 是分类列，不能作为散点图数值横轴”；
- “`v_mag` 是数值列表，请选择具体分量”；
- “所选指标单位不同，首版不支持共享纵轴”；
- “相同 X/系列存在重复行，请选择聚合方式”。

### 4.3 List 分量

`LargeList(Float64)` 等列表列显示为可展开字段。后端 capability 响应提供观察到的层级和分量路径；界面展示：

```text
v_mag
├─ [0]
├─ [1]
└─ [2]
```

缺少目录语义时默认标签为 `v_mag[0]`。用户可为图例填写“A相/B相/C相”等展示名，展示名不改变源 schema。
变长 List 中某行不存在所选路径时，该点作为缺失值丢弃并计入 diagnostics；若所有行都缺失，则预览失败。

### 4.4 预览、保存与复制

- “预览”是显式按钮，不随每次字段变化自动请求。
- 修改任何配置后，已有预览标记为过期，必须重新预览才能保存。
- 保存请求携带 `clientOperationId`，网络重试不会重复创建图表。
- 已保存图表显示“基于此图表新建”和“删除图表”；不提供原地编辑。
- 复制时加载冻结 `ChartConfig`，用户修改后以新名称保存。

### 4.5 来源删除后的发现

来源结果删除后，原结果工作台入口消失，但物化图表仍是存活资源。右侧结果面板增加第四个顶层标签“图表”，按当前
session 列出图表名称、类型、来源名称、来源是否已删除和创建时间。点击后打开只按 view ID 读取的
`ChartViewWorkbench`，不得先请求源 dataset metadata。来源仍存活时，`DatasetWorkbench` 内的图表标签也复用同一个
view hook 和 renderer。用户可在两处按稳定 view ID 打开或删除图表。

## 5. 类型合同

### 5.1 字段引用

所有字段引用都从当前数据集 schema 解析，不接受表达式：

```ts
interface ChartFieldRef {
  column: string
  path?: number[]       // List/FixedSizeList 的零基分量路径，最多 4 层
  label?: string        // 图例展示名，1..64 Unicode 字符
}
```

约束：

- `column` 必须精确匹配现有列名；
- 标量列不得带 `path`；List 叶子必须是服务支持的标量类型；
- `path` 长度为 `1..=4`，每个索引为 `0..=1023`；
- `label` 只用于展示，不参与字段解析或血缘。

### 5.2 ChartConfig

公共请求使用带 `version: 1` 的六分支判别联合，未知字段一律拒绝。各图形不共享含糊的可选字段：

```ts
type ChartSampling =
  | { mode: 'none' }
  | { mode: 'lttb'; maxPointsPerSeries: number }

interface ChartPresentation {
  legend: {
    show: boolean
    position: 'auto' | 'top' | 'bottom' | 'right'
    orientation: 'auto' | 'horizontal' | 'vertical'
  }
  palette: 'default' | 'colorblind' | 'highContrast' | 'monochrome'
  title: { show: boolean }
  axis: { showGrid: boolean }
}

interface LineChartPresentation extends ChartPresentation {
  line: {
    width: 1 | 2 | 3 | 4
    style: 'solid' | 'dashed' | 'dotted'
    showSymbol: boolean
  }
}

type ChartConfig =
  | {
      version: 1
      kind: 'line'
      title?: string
      presentation: LineChartPresentation
      x: ChartFieldRef
      xAxisMode: 'category' | 'value'
      measures: ChartFieldRef[]
      splitBy?: ChartFieldRef
      order: 'source' | 'xAsc' | 'xDesc'
      aggregation: 'none' | 'mean' | 'min' | 'max' | 'sum'
      sampling: ChartSampling
    }
  | {
      version: 1
      kind: 'bar'
      title?: string
      presentation: ChartPresentation
      x: ChartFieldRef
      measures: ChartFieldRef[]
      splitBy?: ChartFieldRef
      order: 'source' | 'xAsc' | 'xDesc'
      aggregation: 'none' | 'mean' | 'min' | 'max' | 'sum'
    }
  | {
      version: 1
      kind: 'scatter'
      title?: string
      presentation: ChartPresentation
      x: ChartFieldRef
      measures: ChartFieldRef[]
      splitBy?: ChartFieldRef
    }
  | {
      version: 1
      kind: 'heatmap'
      title?: string
      presentation: ChartPresentation
      x: ChartFieldRef
      y: ChartFieldRef
      value: ChartFieldRef
      aggregation: 'mean' | 'min' | 'max' | 'sum' | 'count'
    }
  | {
      version: 1
      kind: 'boxplot'
      title?: string
      presentation: ChartPresentation
      category: ChartFieldRef
      value: ChartFieldRef
      splitBy?: ChartFieldRef
      showOutliers: boolean
    }
  | {
      version: 1
      kind: 'histogram'
      title?: string
      presentation: ChartPresentation
      value: ChartFieldRef
      bins: number
    }
```

首版不支持双 Y 轴。`line|bar|scatter` 的多个 measure 必须具有完全相同的 unit（包括全部为 `null`），否则返回
`chart_unit_conflict`。`splitBy` 只能引用标量字符串、布尔或整数分类列。measure 为 `1..=16`，`bins` 为
`1..=200`；LTTB 只允许 `xAxisMode=value` 的 line 使用，`maxPointsPerSeries` 为 `100..=10_000` 且必须小于该
series 的输入点数。category line 不使用 ordinal 伪装数值横轴进行采样。
bar 和 scatter 不支持 sampling，避免对无序点或离散柱做隐式选择。boxplot 由 `showOutliers` 明确决定是否返回离群点。
所有 presentation 字段必填，前端新建配置时使用确定性缺省值：图例 `show=true/position=auto/orientation=auto`、palette
为 `colorblind`、标题按是否有 title 自动显示、网格线开启；line 缺省 `width=2/style=solid/showSymbol=false`。后端不接受
缺省字段或未知样式值，避免同一视觉状态产生不同 config hash。

### 5.3 图形兼容矩阵

| 模板 | 槽位 | 允许类型 | 额外规则 |
|---|---|---|---|
| line | x | 标量分类或数值 | `xAxisMode` 必须匹配；value 模式不得使用字符串 |
| line | measures | 标量数值或数值 List 分量 | 至少 1，最多 16 |
| bar | x | 标量分类 | 重复 `(x,series)` 且 aggregation=none 时拒绝 |
| bar | measures | 标量数值或数值 List 分量 | 最多 16，类别最多 5,000 |
| scatter | x/measures | 标量数值或数值 List 分量 | X 与 Y 都必须有限数值 |
| heatmap | x/y | 标量分类 | value 必须是数值；重复 cell 必须聚合 |
| boxplot | category | 标量分类 | value 必须是数值；按 category/splitBy 分组 |
| histogram | value | 标量数值或数值 List 分量 | bins 为 1..=200 |

布尔只作为分类；字符串不做隐式数值转换；数值不按列名猜时间。当前 reader 不支持的 Arrow date/timestamp 类型不在首版
chart capability 中出现，表格读取能力也不因本设计放宽。

### 5.4 ChartCapabilities

`GET capabilities` 返回用于配置器的确定性元数据：

```ts
interface ChartCapabilityField {
  column: string
  dtype: string
  nullable: boolean
  unit: string | null
  scalarType: 'string' | 'boolean' | 'integer' | 'number' | 'unsupported'
  roles: Array<'category' | 'measure'>
  shape:
    | { kind: 'scalar' }
    | {
        kind: 'list'
        rank: number
        observedPaths: number[][]
        complete: boolean
        truncated: boolean
        reason: 'too_many_components' | null
      }
}

interface ChartCapabilities {
  datasetId: string
  schemaHash: string
  rowCount: number
  fields: ChartCapabilityField[]
  templates: Array<{
    kind: VisualizationKind
    enabled: boolean
    reason: 'no_category_field' | 'no_numeric_field' | 'capability_truncated' | null
  }>
  limits: {
    maxFields: number
    maxObservedPathsPerField: number
    maxSeries: number
    maxPoints: number
    maxCategories: number
    maxHeatmapCells: number
  }
}
```

```json
{
  "datasetId": "7",
  "schemaHash": "...",
  "rowCount": 16,
  "fields": [
    {
      "column": "bus",
      "shape": {"kind": "scalar"},
      "scalarType": "string",
      "unit": null,
      "roles": ["category"]
    },
    {
      "column": "v_mag",
      "shape": {
        "kind": "list",
        "rank": 1,
        "observedPaths": [[0], [1], [2]],
        "complete": true
      },
      "scalarType": "number",
      "unit": null,
      "roles": ["measure"]
    }
  ],
  "templates": [
    {"kind": "line", "enabled": true, "reason": null}
  ],
  "limits": {
    "maxFields": 256,
    "maxObservedPathsPerField": 64,
    "maxSeries": 100,
    "maxPoints": 50000,
    "maxCategories": 5000,
    "maxHeatmapCells": 100000
  }
}
```

固定大小 List 从 Arrow schema 得到精确路径；变长 List 通过窄列扫描发现路径。扫描最多检查配置值
`CHART_CAPABILITY_SCAN_ROWS`（缺省 10,000）行，超过时 `complete=false`。capability 只用于 UI 选择；预览和保存仍完整扫描
所选投影并做权威校验，不能把抽样当作发布证明。

capability 另外限制最多 256 列、每个 List 最多返回 64 个路径、全响应最多 1 MiB。列数或响应整体超限返回
`chart_capabilities_too_large`；单个 List 超过 64 个路径时保留字段但标记
`shape={kind:'list',truncated:true,reason:'too_many_components'}`，该字段不能进入首版图表槽位，不能静默只展示前 64 个路径。

### 5.5 ChartModel

后端不返回 `EChartsOption`。前后端共享以下完整判别联合；实现不得增加未列出的透传 option 字段：

```ts
type CategoryValue =
  | { kind: 'string'; value: string }
  | { kind: 'boolean'; value: boolean }
  | { kind: 'integer'; value: string } // 十进制，避免 Int64/UInt64 超过 JS 安全整数
  | { kind: 'number'; value: number }

interface ChartModelBase {
  contractVersion: 'chart-model-v1'
  viewId: string | null
  sourceDatasetId: string
  sourceDeleted: boolean
  kind: VisualizationKind
  name: string | null
  title: string | null
  configHash: string
  diagnostics: {
    inputRows: number
    candidatePoints: number
    outputPoints: number
    droppedMissing: number
    droppedNonFinite: number
    missingComponents: number
    aggregatedGroups: number
    sampledPoints: number
  }
}

interface LineChartModel extends ChartModelBase {
  kind: 'line'
  presentation: LineChartPresentation
  order: 'source' | 'xAsc' | 'xDesc'
  aggregation: 'none' | 'mean' | 'min' | 'max' | 'sum'
  xAxis: {
    type: 'category' | 'value'
    name: string
    unit: string | null
  }
  yAxis: { name: string | null; unit: string | null }
  series: Array<{
    name: string
    points: Array<{ x: CategoryValue | number; y: number; ordinal: number }>
  }>
}

interface BarChartModel extends ChartModelBase {
  kind: 'bar'
  presentation: ChartPresentation
  xAxis: { type: 'category'; name: string; unit: string | null }
  yAxis: { name: string | null; unit: string | null }
  categories: CategoryValue[]
  series: Array<{ name: string; values: Array<number | null> }>
}

interface ScatterChartModel extends ChartModelBase {
  kind: 'scatter'
  presentation: ChartPresentation
  xAxis: { type: 'value'; name: string; unit: string | null }
  yAxis: { name: string | null; unit: string | null }
  series: Array<{
    name: string
    points: Array<{ x: number; y: number; ordinal: number }>
  }>
}

interface HeatmapChartModel extends ChartModelBase {
  kind: 'heatmap'
  presentation: ChartPresentation
  xAxis: { type: 'category'; name: string }
  yAxis: { type: 'category'; name: string }
  xCategories: CategoryValue[]
  yCategories: CategoryValue[]
  value: { name: string; unit: string | null; min: number; max: number }
  cells: Array<{ xIndex: number; yIndex: number; value: number }>
}

interface BoxplotChartModel extends ChartModelBase {
  kind: 'boxplot'
  presentation: ChartPresentation
  categoryAxis: { name: string; categories: CategoryValue[] }
  valueAxis: { name: string; unit: string | null }
  series: Array<{
    name: string
    values: Array<[number, number, number, number, number] | null>
    outliers: Array<{ categoryIndex: number; value: number }>
  }>
}

interface HistogramChartModel extends ChartModelBase {
  kind: 'histogram'
  presentation: ChartPresentation
  value: { name: string; unit: string | null }
  bins: Array<{ start: number; end: number; count: number; includesEnd: boolean }>
}

type ChartModel =
  | LineChartModel
  | BarChartModel
  | ScatterChartModel
  | HeatmapChartModel
  | BoxplotChartModel
  | HistogramChartModel
```

`CategoryValue` 使用带标签的联合，不把字符串 `"1"`、整数 `1` 与浮点数 `1.0` 混为一类。整数一律以十进制字符串出网，
前端只把它作为类别标签，不转成 JS number；value 轴只接受能够无精度损失转成有限 `f64` 的数值，超出 JS 安全整数范围的
Int64/UInt64 只能作为 category。所有数值在服务端过滤为有限 JSON number；
line 的 `xAxis.type=category` 时 point.x 必须是 `CategoryValue`，`type=value` 时必须是普通有限 number；两种表示不得混用。
line model 回显受控的 `order/aggregation`，使前端在不同 series 各自丢弃缺失值后，仍能区分未聚合的重复类别点和聚合后的
单一类别，并按请求语义重建统一 category slots；它们不是任意 ECharts 排序或聚合配置。
缺失值不会通过 `NaN` 或字符串伪装。bar/boxplot 的 `null` 仅表示某 series 在已确定 category 上没有值，ECharts 编译器显示为空，
不补零。预览的 `viewId/name` 为 `null`；保存图表读取时二者为非空。前端编译器只能使用该联合中的字段。

### 5.6 显示设置编译规则

`presentation` 是受控展示意图，不是 ECharts option。前端编译规则固定为：

- legend `show=false` 时不渲染；`auto` 在桌面为顶部横向、窄屏为底部横向；right 在桌面优先纵向，窄屏为避免溢出可降级到底部；
- orientation 非 auto 时在空间允许时遵循；窄屏可从 vertical 降级为 horizontal，但不得隐藏图例；
- title.show=false 时忽略 config title；true 且 title 缺失时不生成空标题占位；
- axis.showGrid 同时控制适用数值轴的 splitLine，不改变轴标签和数据；
- line.width/style/showSymbol 只映射线宽、线型和 symbol 显示，始终 `smooth=false`；
- theme、字体、tooltip、grid 边距、响应式高度和 aria 仍由前端统一实现，不进入持久配置。

调色板按顺序循环，颜色常量冻结为：

| palette | 颜色 |
|---|---|
| default | `#5470C6 #91CC75 #FAC858 #EE6666 #73C0DE #3BA272 #FC8452 #9A60B4 #EA7CCC` |
| colorblind | `#0072B2 #E69F00 #009E73 #D55E00 #CC79A7 #56B4E9 #F0E442 #000000` |
| highContrast | `#003F5C #BC5090 #FFA600 #2F4B7C #FF6361 #58508D` |
| monochrome | `#0F172A #334155 #475569 #64748B #94A3B8 #CBD5E1` |

系列超过调色板长度时循环颜色，同时由 line.style 或 ECharts 图形类型保持辨识；不自动生成未验证颜色。颜色映射按最终 series
稳定顺序确定，因此刷新后不漂移。

## 6. 后端处理

### 6.1 ChartCompiler

新增独立 application 边界 `application/charting/`，不调用 tensoreval，也不通过 Agent：

```text
ChartConfig
  → resolve source and ownership
  → parse physical schema
  → validate field bindings
  → scan narrow Parquet projection
  → extract scalar/List component values
  → group/aggregate/sort/sample
  → enforce post-transform budgets
  → produce PlotData + ChartModel
```

读取复用 `ParquetDataFrameReader::scan_projected` 和 artifact read gate，只投影配置引用的列。编译器不把服务器路径、Arrow
对象或任意表达式暴露给 API。

### 6.2 数据处理规则

以下规则是权威语义，不由前端或实现者另行猜测：

| 主题 | 规则 |
|---|---|
| 源顺序 | 每个源行取得从 0 开始的 `ordinal`；`order=source` 严格保持，只有 `xAsc/xDesc` 才稳定排序，相等时按 ordinal |
| 类别顺序 | source 模式按首次出现；xAsc/xDesc 对同一物理类型排序，字符串按 Unicode code point、布尔 false 在前、数值按数值 |
| series 名称 | measure 使用 trim 后 label 或规范 `column[path]`；带 splitBy 时为 `measure · category`；任意重名返回 `chart_series_name_conflict` |
| 轴/值名称 | 任意 ChartFieldRef 的展示名为 trim 后 label，否则为规范 `column[path]`；X/category/value 轴使用对应引用的展示名；单 measure 的 Y 轴使用 measure 展示名，多 measure 的 Y 轴 name=null |
| 缺失分量 | path 每次访问不存在都增加 `missingComponents`；同一候选点即使多个字段缺失，`droppedMissing` 只增加一次并丢弃该点 |
| 缺失/非有限 | X/Y/value 任一 null 使该候选点的 `droppedMissing` 增加一次；任一数值为 NaN/Inf 使 `droppedNonFinite` 增加一次；两者不进入聚合 |
| line 重复 X | aggregation=none 时允许并按顺序保留；选择聚合时按 `(series,x)` 聚合并以该组最小 ordinal 排序 |
| bar 重复键 | `(series,x)` 重复且 aggregation=none 返回 `chart_duplicate_requires_aggregation` |
| scatter 重复点 | 允许，保留各自行和 ordinal；不聚合、不采样 |
| heatmap 重复 cell | 必须按所选 reducer 聚合；`count` 统计 x、y 和 value 均非空的输入值数量 |
| 空结果 | 完整处理后没有任何点返回 `chart_no_data`，不返回空 ChartModel |
| boxplot | 按 category/splitBy 收集有限值，四分位数使用 R-7 线性插值；whisker 为落在 `Q1-1.5*IQR` 与 `Q3+1.5*IQR` 内的最远观测值；外侧值仅在 showOutliers=true 时返回 |
| histogram | min=max 时返回一个 `[min,max]` bin；否则等宽分箱，除最后一箱为闭区间外均为 `[start,end)`，`includesEnd` 只在最后一箱为 true |
| LTTB | 只允许 line，按 series 和当前排序后的点执行；不得跨 series；`sampledPoints` 记录删除的点数 |

`mean|min|max|sum` 只消费有限数值；空组不产生点。聚合和采样发生在最终输出预算检查之前，但编译器同时限制输入扫描量和
候选点数，防止用聚合请求绕过资源上限。

diagnostics 计数口径固定为：`inputRows` 是实际扫描源行数；`candidatePoints` 是 measure/List 展开后、缺失过滤前的候选点数；
`outputPoints` 是聚合和采样后的 ChartModel 点/cell/bin/box 数；`aggregatedGroups` 是输入值多于 1 的聚合键数量；
`sampledPoints` 是从聚合后、采样前的点集中被 LTTB 删除的点数。`missingComponents` 按字段访问计数，`droppedMissing` 按候选点
计数，因此两者可以同时增加且前者可能大于后者。

### 6.3 预算

缺省保持当前上限并补充输入上限：

- 原始扫描行数：1,000,000；
- 引用列数：32；
- measure 数：16；
- 最终 series：100；
- line/bar/scatter/boxplot 原始数值：500,000；
- 最终普通图点数：50,000；
- category：5,000；
- heatmap cell：100,000；
- ChartModel 序列化后：10 MiB；
- 图表名称：1..100 Unicode 字符；标题：0..200 Unicode 字符。

上限由命名配置集中定义，但属于公共行为；修改必须同步文档和行为测试。超限不静默截断，除非用户显式选择受控 sampling。

### 6.4 Plot Data 物化

保存时不复制源 Parquet，而是把编译后的窄 Plot Data 写成 Snappy Parquet：

- line：`ordinal,x_kind,x_category_json?,x_value?,series,value`；
- bar：`category_index,category_json,series,value?`；
- scatter：`ordinal,series,x,y`；
- heatmap：`x_index,x_category_json,y_index,y_category_json,value`；
- boxplot：`category_index,category_json,series,min,q1,median,q3,max,outliers`；
- histogram：`bin_index,bin_start,bin_end,count`。

Plot Data 是图表事实来源。`GET /data` 从该 artifact 读取并重建 `ChartModel`，不会再次读取源结果，因此来源删除不影响图表。
category 使用 `CategoryValue` 带标签联合的 canonical JSON 文本保存，读取时恢复同一联合，避免类型碰撞和大整数精度损失。保存时冻结
`chart-model-v1`、canonical config hash、源 schema/content hash、Plot Data schema/hash/content hash 和完整 diagnostics。

### 6.5 幂等和并发

- 预览只读，不写数据库或 CAS。
- “必须先预览”是前端交互门禁，不是 API 授权边界；保存 API 自己执行完整编译和校验，可信 API 客户端可以直接保存。
- 保存请求使用调用方 UUID `clientOperationId`。先规范化所有缺省值、trim name/title/label，再以
  `chart-model-v1 + normalized name + canonical ChartConfig` 计算 request hash；config hash 只包含 canonical ChartConfig。
- canonical config 对缺失 title/splitBy 保持字段省略，把缺失 label 展开为规范 `column[path]`，按 DTO 字段顺序序列化；
  `{mode:'none'}` 不允许携带 `maxPointsPerSeries`。因此持久 `config_json` 仍能直接反序列化为公开 `ChartConfig`，不需要内部 nullable
  变体。规范化后的完整配置才写入 `config_json`。
- 同一源、同一 operation ID、同一 request hash 的存活视图重放返回现有视图；hash 不同返回
  `chart_operation_conflict`。operation 指向已 tombstone 视图时返回 `chart_operation_gone`，调用方必须生成新 ID。
- repository 必须先检查 operation replay，再检查存活名称冲突；并发相同 operation 由唯一索引收敛后读取现有 receipt。
- name/title/label 去除首尾 Unicode whitespace；名称按 SQLite 精确 Unicode 字节、大小写敏感比较，删除后可复用；空名称或空 label
  拒绝，不做自动改名。
- 编译时持有 artifact read gate；发布 artifact 后，在 immediate transaction 中重新检查来源仍属于用户且 session 存活。
- 普通读取不存在、跨用户或已经 tombstone 的来源统一 404；只有初次所有权检查成功、随后发生并发删除时返回
  `chart_source_deleted`（409）。
- 名称冲突返回 `chart_name_conflict`，不覆盖已有图表。

Plot Data blob staging 可以先于事务，但 storage-scope refcount、artifact 业务引用和 `dataset_views` 插入必须在 artifact 发布锁下的
同一 immediate transaction 中完成。进程在 blob rename 后、数据库事务前崩溃只留下无数据库引用的 CAS 对象，由现有 orphan GC
回收，不能留下有 scope refcount 而无 view 的逻辑引用。图表删除和 session 清理也在同一事务中 tombstone/删除业务引用并逐条
`release_refs`；blob GC 在事务后执行。不得按 artifact ID 去重引用释放。

## 7. HTTP API

新增路由：

| Method | Path | 语义 |
|---|---|---|
| GET | `/api/datasets/{id}/chart-capabilities` | 返回字段形状、兼容角色和模板可用性 |
| POST | `/api/datasets/{id}/chart-previews` | 校验配置并返回临时 `ChartModel`，不持久化 |
| POST | `/api/datasets/{id}/views` | 物化并保存不可变图表；创建 201，幂等重放 200 |
| GET | `/api/dataset-views/{id}` | 图表元数据、冻结配置和来源快照 |
| GET | `/api/dataset-views/{id}/data` | 从 Plot Data artifact 返回 `ChartModel` |
| DELETE | `/api/dataset-views/{id}` | tombstone 图表并释放自己的 artifact |
| GET | `/api/sessions/{id}/dataset-views` | cursor 分页列出当前会话存活图表，包括来源删除状态 |

公共 DTO：

```ts
interface ChartPreviewRequest { config: ChartConfig }

interface ChartViewSummary {
  id: string
  sessionId: string
  sourceDatasetId: string
  sourceName: string
  sourceDeleted: boolean
  name: string
  kind: VisualizationKind
  pointCount: number
  sizeBytes: number
  createdAt: string
}

interface ChartViewMetadata extends ChartViewSummary {
  contractVersion: 'chart-model-v1'
  config: ChartConfig
  configHash: string
  sourceSchema: ResultColumn[]
  sourceSchemaHash: string
  sourceContentHash: string
  plotSchema: ResultColumn[]
  plotSchemaHash: string
  plotContentHash: string
  diagnostics: ChartModelBase['diagnostics']
}

interface CreateChartViewRequest {
  clientOperationId: string
  name: string
  config: ChartConfig
}

interface CreateChartViewResponse {
  view: ChartViewMetadata
  replayed: boolean
}

interface ChartViewPage {
  views: ChartViewSummary[]
  nextCursor: string | null
}

interface DeleteChartViewResponse {
  viewId: string
  tombstone: {
    name: string
    kind: VisualizationKind
    sourceDatasetId: string
    sourceName: string
    configHash: string
    plotContentHash: string
    deletedAt: string
  }
}
```

`POST chart-previews` 的 body 为 `ChartPreviewRequest`，成功直接返回 `ChartModel`。`POST views` 返回
`CreateChartViewResponse`；前端使用响应中的 metadata 更新列表，再按 view ID 请求 `/data`，不依赖源 metadata。
`GET dataset-views/{id}` 返回 `ChartViewMetadata`，`GET /data` 返回 viewId/name 非空的 `ChartModel`，DELETE 返回
`DeleteChartViewResponse`。

session 列表使用 `limit=1..100` 和绑定 session/user 的 opaque cursor，按 `created_at_ms DESC,id DESC` 排序；迟到页按 view ID
去重。普通不存在、跨用户和 tombstone 读取统一 404。

HTTP 状态固定为：

| 情况 | 状态 |
|---|---|
| preview 成功 | 200 |
| 首次保存 | 201 |
| operation 幂等重放 | 200，`replayed=true` |
| metadata/data/list 成功 | 200 |
| 删除成功 | 200 |
| 配置、列、分量、单位、重复键、无数据 | 400 |
| 名称、operation、operation gone、所有权检查后的来源删除竞争 | 409 |
| 任一输入/输出/响应预算或用户配额超限 | 413 |
| 不存在、跨用户、普通 tombstone | 404 |
| artifact/物化基础设施失败 | 500 |

保存请求：

```json
{
  "clientOperationId": "1f6d...",
  "name": "三相母线电压",
  "config": {
    "version": 1,
    "kind": "line",
    "presentation": {
      "legend": {"show": true, "position": "top", "orientation": "horizontal"},
      "palette": "colorblind",
      "title": {"show": true},
      "axis": {"showGrid": true},
      "line": {"width": 2, "style": "solid", "showSymbol": false}
    },
    "x": {"column": "bus"},
    "xAxisMode": "category",
    "measures": [
      {"column": "v_mag", "path": [0], "label": "A相"},
      {"column": "v_mag", "path": [1], "label": "B相"},
      {"column": "v_mag", "path": [2], "label": "C相"}
    ],
    "order": "source",
    "aggregation": "none",
    "sampling": {"mode": "none"}
  }
}
```

`DatasetMetadata.views` 保留轻量摘要，但改为 `{id,name,kind,createdAt}`，不在每次 dataset metadata 中返回完整配置。
配置由 `GET /api/dataset-views/{id}` 按需读取。

`ResultOutputRef.views` 与 `DatasetMetadata.views` 使用同一摘要 `{id,name,kind,createdAt}`；删除
`defaultViewId` 和 `views[].isDefault`。成功运行和结果消息仍不自动打开图表。

## 8. 数据库与生命周期

V4 只支持空库初始化，因此直接修改 initial migration，不增加旧库兼容迁移；开发环境需要重置 SQLite 和 DATA_DIR。

`dataset_views` 调整为：

```sql
CREATE TABLE dataset_views (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 source_dataset_id INTEGER NOT NULL REFERENCES result_datasets(id) ON DELETE RESTRICT,
 artifact_id INTEGER REFERENCES artifacts(id),
 client_operation_id TEXT NOT NULL,
 request_hash TEXT NOT NULL,
 name TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('line','bar','scatter','heatmap','boxplot','histogram')),
 config_json TEXT NOT NULL,
 config_hash TEXT NOT NULL,
 chart_contract_version TEXT NOT NULL,
 diagnostics_json TEXT NOT NULL,
 plot_schema_json TEXT NOT NULL,
 plot_schema_hash TEXT NOT NULL,
 plot_content_hash TEXT NOT NULL,
 row_count INTEGER NOT NULL CHECK(row_count>=0),
 point_count INTEGER NOT NULL CHECK(point_count>=0),
 size_bytes INTEGER NOT NULL CHECK(size_bytes>=0),
 source_name TEXT NOT NULL,
 source_schema_json TEXT NOT NULL,
 source_schema_hash TEXT NOT NULL,
 source_content_hash TEXT NOT NULL,
 created_at_ms INTEGER NOT NULL,
 deleted_at_ms INTEGER,
 tombstone_json TEXT,
 CHECK((deleted_at_ms IS NULL AND artifact_id IS NOT NULL AND tombstone_json IS NULL)
    OR (deleted_at_ms IS NOT NULL AND artifact_id IS NULL AND tombstone_json IS NOT NULL))
);
CREATE UNIQUE INDEX uq_dataset_views_operation
 ON dataset_views(source_dataset_id,client_operation_id);
CREATE UNIQUE INDEX uq_dataset_views_live_name
 ON dataset_views(source_dataset_id,name) WHERE deleted_at_ms IS NULL;
```

删除现有 `view_dataset_id` 和伪分析运行依赖。保存图表不再创建 `result_analysis_runs`、脚本 artifact 或
`result_datasets`。视图 lineage 由冻结的 `source_*` 字段表达；源行 tombstone 后仍可通过 source ID 和快照解释。

repository 插入时验证 `session_id` 等于源结果经 run/analysis 得到的同一 session；该冗余列用于来源 tombstone 后的分页、会话删除
统计和清理，不接受客户端值。`diagnostics_json` 必须按 `chart-model-v1` 反序列化后才可出网；非法存量视为内部存储错误。

图表删除写 tombstone 并释放 `artifact_id`；不删除来源结果。来源结果或运行删除不级联图表。会话删除统计和清理必须包含图表
数量、Plot Data 逻辑字节和逐引用 artifact release。会话 deletion preview/impact hash 新增 `chartCount` 和
`chartBytes`，确认时重新计算；GC 仍只清理零引用 CAS 对象。

## 9. 前端结构

新增或调整组件：

- `DatasetWorkbench`：管理数据表、构建器和已保存视图标签；
- `ChartBuilder`：模板、字段、分量、排序/聚合/采样、名称、标题和默认收起的显示设置；
- `ChartFieldSelect`：消费 capabilities，仅允许兼容字段；
- `ChartPreview`：展示 loading、诊断、过期和保存状态；
- `SavedChartPanel`：session 级 cursor 列表，保证来源删除后仍可发现；
- `ChartRenderer`：改为消费 `ChartModel`；
- `charts/*`：从 `ChartModel` 编译 ECharts option，不再读取源 Dataset schema/spec/rows。

状态隔离：

- 数据集、配置或 session 变化时增加 request generation，迟到的 capabilities/preview/data 不得覆盖当前状态；
- preview 使用 `AbortController`，只有最新显式请求可更新；
- 保存成功后刷新 metadata 和 session 图表列表，并直接打开新视图；
- 删除成功后关闭当前视图、移除标签并刷新列表；
- 401 继续走统一登录失效流程；
- 窄屏构建器采用纵向步骤，不产生页面横向溢出。

ECharts 编译器固定关闭动画，按受控 presentation 选择 palette、legend、splitLine 和 line 样式，并统一 tooltip、grid、resize、
dispose 与 aria。后端只校验和回传显示意图，不生成 option；像素尺寸、字体、窄屏降级和具体响应式布局仍由前端控制，任何
formatter 都不开放。

## 10. Agent、Prompt 与工具变化

- 从 Agent manifest、工具注册表和合同清单移除 `create_dataset_view`。
- 从 `run_result_analysis` 工具参数删除 `initialViews`，从 `AnalysisRequest` 删除 `initial_views`，删除分析成功后的自动
  `create_view` 循环、`AnalysisService::create_view/create_resource_view` 和旧工具模块。比较服务不再构造空
  `initial_views`。
- `result-analysis` skill 只负责筛选、聚合、关联和生成派生 DataFrame；不再声明创建图表。system prompt、comparison skill
  和工具说明不得保留旧建图入口。
- 用户在对话中要求“画图”时，Agent 说明应在结果工作台点击“新建图表”；如果用户同时明确要求生成新的分析数据，仍可先调用
  `run_result_analysis`，但不得自动创建视图。
- `list_result_datasets` 仍返回数据集和 schema，但不再需要为 Agent 返回完整 view spec。
- Agent bundle 版本、manifest hash、工具总数、skill requiredTools、功能目录和发布测试同步更新。
- 重写当前依赖 `initialViews/create_dataset_view` 的 resilience、data resource、result deletion、tool manifest 和
  `v3-workflows.spec.ts` 证据；不允许旧路径因未被调用而继续编译存在。

该变化消除 LLM 到图表配置的写权限，但不改变模型修改、研究准备、受管执行、分析或比较的现有授权边界。

## 11. 稳定错误码

新增或收敛以下错误：

- `chart_config_invalid`：联合类型、未知字段或参数范围错误；
- `chart_capabilities_too_large`：列数、路径数或 capability 响应超过预算；
- `chart_column_missing`：字段不存在；
- `chart_column_incompatible`：字段物理类型不适合槽位；
- `chart_component_invalid`：List 路径非法或所有行都不存在；
- `chart_unit_conflict`：多个 measure 单位冲突；
- `chart_series_name_conflict`：规范化后的 series 展示名重复；
- `chart_duplicate_requires_aggregation`：固定模板要求唯一但存在重复键；
- `chart_no_data`：完整过滤后没有可展示点；
- `chart_too_large`：输入、series、category、cell、点数或响应超过预算；
- `chart_name_conflict`：同一源结果的存活图表重名；
- `chart_operation_conflict`：幂等 ID 重用且请求 hash 不同；
- `chart_operation_gone`：幂等 ID 指向已经删除的图表；
- `chart_source_deleted`：预览或保存已完成首次用户/存活校验后，来源在发布前发生并发 tombstone；普通 tombstone 请求仍统一 404；
- `chart_materialization_failed`：Plot Data 构建或 CAS 发布失败；
- `chart_not_found`：只在内部映射，公共响应仍为统一 404。

错误 `details` 只返回字段名、槽位、观察类型、估算量和公共限制，不返回路径、SQL、Arrow debug 数据或完整源行。

## 12. 安全与隐私

- API 只接受稳定 dataset ID 和强类型配置；所有权查询显式接收 `UserId`。
- 配置禁止未知字段，字符串长度和数组数量有界。
- 图例和标题作为纯文本进入 ECharts，不使用 HTML/formatter。
- ChartModel 不包含服务器路径、artifact ID、脚本、命令或任意代码。
- Plot Data artifact 属于认证用户 storage scope；跨用户 view/source ID 统一 404。
- capability 抽样、preview 和 save 都使用窄投影和扫描预算，防止通过图表接口读取任意未选列或制造无界内存。

## 13. 验证与功能追溯

### 13.1 Rust

- `ChartConfig` 每个 variant 的 serde、未知字段、长度和范围；
- presentation 枚举、line 专用字段、canonical hash 和非法样式拒绝；
- dtype/slot 兼容矩阵；
- 标量与嵌套 List 路径提取、变长 List、null、NaN/Inf 和全缺失；
- 多 measure、splitBy、重复键聚合、排序、分箱、boxplot 分位数和 LTTB；
- 输入和输出预算边界；
- preview 无写入；save 幂等、名称冲突、来源删除竞争和失败 artifact release；
- API 直接 save（未 preview）仍完整验证；operation gone、并发同 operation 和 replay-before-name-conflict；
- Plot Data artifact 引用与 view 插入/删除的单事务结算，以及 blob rename 后崩溃 orphan GC；
- repository 用户隔离、session 归属、source tombstone 后读取、view tombstone 和 session 清理引用；
- Axum 所有新增路由的 2xx、400/409/413、401 和跨用户 404。

### 13.2 Vitest

- capability 到字段选项的确定性映射；
- 每种模板只显示合法槽位和字段；
- `v_mag[0..2]` 生成三条系列请求；
- 配置变化使 preview 过期，未重新预览不能保存；
- 请求代次与 AbortController 隔离；
- 六类 `ChartModel -> EChartsOption` golden tests；
- 四套 palette、legend 位置/方向、标题、网格线、line width/style/symbol 和窄屏 legend 降级；
- diagnostics、错误原因、窄屏步骤和无障碍名称；
- 保存、复制、删除和来源已删除卡片。
- “图表”第四标签从 view ID 打开独立工作台，不请求 tombstone source metadata。

### 13.3 Playwright

确定性旅程至少覆盖：

1. 运行生成包含 `bus: Utf8View`、`v_mag: LargeList(Float64)` 的结果；
2. 打开指定非首个结果，默认显示数据表；
3. 点击“新建图表”，选择 line、bus 和 `v_mag[0..2]`，填写三相标签；
4. 修改图例位置、调色板和折线样式，预览出现三条系列并反映设置，保存后新增图表标签；
5. 刷新页面后图表和配置恢复；
6. 删除源结果后，从“已保存图表”仍能打开同一图表并看到来源已删除提示；
7. 删除图表后入口消失且 artifact 配额释放；
8. 第二用户对 capability、preview、save、metadata、data、delete 全部得到 404；
9. 390px viewport 无横向溢出并通过 axe serious/critical 门禁。

另用 API/Rust 行为测试覆盖六种图形、预算和所有稳定错误；浏览器旅程不承担枚举式穷举。

### 13.4 合同与文档

- 更新 `docs/development-status.md`、`docs/api-design.md`、`docs/db-design.md`、`docs/frontend-design.md`、
  `docs/agent-prompt-skills-design.md` 和 `docs/integration-testing-design.md`；
- 更新 `frontend/src/types/` 与后端 contract manifest；
- 更新 `integration-tests/catalog/features.yaml`，移除 `tool:create_dataset_view`，加入新增路由和错误；
- 七条新增/变更 view 路由各自绑定真实 Rust 行为和 Playwright 2xx/3xx 路由证据；
- 保持六种 `view:*` 公共枚举，但行为证据改为用户配置式创建；
- `./scripts/check-functional-coverage --require-pr` 必须继续为 100%。

## 14. 交付顺序

1. 冻结 `ChartConfig`、`ChartCapabilities`、`ChartModel` 与错误码合同。
2. 建立 chart compiler 的纯函数测试和 Parquet 窄扫描实现。
3. 重构 `dataset_views` 持久化，移除伪分析运行和整表复制。
4. 增加 capability、preview、save、metadata、data、delete、session list API。
5. 前端改为 `ChartModel` 编译器，并实现构建器、预览和保存图表列表。
6. 移除 Agent `create_dataset_view` 权限并更新 skill/manifest。
7. 补齐 Rust、Vitest、Playwright、功能目录和当前设计文档。
8. 执行 fmt、Clippy、后端全量测试、Vitest、生产构建、Playwright 和 PR 功能追溯。

## 15. 验收标准

- 用户无需发送对话消息即可从仿真结果创建、预览、保存和恢复图表。
- `bus + v_mag[0..2]` 能稳定生成三相折线，配置中不存在任意 ECharts JSON。
- 图例、系列名称、固定调色板、标题、网格线和折线样式可以预览、保存并在刷新后恢复，窄屏不会因右侧图例溢出。
- 后端只返回 `ChartModel`，前端编译器不读取源 DataFrame 的 List 结构。
- 保存图表物化的是窄 Plot Data，不复制完整源结果，也不创建虚假分析运行或脚本 artifact。
- 所有不兼容组合在保存前给出稳定、可操作的诊断。
- 来源删除后保存图表继续可读并可从 session 图表列表发现。
- Agent 工具面不再包含 `create_dataset_view`，重复的错误工具调用路径从产品中消失。
- 新路由、DTO、数据库约束、错误码、前端行为和删除语义全部进入功能目录与自动化行为测试。
