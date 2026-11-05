# API、前端状态与界面整合设计

**日期：** 2026-11-05  
**范围：** 仓库维护路线 8.8  
**状态：** 已批准（用户授权连续完成 8.7–8.9）

## 1. 目标

在领域接口已经稳定的前提下，收敛完全相同的下载响应、请求代次和 impact confirmation 交互；删除无消费者的 hook 与只转发别名。保持现有 HTTP/SSE、DTO、稳定错误码、固定按钮/自然语言权限边界、390px 布局和键盘行为。

## 2. 审计结论

- dataset、data resource 与 model DataFrame 下载 handler 分别读取文件、设置 `Content-Type`/`Content-Disposition` 并构造 body，语义完全相同。
- 图表、比较、案例、工作流和数据表 hooks 各自维护 generation/AbortController。复杂轮询和分页语义不同，不能合并成万能请求 hook；但“开始新代次、取消上一请求、只接受当前响应、卸载失效”是同一原语。
- `useComparison` 没有 generation 或 abort，切换 comparison ID 后迟到响应会覆盖当前视图；错误界面也没有重试入口。
- `useAnalysisRun` 没有消费者，`useDatasetView` 只是 `useChartView` 的重导出，形成无意义层。
- 会话、运行、数据资源与版本删除都先读取 impact token 再确认，但 dialog 的 Escape、focus、backdrop、loading、error 和重试行为不一致；版本删除仍使用不可访问且无法展示结构化影响的 `window.confirm`。
- cursor 分页的资源和图表各有领域特有状态；本阶段只共享稳定 ID 合并函数和请求 gate，不合并分页 hook。

## 3. 方案比较

### 方案 A：引入 React Query 等全局异步状态框架

能统一缓存、重试和请求取消，但会改变整个仓库的状态所有权、测试方式和依赖面，且消息/SSE、轮询、cursor 分页并不共享同一缓存语义。不采用。

### 方案 B：建立万能 `useAsyncResource`/`usePagination`/`useMutation` 抽象

表面减少行数，但需要大量开关表达“保留旧数据、后台轮询、projection、stale conflict、分页追加、并发 mutation”，会把领域差异重新塞进条件分支。不采用。

### 方案 C：只共享相同生命周期原语

提取后端 attachment response、前端 request generation gate、稳定 ID merge 和可访问 confirmation dialog shell。具体数据、错误文案、轮询、分页、impact 内容和成功副作用仍由领域 hook/组件拥有。选用此方案。

## 4. 设计

### 4.1 后端下载响应

新增 API 内部 helper，接收已经完成用户作用域校验的 `std::fs::File`、媒体类型和文件名，统一有界地读取并构造 attachment response。dataset、data resource、model DataFrame handler 仍各自解析稳定 ID、查询领域 service 和决定文件种类，只把最终 HTTP 映射交给 helper。

不改变路由、响应 header、body 或错误码。

### 4.2 请求代次 gate

新增前端 hook 原语，仅负责：

- `begin()` 中止上一 controller、递增 generation 并返回当前 signal/token；
- `isCurrent(token)` 判断响应是否仍属于当前组件/资源；
- `invalidate()` 在卸载或身份变化时中止并失效。

它不存放领域 data/loading/error。优先用于 `useComparison`、`useChartView` 和数据资源工作台；复杂的 model DataFrame、轮询与 cursor hooks保留自己的领域状态，但可在完全等价时复用 gate。

`useComparison` 返回 `{value, loading, error, reload}`，ID 切换立即清空旧值，迟到响应不写入，失败可重试。ComparisonView 显示独立 loading/error/retry。删除无消费者的 `useAnalysisRun`，DatasetWorkbench 直接导入 `useChartView` 并删除转发 hook。

### 4.3 分页状态

建立纯 `mergeByStableId`，资源与已保存图表分页用稳定 ID 覆盖旧项并保持首次顺序。请求代次、cursor 和 in-flight gate 仍留在各自 hook；初始 reload 必须使旧分页响应失效，失败保留已经加载的行/卡片并允许重试。

DataResourceWorkbench 的初始读取和 `loadMore` 共用同一 request gate，关闭或切换资源会取消分页请求，迟到页不能写回。

### 4.4 Impact confirmation

新增共享 `ImpactConfirmationDialog` 外壳，统一：

- 原生 dialog 语义、标题/描述关联、初始焦点与 Tab focus trap；
- Escape 和 backdrop 只在非提交状态关闭；
- loading、error、取消、确认按钮和 busy 文案；
- 390px 内可滚动且不横向溢出。

会话、运行、数据资源和模型版本组件继续拥有目标、preview DTO、token、刷新与成功副作用。dialog children 只渲染领域影响内容。确认失败时保持 dialog；影响可能变化的流程重新读取 preview，再要求用户以新 token 确认。

版本删除改为显式 dialog，不再使用 `window.confirm`。激活历史版本放弃草稿仍是独立的明确确认语义，本阶段不与删除影响合并。

## 5. 非目标与合同

- 不改变导航、公共路由、DTO、错误码、数据库或功能目录。
- 不把 SSE feed、轮询、cursor pagination 或 mutation 合并成全局 store。
- 不改变固定按钮与自然语言写操作的权限分工。
- 不改变 ECharts/ChartModel、Parquet 或 engine adapter 边界。

## 6. 验收

- 三类下载端点继续返回相同 attachment headers 和字节。
- comparison、chart 和 data resource 读取在快速切换/关闭后忽略迟到响应；失败都有明确重试路径。
- 四类 impact 删除使用同一可访问 dialog shell；版本删除不再调用 `window.confirm`。
- 现有 loading、空、失败、竞争、恢复测试通过，新增迟到响应、focus/Escape、token 刷新和 390px 行为测试。
- 前后端格式/静态检查、Vitest、生产构建、Playwright 与功能覆盖无回归。

