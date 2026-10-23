# V3 固定用户登录与数据隔离设计

> 状态：已批准。本文只定义 V3 目标，不为未上线的 V1/V2 保留数据库、接口或配置兼容。

## 1. 背景与意图

V3 为配电网仿真智能体增加一套简单、固定的用户登录能力。用户清单来自
`backend/resources/init_users.csv`，当前文件含 `ID,NAME` 两列和 30 条用户记录。系统不提供用户新增、
删除、资料修改、找回密码或修改密码功能；修改用户清单后通过重置数据库重新初始化。

每个用户登录后只能访问自己的会话、模型、上传文件、仿真任务、分析结果、历史搜索和存储配额。左侧栏
底部显示当前用户名，交互参考常见对话产品：点击用户区展开用户 ID 和退出入口。

系统允许部署到公网，但本版本明确不把账号安全强化作为目标。密码固定等于用户 ID 是已知且已接受的
风险；设计仍采用密码哈希、随机会话令牌和 HttpOnly Cookie，避免无成本的明文存储和浏览器脚本泄露。

## 2. 目标、非目标与反目标

### 目标

- 空数据库首次启动时，从固定 CSV 原子初始化全部用户。
- 使用用户 ID 和密码登录；初始且永久密码等于用户 ID。
- 登录在浏览器关闭和重新打开后保持，默认 30 天滑动有效。
- 退出登录后，当前服务端会话立即失效。
- 所有用户业务数据和配额按用户严格隔离，跨用户与不存在资源统一返回 404。
- `/api/me` 返回稳定用户 ID 和显示名称；前端左侧栏底部显示名称。
- 未登录用户只能访问健康检查、登录和幂等退出接口。

### 非目标

- 用户注册、邀请、增删改、禁用、角色和权限管理。
- 修改密码、首次登录改密、找回密码、验证码、MFA 或第三方登录。
- 登录失败限流、账号锁定、异常登录检测、设备管理或会话管理页面。
- V1/V2 数据迁移、旧身份 scope 兼容、旧 Cookie 或旧配置兼容。
- TLS、WAF、反向代理或公网边界防护方案。

### 反目标

- 不把 CSV 当成运行期用户目录，不在每次启动时静默增量同步。
- 不在数据库、Cookie、localStorage、日志或 API 响应中保存或暴露明文密码。
- 不以客户端传入的用户 ID 决定资源归属。
- 不保留 `instance/trusted_proxy` 双模式造成的身份分支；V3 只有固定用户登录这一条生产身份链路。

## 3. 方案选择

### 方案 A：数据库持久化的不透明会话令牌（已选）

登录成功后生成高熵随机令牌，浏览器 Cookie 保存原始令牌，数据库只保存令牌的 SHA-256 摘要。每次请求
由后端查询会话并得到用户身份。退出时删除会话记录，因此能立即撤销；会话过期和滑动续期也可测试、
可观测。

代价是新增一张认证会话表，并在受保护请求上增加一次轻量数据库查询。

### 方案 B：签名的无状态 Cookie（未选）

Cookie 内携带用户 ID 和过期时间，可省去认证会话表和查询。但服务端不能可靠撤销单个已签发会话，
退出主要依赖浏览器删除 Cookie；密钥轮换还会让全部用户同时退出，不适合已确认的“退出立即失效”。

### 排除方案

HTTP Basic Auth、前端用户选择器和长期 API Token 都不能同时满足产品化登录页、持久登录和可靠退出，
不进入实现候选。

## 4. V3 数据模型

V3 只支持从空库初始化，因此直接重写权威 initial migration，不追加兼容迁移。

### `users`

| 字段 | 约束与含义 |
|---|---|
| `id` | `TEXT PRIMARY KEY`；CSV 中的稳定用户 ID，同时是登录账号和初始密码来源 |
| `name` | 非空显示名称 |
| `password_hash` | 非空 Argon2id 编码哈希；绝不保存原始 ID 密码 |
| `storage_scope_id` | 唯一外键，指向该用户的配额与 artifact 计费主体 |
| `created_at_ms` | 初始化时间 |

用户 ID 和名称在导入时去除字段两端空白，然后校验非空、无控制字符、ID 唯一且名称长度有界。CSV 必须
只有 `ID,NAME` 两列；空文件、重复 ID、多余字段或非法 UTF-8 均使启动失败，且不留下部分用户。

