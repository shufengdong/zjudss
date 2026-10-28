# 前端设计（V4）

> 右侧工作台使用 V4 统一上传和独立数据资源协议；对话区已经接入研究确认卡、受管执行进度以及运行到
> 历史脚本的精确定位。独立资源需求见
> [`2026-10-26-independent-data-resources-design.md`](superpowers/specs/2026-10-26-independent-data-resources-design.md)。

## Layout

桌面为历史侧栏、对话区、结果面板三列；窄屏把两侧改为可键盘访问的 drawer，并禁止横向溢出。左下角显示
固定用户名和退出入口。用户切换后清空会话业务状态，localStorage 只保存当前用户的最后会话 ID。

登录卡使用 SGOOL 官网 `images/biglogo2.png` 的 1024×1024 圆形原图作为品牌标识；标识与“配电网仿真智能体”
标题在同一横行居中显示，窄屏缩小字号和标识但不换行。图片保存在前端静态资源中，页面不依赖外站运行时可用性。

## Upload routing

Composer 固定按钮支持文件和目录：`.rss` 逐个创建脚本；DSS/bundle/ZIP/CSV/Excel/Parquet 合并为一次
统一 upload batch；其他扩展名在客户端标记失败。批次卡展示模型和数据两个子事务状态、候选、sheet、用途与诊断。
用户显式选择 `model_table|data_resource|skip` 后发布；全选排除隐藏 sheet，部分成功时分别显示两个子事务状态，
只重试失败的一侧。批次稳定 ID 来自 durable `upload-batch` 消息，刷新后逐项恢复，切换会话立即清空旧队列。
对话中的 durable 批次消息复用同一套解析设置、用途、sheet 全选/取消全选和确认控件；批次尚未恢复时保留
“查看并确认”入口，打开右侧数据文件页继续处理。

## Managed study configuration

Agent 只负责根据自然语言建立 revisioned 研究配置。配置卡按服务端状态依次显示目录门面及其方法差异、模型输入、
Data 资源绑定、Struct 参数、受控列映射和结果集合。唯一匹配资源可以预选但仍由用户确认；同会话同类研究可以显示
上次资源复用建议。Struct 字段直接使用 snapshot 目录的 bool/number/enum/string/date、单位、默认值和范围合同，`para`
与其他 Struct 一样引用普通数据资源；修改已有资源时必须选择覆盖当前资源或另存为新资源。
参数卡从服务端 `baselineValues` 恢复资源基线；刷新后仍显示尚未确认的字段变化和覆盖/另存选择，不把当前草稿值误作
新的资源基线。

所有 mutation 携带当前 `expectedRevision`。服务端返回 `configuration_stale` 时刷新整张卡并要求用户基于新 revision
重试，浏览器不合并旧输入。只有状态达到 `ready_to_confirm` 才显示完整摘要和“确认并生成脚本”；自然语言回复不能
代替该按钮。摘要列出实际函数、snapshot、门面所需模型表或“无需模型”、所有 Data/Struct 资源和结果集合。

确认后对话显示受管工作流的服务端执行/修正计数、动态耗时和取消按钮。attempt 默认折叠，展开只显示脚本 hash、
退出码和脱敏 stderr。成功只列结果数据集，不自动打开表格或图；失败、用户取消和服务重启使用“重新尝试”，先读取
输入差异预览，变化时再次确认。成功生成的脚本卡仍使用“运行”，每次点击按最新资源重新运行且不进入自动修正循环。

## Workspace

模型工作区显示当前 active、可变 draft 和不可变版本历史；未决统一上传集中显示在“数据文件”页。只有非空 draft
显示“生成版本”，并提供最多 200 个 Unicode 字符的可选单行描述。发布后使用服务端响应中的冻结描述刷新列表；
内容去重时明确显示复用的已有版本及其原描述。失败保留输入，成功或切换会话清空输入。激活历史版本遇到 draft
时使用明确确认框放弃草稿。版本删除先读取影响预览，再提交一次性 impact token。

## Data resources

右侧“数据文件”页列出当前会话的普通 DataFrame 资源，显示 display name、稳定 alias、schemaRef（若有）、来源、
行列数、大小、状态和创建信息。`para` 与负荷曲线等资源使用同一列表和生命周期，不进入模型版本。列表用 opaque
cursor 分页且按稳定资源 ID 去重；迟到的旧会话响应不能覆盖当前页面。

