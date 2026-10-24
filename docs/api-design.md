# V3 HTTP 与 SSE API

## Conventions

除 `/health` 与登录外，所有路由要求 HttpOnly 登录 Cookie。ID 出网为字符串，时间为 ISO 8601，字段为
camelCase。跨用户与不存在资源统一 404。错误响应为
`{"error":"…","code":"stable_code","details":…}`。上传使用 multipart；模型导入返回 202，运行创建返回
202（幂等重放为 200）。

## Routes

| Method | Path | Semantics |
|---|---|---|
| GET | `/health` | 存活检查 |
| POST | `/api/auth/login` | 固定用户登录 |
| POST | `/api/auth/logout` | 撤销当前浏览器会话 |
| GET | `/api/me` | 当前用户 |
| GET | `/api/storage-usage` | 当前用户配额 |
| GET | `/api/examples` | 搜索可模型化案例 |
| GET | `/api/examples/{id}` | 案例详情 |
| POST | `/api/sessions/{id}/draft/example` | 案例进入模型导入 |
| GET/POST | `/api/sessions` | 列表/新建会话 |
| PATCH/DELETE | `/api/sessions/{id}` | 改名/删除整个会话 |
| GET/POST | `/api/sessions/{id}/messages` | 分页消息/发送消息；POST 返回权威 `sessionTitle` 供界面立即同步自动标题 |
| GET | `/api/sessions/{id}/events` | SSE replay + live |
| GET | `/api/history/search` | 当前用户历史检索 |
| POST | `/api/sessions/{id}/model-imports` | 上传模型来源 |
| GET/DELETE | `/api/model-imports/{id}` | 读取/放弃导入 |
| POST | `/api/model-imports/{id}/resolve` | 选择候选并整表合并草稿 |
| GET/POST | `/api/sessions/{id}/scripts` | 列表/上传单个 `.rss` |
| GET | `/api/scripts/{id}` | 懒加载脚本全文 |
| GET | `/api/sessions/{id}/workspace` | active、draft、导入状态 |
| GET/POST | `/api/sessions/{id}/versions` | 历史/显式生成版本 |
| GET/DELETE | `/api/versions/{id}` | 详情/带 impact token 删除 |
| POST | `/api/versions/{id}/activate` | 激活历史版本；草稿需确认放弃 |
| POST | `/api/versions/{id}/deletion-preview` | 生成短期删除影响 token |
| POST | `/api/scripts/{id}/runs` | 唯一运行创建入口 |
| GET | `/api/sessions/{id}/runs` | 运行记录 |
| GET | `/api/runs/{id}` | 冻结运行快照 |
| GET | `/api/runs/{id}/datasets` | 运行的存活结果 |
| GET/DELETE | `/api/datasets/{id}` | 元数据/删除单个结果 |
| GET | `/api/datasets/{id}/rows` | cursor 分页投影 |
| GET | `/api/datasets/{id}/download` | 下载 Parquet |
| GET | `/api/analysis-runs/{id}` | 分析记录与结果 |
| GET | `/api/dataset-views/{id}/data` | 物化视图数据 |
| GET | `/api/run-comparisons/{id}` | 运行比较及派生结果 |

## Model import and draft

模型上传字段为 `clientImportId`、可选 `expectedDraftId/expectedDraftRevision`、`relativePaths`，以及一个或多个
`file.<clientFileId>`。resolve 提交 `candidateIds` 与同一 expected draft。状态为
`accepted|processing|awaiting_resolution|ready_to_apply|applied|failed|discarded`。只有 resolve 最终事务修改草稿。

发布请求携带 `expectedDraftId`、`expectedDraftRevision`、`clientPublishId`。成功冻结版本、清空草稿并自动激活。
激活历史版本只移动 active 指针；存在草稿时必须提交 `discardDraft=true` 以及准确 expected draft。

## Scripts and runs

上传脚本 multipart 只接受字段 `file` 的单个 UTF-8 `.rss`。脚本详情返回全文；列表和消息只返回有界预览。
运行请求为 `{modelVersionId, clientRunId}`。服务端要求该版本仍为 active，重新检查脚本静态有效性和所需
DataFrame alias，再异步执行。Agent 没有运行、激活或删除工具。

## Messages and errors

持久消息类型是 `user`、`assistant`、`tool-call`、`progress`、`result`、`attachment`、`model-status`、
`comparison`、`script`。运行关联字段统一为 `runId`。成功的 `create_script` tool-call 和 `script` 消息是脚本生成的持久成功事实；
后续模型收尾失败不追加通用 Agent 失败，缺少 assistant 收尾时由服务端补充检查并点击“运行”的提示。

主要稳定错误包括认证/隔离、配额与上传、导入与草稿冲突、脚本校验/兼容、active race、运行队列、版本删除
影响变化、结果 busy、分析/比较/视图校验、历史索引不可用、engine snapshot/contract 不匹配。权威枚举由
`cargo run --bin contract-manifest` 产生，并由功能目录逐项覆盖。
