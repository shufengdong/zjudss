# 配电网仿真智能体 V3 HTTP/SSE 契约

> 状态：已实现。字段使用 camelCase，所有 ID 为字符串，时间为 ISO 8601。前端权威类型位于
> `frontend/src/types/`。

## 1. 通用规则

- JSON API 位于 `/api`；`GET /health` 返回文本 `ok`。
- 错误体：`{"error":"可读信息","code":"stable_code","details":{...}}`。
- 除 `GET /health`、`POST /api/auth/login` 和 `POST /api/auth/logout` 外，所有端点都要求有效的
  `zjudss_session` Cookie。跨用户和不存在资源都返回相同的 `404 not_found`。
- 写领域状态默认通过 `POST /sessions/{id}/messages` 的自然语言触发；REST 写接口只负责会话管理和
  文件传输。

## 2. 端点

| 方法 | 路径 | 作用 |
|---|---|---|
| POST | `/api/auth/login` | 使用固定用户 ID/密码登录并设置 HttpOnly Cookie |
| POST | `/api/auth/logout` | 撤销当前浏览器登录并清除 Cookie；未登录时也返回 204 |
| GET | `/api/me` | 当前用户 `{id,name}` |
| GET/POST | `/api/sessions` | 会话列表/创建 |
| PATCH/DELETE | `/api/sessions/{id}` | 重命名/删除 |
| GET/POST | `/api/sessions/{id}/messages` | 消息分页/提交自然语言 |
| GET | `/api/sessions/{id}/events` | 可恢复 SSE |
| POST | `/api/sessions/{id}/attachments` | 多文件/目录上传 |
| GET | `/api/history/search` | scoped 聊天全文搜索 |
| GET | `/api/examples` | 只读案例搜索 |
| GET | `/api/examples/{id}` | 案例详情和受控文件树 |
| GET | `/api/sessions/{id}/workspace` | active/draft/活动摄取 |
| GET | `/api/sessions/{id}/versions` | 版本历史 |
| GET | `/api/versions/{id}` | 版本详情和输入推断 |
| GET | `/api/ingestions/{id}` | 摄取恢复 |
| GET | `/api/sessions/{id}/tasks` | 仿真历史 |
| GET | `/api/tasks/{id}` | 可复现任务详情 |
| GET | `/api/tasks/{id}/datasets` | 原始结果目录 |
| GET | `/api/datasets/{id}` | DataFrame metadata/schema/views |
| GET | `/api/datasets/{id}/rows` | cursor 分页和列投影 |
| GET | `/api/datasets/{id}/download` | Parquet 下载 |
| GET | `/api/analysis-runs/{id}` | 分析血缘和输出 |
| GET | `/api/dataset-views/{id}/data` | 预算内视图投影 |
| GET | `/api/task-comparisons/{id}` | 比较、兼容性和差值数据集 |
| GET | `/api/storage-usage` | 当前计费 scope 的配额 |

## 3. 登录会话

`POST /api/auth/login` 请求为 `{"id":"...","password":"..."}`，成功返回
`{"user":{"id":"...","name":"..."}}` 并设置 `zjudss_session`。Cookie 为 `HttpOnly`、
`SameSite=Lax`、`Path=/`，有效期由 `SESSION_TTL_DAYS` 控制；公网 HTTPS 部署必须设置
`SESSION_COOKIE_SECURE=true`。未知 ID 与错误密码统一返回 `401 invalid_credentials`。

服务端只保存随机 token 的 SHA-256。登录会话进入后一半有效期时由普通受保护请求续期；SSE 心跳只
检查有效性而不续期。当前 Cookie 登出或到期后，其 SSE 立即结束；同一用户的其他浏览器登录不受影响。
缺少、未知或过期 Cookie 返回 `401 authentication_required`。

## 4. 会话、消息和 SSE

`GET /api/sessions/{id}/messages?limit=50&beforeId=...&aroundId=...` 返回：

```json
{"messages":[],"nextBeforeId":"120","hasMoreBefore":true}
```

`beforeId` 与 `aroundId` 互斥，limit 范围 1–100。POST 请求为
`{"message":"把负荷提高 20% 后重新做潮流并比较"}`，admission 成功返回
`202 {"ok":true}`；队列满返回 429 且不落消息。

SSE 的持久帧格式：

```text
id: 128
event: message
data: {"id":"128","type":"assistant","timestamp":"...","content":"完成"}
```

客户端重连可携带 `Last-Event-ID`。无法补发时服务端发送 `event: resync-required`，客户端重新读取
消息快照。

## 5. 八类消息

公共字段为 `id/type/timestamp`。

- `user`：`content`
- `assistant`：Markdown `content`
- `tool-call`：稳定 `callId`、`toolName/args/status/summary`
- `progress`：`operation:{kind,id}` 和全量 `stages`
- `attachment`：`clientBatchId/clientFileId/ingestionId/uploadStatus/file`
- `model-status`：`workspaceId/scope(active|draft)/snapshot|null`
- `result`：`taskId` 或 `analysisRunId`、title 和通用 dataset outputs
- `comparison`：comparison/baseline/candidate/analysis ID 和差值 outputs

progress 按 operation 合并，tool-call 按 callId 合并，附件按 batch/file 合并，result 按 producer 和
dataset 更新。消息中不包含固定 metrics、服务器路径或任意图表 option。

## 6. 上传

multipart 字段：

- `clientBatchId`：本次选择的稳定幂等 ID；
- 每个文件为 `file.<clientFileId>`；
- `relativePaths`：clientFileId 到规范相对路径的 JSON 对象。

接受后返回 `202 {"ok":true,"ingestionId":"...","accepted":[...]}`。相同 batch/file ID 与相同字节
幂等；复用 ID 但改变字节返回 `409 idempotency_conflict`。错传或修正文件应生成新 ID，随后在对话
中说明替换/移除关系。

## 7. 案例和工作区

`GET /api/examples?query=...&studyType=...&sourceKind=...&limit=20` 返回当前
`engineResourceSnapshot` 和案例摘要。详情只暴露虚拟相对路径、hash、验证状态和 required features，
不暴露 TENSOREVAL_CWD。导入没有公开 POST；案例卡发送可见自然语言消息，由 agent 调用
`import_example`。

workspace 返回 `id/sessionId/active/draft/activeIngestions`。版本输入含 artifact/input ID、文件名、
sheet、table kind、confidence、resolution、候选、匹配/缺失列和 dtype 冲突。

## 8. 任务与 DataFrame

任务详情固定返回 modelVersionId、studyType/spec、scriptArtifactId、agentBundleVersion、
agentSkillRefs、engineContractVersion、状态、错误和时间。

`GET /api/datasets/{id}/rows?limit=100&cursor=...&columns=a,b` 返回：

```json
{"datasetId":"7","columns":["a","b"],"rows":[[1,null]],"nextCursor":null,"diagnostics":{}}
```

limit 范围 1–500，cursor 绑定 dataset/schema，不能跨数据集复用。下载媒体类型为
`application/vnd.apache.parquet`。

视图只支持 line/bar/scatter/heatmap/boxplot/histogram 的受控 spec。超预算返回 422 和
`details:{kind,estimated,limit}`，不返回静默截断的数据。

## 9. 历史搜索和配额

`GET /api/history/search?query=...&studyType=...&status=...&fromMs=...&toMs=...&limit=20` 返回 session
命中、标题、更新时间、score 和 snippet。FTS 不可用返回 503，消息读取不受影响。

`GET /api/storage-usage` 返回 artifact/字节使用量、限制、warningRatio、isWarning 和 isExceeded。
配额满只拒绝新 artifact。