“查看”按稳定资源 ID 打开 cursor 分页表格；规范文件始终可下载，只有 `originalAvailable=true` 时显示原文件入口。
删除先读取 impact preview，再在命名确认框中展示未确认配置、冻结引用和历史引用，确认后才携 token 删除。
“重新上传”建立 replacement batch，旧资源在处理期间仍可用；差异确认提供“覆盖当前资源”“另存为新资源”与
“取消替换”三个动作，终态后刷新 alias owner。当前 snapshot 明确给出 Struct schema 且资源为单行时显示
“修改参数”，普通表不显示该按钮；实际参数卡由研究确认界面承载。

当前 active 和可变 draft 默认各自收起，摘要分别显示版本或修订号、表格数量和总行数；两个区块独立展开，
同一会话内 workspace 刷新保留用户选择，切换会话时恢复默认收起。不存在 active 或 draft 时继续显示原空状态，
不提供无意义的展开按钮。

active、draft 和展开后的历史版本按表显示“查看”按钮。按钮以 `OpenDataFrame` 判别联合打开只读
`ModelDataFrameWorkbench`：版本源保存 `dataframeId + versionId + tableName`，草稿源额外保存
`draftId + revision`。工作台显示元数据、列选择、cursor 分页、“加载更多”和 Parquet 下载，不出现结果删除、
图表、分析或物化视图操作。历史表无需激活即可查看，打开表和下载都不改变 active、draft 或运行记录。

草稿 revision 冲突时保留已显示的旧行，刷新 workspace，并按相同表名定位新草稿表；仍存在时用户可重新加载，
已消失时关闭工作台并显示说明。切换会话、数据源或列投影会建立新的请求代次，迟到响应不能覆盖当前表。

## Example library

“案例库”是高频固定入口。界面消费后端 rssbundle 目录树：顶层分类默认展开，嵌套目录可用带
`aria-expanded` 的按钮展开；搜索由服务端匹配 manifest 元数据并自动展开返回叶节点的祖先。损坏案例仍显示验证
状态与诊断，但没有“使用此案例”操作。

桌面使用目录/详情双栏；窄屏先显示目录，选择叶节点后切换详情，并提供明确的“返回案例目录”。详情展示 bundle
相对路径、来源和脚本入口、study、solver、compatibility、results、generator、文件角色计数及诊断。加载、空结果、
请求失败与重试都有独立状态；切换搜索或案例时取消旧响应的状态写入。有效案例的固定按钮继续调用
`onUse(id,title,engineResourceSnapshot)`，进入现有候选 DataFrame 确认流程。

## Scripts and runs

“新建脚本”弹窗使用宽版等宽字体编辑区，默认占约半个视口高度，并允许从右下角纵向拖动调整；最小高度保证
脚本可读，最大高度受当前视口限制，窄屏时弹窗自身可滚动，操作按钮始终可达。

脚本消息使用代码块，默认只显示有界开头；“展开全文”从详情 API 懒加载，“收起”恢复预览，“复制”复制全文。受管工作流
终态事务写入的紧凑脚本消息若只含稳定 ID、名称和 hash，脚本卡会自动读取详情补齐 `canRun`、preview 和正文状态，避免
刷新后“运行”永久禁用。工作流卡首次进入终态时刷新当前消息窗口与运行列表，使同一事务发布的脚本和结果立即可见。
脚本创建后绝不自动运行；“运行”不提交模型 ID，后端在点击时解析最新资源，因此没有 active 的资源型脚本仍可提交。
成功 Agent 脚本显示“运行”，失败受管脚本的派生流程显示“重新尝试”；输入发生变化时必须先展示差异并由用户确认。
运行记录按 session 展示冻结的模型与脚本信息。终态记录提供删除动作并先展示结果数、字节数和派生/提升引用影响；
非终态不提供可执行删除。每条运行默认收起；摘要用冻结的 study-plan function 显示五类中文研究标题，缺少合法审计信息时
回退到不可变 `scriptName`，同时显示中文状态、脚本名、模型版本、创建时间和耗时；展开后明确列出创建、开始与结束时间，
缺失或无效值显示 `—`。终态耗时取冻结时间戳，排队/运行中的
耗时由运行记录内唯一的一秒计时器刷新，不用计时器轮询后端状态。展开的失败记录显示中文阶段、有界公共错误码/原因、退出码和
耗时；存在 `error.engineOutput` 时，在限高、可滚动、保留换行的等宽区域显示经服务端隐藏路径后的 tensoreval 报错。展开的
排队/运行记录显示当前阶段且不请求结果。
运行记录主区域在存在 `scriptMessageId` 时定位对应的历史脚本消息，详情由独立按钮展开，二者不会形成嵌套按钮；缺少
锚点时明确显示脚本尚未生成。目标不在当前页时先用 `loadAround` 进入 anchored history window，窄屏先收起结果面板，
然后聚焦、滚动并短暂高亮准确消息；锚定窗口始终提供“返回最新消息”。删除运行必须先读取影响 token 并在命名 dialog
确认，成功后移除运行、失效结果缓存并关闭被删除结果的工作台，冲突或影响变化保留原记录并显示服务端诊断。