### `auth_sessions`

| 字段 | 约束与含义 |
|---|---|
| `token_hash` | `TEXT PRIMARY KEY`；原始随机令牌的 SHA-256 十六进制摘要 |
| `user_id` | 外键到 `users(id)` |
| `created_at_ms` | 登录时间 |
| `last_seen_at_ms` | 最近一次触发续期的时间 |
| `expires_at_ms` | 服务端最终有效期 |

为 `user_id` 和 `expires_at_ms` 建索引。过期记录可在认证查询命中时删除，并在启动时做一次有界清理；
本版本不增加独立后台清理任务。

### 所有权与配额

- `storage_scopes` 继续负责 artifact 数量、字节配额和引用计数，但 V3 只保留用户拥有的 scope；每个
  `users.storage_scope_id` 一对一绑定一个 scope。
- 业务 `sessions` 直接保存 `user_id` 外键，不再保存 `access_scope_kind/access_scope_key`。
- repository 的公共查询入口显式接收认证后的 `UserId`。会话子资源继续通过 join 到业务 session 验证
  所有权，不能只凭资源主键查询。
- artifact 内容仍可在物理 CAS 层按哈希去重，但可见性和配额引用只通过当前用户的 storage scope。
- 删除业务会话不会删除用户；用户本身在产品中没有删除路径。

这种模型有意删除 V2 的 `instance/owner/tenant` 通用 scope 抽象。固定本地用户是 V3 唯一身份来源，直接
外键比字符串 scope 更简单，也能让数据库约束参与隔离。

## 5. CSV 初始化合同

`init_users.csv` 作为构建输入嵌入后端二进制。修改 CSV 后重新构建并重置数据库，下一次启动即按新内容
初始化；运行中的部署不会读取或热更新外部 CSV 文件。

迁移完成后，启动过程在单个事务内执行：

1. 解析并完整校验 CSV。
2. 计算原始 CSV 字节的 SHA-256。
3. 若 `users` 为空，为每个 ID 生成独立 Argon2id salt 和密码哈希，建立对应 storage scope，并插入用户。
4. 在 `system_metadata` 保存 `init_users_sha256`。
5. 若 `users` 已非空，只比较已保存 hash 与当前内嵌 CSV；相同则继续启动，不进行同步。
6. hash 不同则以稳定错误 `user_seed_mismatch_reset_required` 拒绝启动，明确要求重置数据库。

解析、哈希或任一插入失败时事务整体回滚。CSV 中的 ID 不出现在普通启动日志；日志只记录用户数量和
seed hash 的短前缀。

## 6. 认证与会话流程

### 登录

`POST /api/auth/login`

```json
{"id":"用户 ID","password":"用户输入"}
```

后端按 ID 查询用户并验证 Argon2id。用户不存在与密码错误统一返回：

```json
{"error":"用户 ID 或密码错误","code":"invalid_credentials"}
```

成功时创建 32 字节以上的密码学随机令牌，只将摘要写入 `auth_sessions`，并返回用户：

```json
{"user":{"id":"...","name":"..."}}
```

同时设置 `zjudss_session` Cookie：`HttpOnly`、`SameSite=Lax`、`Path=/`；默认
`Max-Age=2592000`（30 天），实际值由 `SESSION_TTL_DAYS` 统一派生。生产部署通过配置启用 `Secure`；
Cookie 中只有不透明令牌。

### 请求认证

除 `GET /health`、`POST /api/auth/login` 和幂等的 `POST /api/auth/logout` 外，所有 API 路由都在统一认证
middleware 后。middleware：

1. 读取 Cookie，计算令牌摘要。
2. 查询未过期 `auth_sessions` 并 join `users`。
3. 将只读 `CurrentUser { id, name, storage_scope_id }` 放入请求 extensions。
4. 无 Cookie、令牌未知或已过期统一返回 `401 authentication_required`。

业务 handler 和 repository 不读取 Cookie，也不接收客户端声称的用户 ID。

### 持久与滑动续期

会话有效期统一取 `SESSION_TTL_DAYS`，默认 30 天。Cookie `Max-Age`、数据库 `expires_at_ms` 和续期增量
都使用这个值；续期阈值固定为 TTL 的一半。为避免每个请求都写数据库，仅当剩余有效期小于等于该阈值
时更新 `expires_at_ms/last_seen_at_ms` 并重新发送 Cookie，并发续期必须幂等。

