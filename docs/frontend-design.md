# V3 前端设计

## Layout

桌面为历史侧栏、对话区、结果面板三列；窄屏把两侧改为可键盘访问的 drawer，并禁止横向溢出。左下角显示
固定用户名和退出入口。用户切换后清空会话业务状态，localStorage 只保存当前用户的最后会话 ID。

## Upload routing

Composer 固定按钮支持文件和目录：`.rss` 逐个创建脚本；DSS/bundle/ZIP/CSV/Excel/Parquet 合并为一次
模型导入；其他扩展名在客户端标记失败。导入卡展示阶段、候选、整表目标与诊断，用户用“确认合并”或“抛弃”
完成决策。窄屏出现确认项时工作区 drawer 可显式打开。

## Workspace

模型工作区显示当前 active、可变 draft、未决导入和不可变版本历史。只有非空 draft 显示“生成版本”；点击后
发布并自动激活。激活历史版本遇到 draft 时使用明确确认框放弃草稿。版本删除先读取影响预览，再提交一次性
impact token。前端不把这些固定动作伪装成聊天消息。

## Example library

“案例库”是高频固定入口。界面消费后端 rssbundle 目录树：顶层分类默认展开，嵌套目录可用带
`aria-expanded` 的按钮展开；搜索由服务端匹配 manifest 元数据并自动展开返回叶节点的祖先。损坏案例仍显示验证
状态与诊断，但没有“使用此案例”操作。

桌面使用目录/详情双栏；窄屏先显示目录，选择叶节点后切换详情，并提供明确的“返回案例目录”。详情展示 bundle
相对路径、来源和脚本入口、study、solver、compatibility、results、generator、文件角色计数及诊断。加载、空结果、
请求失败与重试都有独立状态；切换搜索或案例时取消旧响应的状态写入。有效案例的固定按钮继续调用
`onUse(id,title,engineResourceSnapshot)`，进入现有候选 DataFrame 确认流程。

## Scripts and runs

脚本消息使用代码块，默认只显示有界开头；“展开全文”从详情 API 懒加载，“收起”恢复预览，“复制”复制全文。
脚本创建后绝不自动运行；有 active 且静态/动态兼容时才启用“运行”。运行记录按 session 展示冻结的模型与
脚本 ID，无运行删除按钮。

## Results

结果面板按运行读取数据集。DatasetWorkbench 使用 cursor 分页、服务端列投影和 Parquet 下载；单个结果可以
删除，删除后关闭该数据集但保留运行和兄弟结果。图表只消费服务端验证的六种 spec，不执行 formatter 或任意
ECharts option。比较和物化视图通过稳定 ID 打开。数据分页将 Arrow 的 `Utf8`、`LargeUtf8` 与 `Utf8View`
统一输出为 JSON 字符串；成功运行的进度卡必须把包括“收集结果”在内的全部阶段标记为完成。桌面数据集详情
在遮罩层内水平居中，内容宽度不同不得改变对齐方式；窄屏继续占满可用宽度。

## State and recovery

消息 state 以 session 分区，先建立 SSE 再读快照，按稳定 message ID 去重。临时 user 消息在 POST 成功后
替换成服务端 ID，并使用响应中的权威 `sessionTitle` 同步更新侧栏和页头。用户没有任何会话时，首次进入自动创建并选中一个标题为“新会话”的空会话；该默认标题仍由首条用户消息摘要自动替换。删除会话必须先用弹框说明聊天、模型、脚本、运行和结果会被不可恢复地删除，并由用户显式确认；删除最后一个会话后建立同样的替代会话，保证发送、上传和案例应用都有明确归属。
会话准备期间不渲染上述主操作；初始化失败显示持久的重试和退出入口，不保留可点击的无效控件。
workspace、imports、scripts、runs 和 usage 独立加载；切换 session 时清理导入队列，避免旧会话卡片泄漏。401 统一进入登录页，
SSE resync 触发有界重载。

## Accessibility

交互元素使用原生 button/input、可辨别名称和 focus 顺序；modal/drawer 有遮罩关闭控件。桌面和 390px
viewport 都由 Playwright + axe 检查 serious/critical violations。
