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

## Scripts and runs

脚本消息使用代码块，默认只显示有界开头；“展开全文”从详情 API 懒加载，“收起”恢复预览，“复制”复制全文。
脚本创建后绝不自动运行；有 active 且静态/动态兼容时才启用“运行”。运行记录按 session 展示冻结的模型与
脚本 ID，无运行删除按钮。

## Results

结果面板按运行读取数据集。DatasetWorkbench 使用 cursor 分页、服务端列投影和 Parquet 下载；单个结果可以
删除，删除后关闭该数据集但保留运行和兄弟结果。图表只消费服务端验证的六种 spec，不执行 formatter 或任意
ECharts option。比较和物化视图通过稳定 ID 打开。

## State and recovery

消息 state 以 session 分区，先建立 SSE 再读快照，按稳定 message ID 去重。临时 user 消息在 POST 成功后
替换成服务端 ID。用户没有任何会话时，首次进入自动创建并选中一个空会话；删除最后一个会话后也建立替代会话，保证发送、上传和案例应用都有明确归属。
会话准备期间不渲染上述主操作；初始化失败显示持久的重试和退出入口，不保留可点击的无效控件。
workspace、imports、scripts、runs 和 usage 独立加载；切换 session 时清理导入队列，避免旧会话卡片泄漏。401 统一进入登录页，
SSE resync 触发有界重载。

## Accessibility

交互元素使用原生 button/input、可辨别名称和 focus 顺序；modal/drawer 有遮罩关闭控件。桌面和 390px
viewport 都由 Playwright + axe 检查 serious/critical violations。