数据库提交与 HTTP 响应无法跨边界原子化，因此这里采用单向保证：必须先成功提交数据库续期，才能在
响应中附加续期 Cookie；数据库更新失败则当前请求失败且不发送新 Cookie。数据库已经续期但响应在网络
中丢失是允许的，此时客户端仍持有原到期时间，数据库中多出的有效期最终按过期清理，不扩大客户端访问
期限。

SSE 只在建连请求时参与滑动续期，服务端 heartbeat 本身不视为用户活动，不能让一个无人使用但长期打开的
标签页永久续期。事件流保存认证会话的稳定内部标识，并在 heartbeat 时重新检查过期/撤销状态；会话过期
后主动结束流。logout 删除会话记录后同时向该会话的所有活动 SSE 连接广播撤销信号，使已建立的事件流
立即终止，而不是继续接收数据到下一次普通 API 请求。

### 当前用户与退出

- `GET /api/me` 返回 `{"id":"...","name":"..."}`。
- `POST /api/auth/logout` 不要求已有有效身份。若 Cookie 对应一条会话则删除记录；无 Cookie、令牌未知或
  已过期也统一返回 204，并发送过期 Cookie。因此重复退出保持幂等。
- 登出只撤销当前浏览器会话；同一用户在其他浏览器的会话继续有效。本版本无“退出所有设备”。

## 7. HTTP 与错误语义

V3 新增或改变的公共合同：

| 方法 | 路径 | 是否公开 | 语义 |
|---|---|---|---|
| POST | `/api/auth/login` | 是 | 校验固定用户并建立持久会话 |
| POST | `/api/auth/logout` | 是 | 若存在则撤销当前会话，并始终清 Cookie |
| GET | `/api/me` | 否 | 返回用户 `id/name` |
| GET | `/health` | 是 | 存活检查，不泄露用户信息 |

`Principal` DTO 被 `User { id, name }` 取代，不再向前端暴露 `subject/scopeKind/scopeKey`。领域资源的跨
用户访问继续使用 404 `not_found`；身份缺失使用 401 `authentication_required`；登录凭据错误使用
401 `invalid_credentials`。

## 8. 前端体验

### 认证状态机

应用根节点维护 `loading | anonymous | authenticated` 三态：

- 初始请求 `/api/me`；成功后才加载会话、配额和其他业务数据。
- 返回 401 时进入 anonymous，只渲染登录页，不触发业务 API 或 SSE。
- 登录成功后保存内存中的 `User` 并进入主工作台；不在 localStorage 保存身份、密码或会话令牌。
- 任一普通 API 返回 `authentication_required` 时，由统一 API client 发出认证失效事件，清空所有用户
  业务内存状态并回到登录页。
- SSE 连接异常时执行一次 `/api/me` 探测；确认 401 后停止重连并回到登录页，避免无休止重连。

### 登录页

未登录页面为单一居中卡片，包含产品名称、用户 ID、密码、登录按钮和通用错误区域。按 Enter 可提交；
提交期间禁用重复提交。页面不显示注册、找回密码、改密、“记住我”或第三方登录入口。持久登录由服务端
Cookie 固定提供。

### 左侧栏用户区

历史会话列表下面增加固定底栏，与滚动列表分离：

- 显示由姓名首字符生成的头像占位和完整 `name`，过长时截断并提供 title。
- 点击后向上展开菜单，显示只读用户 ID 和“退出登录”。
- 退出成功后清空当前用户的消息、工作区、任务、结果和选择状态，再显示登录页。
- 窄屏抽屉使用同一底栏；用户菜单打开时应具备焦点管理、Escape 关闭和可访问名称。

导航偏好键改为 `zjudss:last-session:user:<id>`，避免不同用户互相恢复对方的 session ID。该值只作导航
偏好，服务端仍会验证归属；无权或不存在时回退到当前用户的第一条会话。

## 9. 配置与运行

- 删除 V3 不再需要的 `AUTH_MODE`、`AUTH_PROXY_TOKEN` 以及对应 instance/trusted-proxy 分支。
- 增加 `SESSION_TTL_DAYS`，默认 30；只接受正整数。它同时控制 Cookie `Max-Age`、数据库有效期和滑动
  续期增量，续期阈值为 TTL 的一半。
