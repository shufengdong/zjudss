# 前端设计（V4 迁移中）

> 以下界面仍是进入 V4 前的 V3 基线。后端 Task 4 已切换到统一 upload batch 协议，现有前端导入卡尚未迁移，
> 由 Task 12 一次完成统一上传交互和数据文件工作台；在 Task 12 完成前不把本节描述当作当前可联调合同。
> V4 将在右侧工作台新增持久的“数据文件”入口，并增加研究确认卡、运行删除及运行到历史脚本
> 的精确定位，用于负荷预测等不参与模型版本管理的 DataFrame；需求见
> [`2026-10-26-independent-data-resources-design.md`](superpowers/specs/2026-10-26-independent-data-resources-design.md)。

## Layout

桌面为历史侧栏、对话区、结果面板三列；窄屏把两侧改为可键盘访问的 drawer，并禁止横向溢出。左下角显示
固定用户名和退出入口。用户切换后清空会话业务状态，localStorage 只保存当前用户的最后会话 ID。

登录卡使用 SGOOL 官网 `images/biglogo2.png` 的 1024×1024 圆形原图作为品牌标识；标识与“配电网仿真智能体”
标题在同一横行居中显示，窄屏缩小字号和标识但不换行。图片保存在前端静态资源中，页面不依赖外站运行时可用性。

## Upload routing

Composer 固定按钮支持文件和目录：`.rss` 逐个创建脚本；DSS/bundle/ZIP/CSV/Excel/Parquet 合并为一次
模型导入；其他扩展名在客户端标记失败。导入卡展示阶段、候选、整表目标与诊断，用户用“确认合并”或“抛弃”
完成决策。窄屏出现确认项时工作区 drawer 可显式打开。

## Workspace

模型工作区显示当前 active、可变 draft、未决导入和不可变版本历史。只有非空 draft 显示“生成版本”；点击后
发布并自动激活。激活历史版本遇到 draft 时使用明确确认框放弃草稿。版本删除先读取影响预览，再提交一次性
impact token。前端不把这些固定动作伪装成聊天消息。

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

脚本消息使用代码块，默认只显示有界开头；“展开全文”从详情 API 懒加载，“收起”恢复预览，“复制”复制全文。
脚本创建后绝不自动运行；“运行”不提交模型 ID，后端在点击时解析最新资源，因此没有 active 的资源型脚本仍可提交。
成功 Agent 脚本显示“运行”，失败受管脚本的派生流程显示“重新尝试”；输入发生变化时必须先展示差异并由用户确认。
运行记录按 session 展示冻结的模型与脚本信息。终态记录提供删除动作并先展示结果数、字节数和派生/提升引用影响；
非终态不提供可执行删除。每条运行默认收起；摘要用冻结的 study-plan function 显示五类中文研究标题，缺少合法审计信息时
回退到不可变 `scriptName`，同时显示中文状态、脚本名、模型版本、创建时间和耗时；展开后明确列出创建、开始与结束时间，
缺失或无效值显示 `—`。终态耗时取冻结时间戳，排队/运行中的
耗时由运行记录内唯一的一秒计时器刷新，不用计时器轮询后端状态。展开的失败记录显示中文阶段、有界公共错误码/原因、退出码和
耗时；存在 `error.engineOutput` 时，在限高、可滚动、保留换行的等宽区域显示经服务端隐藏路径后的 tensoreval 报错。展开的
排队/运行记录显示当前阶段且不请求结果。

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
workspace、imports、scripts、runs 和 usage 独立加载；切换 session 时清理导入队列，避免旧会话卡片泄漏。401 统一进入登录页，
SSE resync 触发有界重载。

## Accessibility

交互元素使用原生 button/input、可辨别名称和 focus 顺序；modal/drawer 有遮罩关闭控件。桌面和 390px
viewport 都由 Playwright + axe 检查 serious/critical violations。模型版本历史展开按钮以可见版本名称作为
accessible name，并通过 `aria-expanded` 暴露状态；窄屏模型表工作台不得造成页面横向溢出。