## Results

成功运行只有在展开时才读取当前全部存活数据集，并逐项显示名称、行数和大小；用户点击指定数据集的“查看”后直接按稳定 ID
打开它，不再固定打开第一个结果。运行列表和单次结果加载分别展示 loading、错误与重试；迟到响应按请求代次丢弃。展开状态在
同一 session 的运行列表刷新中保留，切换 session 时清空。DatasetWorkbench 使用 cursor 分页、服务端列投影和 Parquet 下载；单个结果可以
删除，删除成功后提升 `datasetRevision`，失效所有已缓存结果并使当前展开的成功记录立即重新读取；当时收起的记录在下次展开时
重新读取，因此不会恢复已删除入口，同时保留运行和兄弟结果。图表只消费服务端验证的六种 spec，不执行 formatter 或任意
ECharts option。比较和物化视图通过稳定 ID 打开。数据分页将 Arrow 的 `Utf8`、`LargeUtf8` 与 `Utf8View`
统一输出为 JSON 字符串，将 `List`、`LargeList`、ListView 和定长 List 递归输出为 JSON 数组；成功运行的
进度卡必须把包括“收集结果”在内的全部阶段标记为完成。桌面数据集详情
在遮罩层内水平居中，内容宽度不同不得改变对齐方式；窄屏继续占满可用宽度。

## State and recovery

消息 state 以 session 分区，先建立 SSE 再读快照，按稳定 message ID 和 revision 去重；同一操作卡的新 revision 原位替换旧卡。
临时 user 消息在 POST 成功后替换成服务端 ID，并使用响应中的权威 `sessionTitle` 同步更新侧栏和页头。历史搜索命中远端消息时，
`loadAround(messageId)` 以服务端返回的连续 context page 建立独立 anchored window，不与可能有缺口的 latest page 拼接。
锚定期间 SSE 继续更新隐藏的 latest cache 和游标，界面显示“返回最新消息（有新消息）”；`returnLatest` 恢复已缓存最新窗口，
必要时重新加载最新页。

用户没有任何会话时，首次进入自动创建并选中一个标题为“新会话”的空会话；该默认标题仍由首条用户消息摘要自动替换。
删除会话第一次点击只读取影响预览，命名 dialog 显示模型、数据资源、脚本、运行、结果、分析 artifact 和逻辑空间；取消不发送
DELETE，确认时携带预览 token，提交期间禁用重复操作。影响已变化时刷新预览后由用户再次确认。删除最后一个会话后建立同样的
替代会话，保证发送、上传和案例应用都有明确归属。
会话准备期间不渲染上述主操作；初始化失败显示持久的重试和退出入口，不保留可点击的无效控件。
workspace、upload batches、data resources、scripts、runs 和 usage 独立加载；切换 session 时清理上传队列和资源页，避免旧会话卡片泄漏。401 统一进入登录页，
SSE resync 触发有界重载。

## Accessibility

交互元素使用原生 button/input、可辨别名称和 focus 顺序；modal/drawer 有遮罩关闭控件。桌面和 390px
viewport 都由 Playwright + axe 检查 serious/critical violations。模型版本历史展开按钮以可见版本名称作为
accessible name，并通过 `aria-expanded` 暴露状态；窄屏模型表工作台不得造成页面横向溢出。