- 增加 `SESSION_COOKIE_SECURE`，开发默认 false，公网 HTTPS 部署设为 true。
- 随机会话令牌不需要签名密钥，因为服务端只接受数据库中摘要匹配的令牌。
- V3 发布必须使用空数据库和空业务 `DATA_DIR`；不提供 V2 数据迁移或双读。
- `init_users.csv` 必须进入构建依赖和发布物审计；候选发布报告记录其 SHA-256。

尽管产品决定暂不做安全加固，部署文档仍需醒目标明“密码等于 ID”不适用于存放敏感数据的正式公网
服务。这是风险记录，不改变本次功能范围。

## 10. 失败与边界情况

- CSV 非法或 seed hash 与已有数据库不一致：启动前失败，不接流量。
- 登录失败：保留用户输入的 ID，清空密码并显示统一错误。
- Cookie 过期或数据库被重置：下一请求回登录页，不保留旧用户的内存数据。
- 用户在多个标签页退出：其他标签页在下一 API 请求或认证探测后回登录页；可用 `storage`/BroadcastChannel
  作为即时体验优化，但服务端撤销才是事实来源。
- 登录后用户没有会话：显示空工作台，由用户新建仿真。
- 同一浏览器依次登录不同用户：所有 React keyed state 必须重建，不能只替换顶部用户对象。
- 用户名重复允许，因为身份以唯一 ID 为准；侧栏菜单同时展示 ID 以消除歧义。

## 11. 验证与追溯

### 后端测试

- CSV 正常初始化 30 个用户，密码字段为 Argon2id 哈希且不等于 ID。
- 空、重复 ID、错误表头、多列、非法字段和事务中失败不会产生部分用户。
- 相同 seed 可重复启动；修改 seed 后对非空数据库返回 `user_seed_mismatch_reset_required`。
- 正确/错误/未知用户登录，Cookie 属性、持久期限、滑动续期、过期和幂等退出。
- 未登录不能访问任何受保护 API。
- 两个用户之间对会话、消息、上传、workspace、版本、任务、dataset、下载、分析、比较、历史搜索和配额
  均隔离；已知对方 ID 也只能得到 404。
- 同一用户的两个认证会话相互独立，退出一个不会撤销另一个。
- SSE heartbeat 不延长登录期限；会话到期或当前浏览器 logout 会主动关闭对应事件流。

### 前端测试

- `/api/me` 的 loading、anonymous、authenticated 三态。
- 登录表单键盘提交、通用错误、重复提交保护和成功切换。
- 左侧栏底部姓名、用户 ID 菜单、退出和窄屏行为。
- 401 清空用户业务状态；切换用户不会闪现上一用户数据。
- SSE 因认证失效停止重连并返回登录页。

### 浏览器全链路

- 浏览器登录后刷新和重启上下文仍保持登录。
- 用户 A 创建会话、上传并运行后，用户 B 的列表、搜索和直接 URL 均不可见。
- 退出后 Cookie 失效，浏览器后退或直接请求受保护 URL 不能恢复数据。
- 数据库重置后按当前 CSV 重新初始化，旧 Cookie 自动失效。

所有新增公共路由、DTO、错误码和界面行为必须同步更新：

- `docs/api-design.md`
- `docs/db-design.md`
- `docs/frontend-design.md`
- `docs/development-status.md`
- `docs/v2-deployment.md`（实施时重命名为 V3 发布文档）
- `docs/integration-testing-design.md`
- `frontend/src/types/`
- `integration-tests/catalog/features.yaml`

功能目录必须为登录、退出、持久会话、CSV 初始化和逐类跨用户隔离关联真实行为测试；路由枚举或表结构
断言不能替代行为覆盖。

## 12. 留给实施者的局部选择

以下选择不改变产品行为，可在实施计划中按现有 Rust/Axum/React 技术栈决定：

- Cookie 解析库和安全随机数库的具体 crate。
- Argon2id 的具体参数，前提是在开发和 CI 中耗时可接受并记录为常量。
- 多标签页即时退出使用 BroadcastChannel 还是 storage event。
- 过期认证会话启动清理的单次批量上限。
