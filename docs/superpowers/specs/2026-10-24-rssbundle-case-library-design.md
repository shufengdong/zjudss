# RSSBundle 案例库目录化设计

## 1. 背景与目标

当前案例资源同时发现 `sparrowzz/dss` 根目录的 `.rss` 文件和
`sparrowzz/dss/rssbundle` 下的 bundle，并以扁平列表提供给前端和 Agent。该结构与 eig-rs
当前发布的案例基线不一致，也丢失了案例在源码目录中的分类关系。

本次调整以当前 `TENSOREVAL_CWD/sparrowzz/dss/rssbundle` 为唯一案例根目录。中间目录仅表示分类；
只有名称以 `.rssbundle` 结尾、且目录内包含 `manifest.json` 的目录才表示案例。案例内容、分析类型和
可导入文件全部由 manifest 决定。

完成后应满足：

- 案例库不再发现或展示 `sparrowzz/dss` 根目录的 `.rss` 文件。
- 前端目录层级与 `rssbundle` 下的真实相对目录层级一致。
- 前端、HTTP API 和 Agent 工具共享同一份后端目录与 manifest 解释结果。
- 应用案例时只导入 manifest 声明的文件，并在导入前验证路径、入口和内容哈希。
- 无效案例仍可在目录中看到并检查诊断，但不能进入模型导入流程。

当前 eig-rs 基线包含 91 个 manifest，顶层目录为 `EPRITestCircuits`、`Examples` 和
`IEEETestCases`。数量只作为当前观测值，不写成产品合同。

## 2. 资源边界与案例身份

资源扫描器只递归访问：

```text
TENSOREVAL_CWD/sparrowzz/dss/rssbundle
```

目录语义如下：

- 普通目录：分类节点，可以包含下一层分类或 `.rssbundle` 案例目录。
- `.rssbundle` 目录：案例边界；必须直接包含 `manifest.json`。
- `.rssbundle` 目录内部：案例内容，不再参与案例分类树递归。
- 普通目录中的其他文件：不是案例，忽略。
- `rssbundle` 根目录之外的 `.rss`、`.dss` 或 manifest：不是案例，忽略。

案例 ID 继续由 `rssbundle` 根目录下的 bundle 相对路径稳定派生。路径移动或重命名表示新案例；同一路径
中文件内容变化会改变 engine resource snapshot 和案例树哈希，但不改变 example ID。

案例结构增加以下规范字段：

```text
id                 稳定 example ID
title              bundle 目录名去掉 .rssbundle
categoryPath       从 rssbundle 根到 bundle 父目录的目录名数组
bundlePath         bundle 相对 rssbundle 根的可展示路径
entryPath          manifest.entry，在 bundle 内的相对路径；manifest 无效时为 null
manifest           已验证并规范化的 manifest 摘要；manifest 无效时为 null
files              manifest.json 与 manifest.files 声明文件
treeHash           manifest 和声明文件内容形成的快照哈希
importable         是否允许进入模型导入
diagnostics        不可导入原因
```

系统不把物理绝对路径暴露给浏览器或 LLM。

## 3. Manifest 合同

后端为 eig-rs 当前 schema 建立显式结构，而不是通过松散 JSON 猜测字段。需读取并保留：

- `schema_version`
- `artifact`
- `generator`
- `psa_contract_version`
- `entry`
- `study.kind`
- `study.compatibility`
- `study.solver`
- `study.results`
- `source.entry`
- `source.root`
- `source.tree_sha256`
- `files[].path`
- `files[].role`
- `files[].sha256`
- `inputs`

其中 `inputs` 保留为 JSON 数组摘要，因为具体输入结构属于 eig-rs manifest 合同，zjudss 不预设仿真类型。

案例可导入必须同时满足：

1. manifest 是合法 JSON，且必填字段可解析。
2. `artifact` 是当前支持的 `mems.dss-rss-bundle`。
3. `entry` 和所有 `files[].path` 都是 bundle 内安全相对路径。
4. `entry` 在 `files` 中且对应文件存在。
5. `files` 不包含重复路径。
6. 每个声明文件存在、是普通文件、未经过符号链接，并且 SHA-256 与 manifest 一致。
7. 文件数量、单案例总字节数和路径深度满足现有资源限制。
8. manifest 文件清单至少包含可供现有模型导入流程处理的 DSS 文件。

`source.tree_sha256` 作为来源追踪元数据展示和保存，不替代逐文件哈希验证。未在 `files` 中声明的 bundle
内部文件既不参与 tree hash，也不进入导入批次。

manifest 无效时，扫描器创建一个不可导入案例叶节点。诊断使用稳定错误码和可读消息，不回传绝对路径或原始
系统错误。单个案例损坏不阻止其他案例被发现；超过全局资源上限、目录逃逸或扫描根不安全仍阻止服务启动。

## 4. 目录树与 HTTP API

后端根据所有案例的 `categoryPath` 生成规范树。目录和案例都按不区分大小写的名称排序；名称相同时使用原始
名称和稳定 ID 保证顺序确定。空目录不返回，因为其中没有可用的案例语义。

`GET /api/examples` 返回完整或经过搜索裁剪的树：

```json
{
  "engineResourceSnapshot": "ers_...",
  "tree": [
    {
      "type": "directory",
      "name": "IEEETestCases",
      "path": ["IEEETestCases"],
      "children": [
        {
          "type": "directory",
          "name": "123Bus",
          "path": ["IEEETestCases", "123Bus"],
          "children": [
            {
              "type": "example",
              "example": {
                "id": "...",
                "title": "Run_IEEE123Bus",
                "categoryPath": ["IEEETestCases", "123Bus"],
                "bundlePath": "IEEETestCases/123Bus/Run_IEEE123Bus.rssbundle",
                "studyType": "execution_plan",
                "importable": true
              }
            }
          ]
        }
      ]
    }
  ]
}
```

查询参数保留 `query` 和 `studyType`。`query` 匹配标题、bundle 路径、分析类型、solver、结果名和
`source.entry`；返回结果保留所有匹配案例及其祖先目录。案例规模受启动时的 `max_examples` 限制，因此目录
接口不再使用会破坏树完整性的默认 20 条截断。`sourceKind` 固定为 `rssbundle`，不再作为有效筛选维度。

`GET /api/examples/{id}` 返回案例摘要、目录路径、规范化 manifest 摘要、声明文件角色与哈希、tree hash
和诊断。manifest 无法解析或不满足合同时，`manifest` 和 `entryPath` 为 `null`。它仍不返回文件内容或物理路径。

`POST /api/sessions/{sessionId}/draft/example` 请求格式保持不变。后端用 example ID 和
`engineResourceSnapshot` 锁定资源版本。

## 5. 应用案例与模型导入

应用流程保持现有“进入导入确认，不直接发布版本”的产品语义：

1. 校验用户、会话、资源快照和 draft revision。
2. 按 example ID 读取当前快照内的案例。
3. 拒绝 `importable=false` 的案例。
4. 从已发布 engine resource snapshot 中读取 manifest 和声明文件。
5. 再次校验每个文件 SHA-256，防止快照发布后资源被替换。
6. 使用 manifest 中的相对路径构造上传批次；保留 `manifest.json`，使现有 bundle ingestion 仍以 manifest
   为入口解析完整模型。
7. 模型导入的 source summary 记录 example ID、title、categoryPath、bundlePath、snapshot ID、manifest
   schema、source tree hash、案例 tree hash 和文件数量。
8. 后台执行现有 inspect/normalize 流程，前端继续显示候选 DataFrame 并要求用户确认合并。

该流程不创建脚本、模型版本、仿真运行或结果。

## 6. 前端案例库

案例库弹窗改为目录浏览器：

- 左侧或主列表区域展示可展开的目录节点与案例叶节点，初始展开顶层目录。
- 展开状态由目录 path 标识；搜索时自动展开匹配案例的祖先目录。
- 案例叶节点显示名称、study kind、solver 和验证状态。
- 选择叶节点后在详情区域展示 bundle 路径、来源入口、兼容性、结果类型、文件角色统计、生成器和诊断。
- 只有 `importable=true` 时显示可用的“使用此案例”操作。
- 搜索无结果、加载失败、无案例和无效案例都有明确状态。
- 窄屏按“目录列表 → 案例详情 → 返回”的单列方式工作，避免强制横向双栏。

目录浏览属于高频固定交互，使用展开、返回和“使用此案例”按钮；案例应用后的参数修改与仿真仍通过现有聊天和
工作区流程完成。

## 7. Agent 工具

`search_examples` 继续只读，返回匹配案例的：

- example ID
- title
- category path 和 bundle path
- study kind、solver、compatibility、results
- source entry
- importable 和 diagnostics
- snapshot ID

`inspect_example` 返回单个案例的规范 manifest 摘要、声明文件角色与哈希以及验证结果。工具不接收路径作为
身份，也不返回物理路径。Agent 仍无权直接应用案例；用户通过前端固定按钮确认案例导入。

## 8. 启动、快照与发布门禁

engine resource snapshot 的身份包含：

- 每个案例稳定 ID、bundle 相对路径和 category path
- manifest 字节哈希
- manifest 声明文件的路径、角色和实际内容哈希
- importable 状态和诊断

删除根 `.rss` 案例发现后，启动日志只报告发现案例数、可导入数和不可导入数。真实依赖发布门禁改为：

- 至少发现一个 rssbundle 案例。
- 至少有一个可导入案例。
- 所有被标记为可导入的案例均能进入模型导入流程。
- 代表性测试至少覆盖 `execution_plan`、`static_power_flow`、`fault_study`、`harmonic` 中实际存在的类型。

资源树变更后仍需重启服务生成新快照。

## 9. 错误处理与安全

- 扫描和导入都拒绝绝对路径、`..`、符号链接和 bundle 根逃逸。
- API 与 Agent 输出只包含相对展示路径。
- manifest 声明文件缺失、重复、哈希不符或入口缺失时，案例不可导入并提供稳定诊断。
- 资源快照不匹配继续返回现有冲突错误，要求刷新案例库后重试。
- 搜索与目录构建完全基于启动时不可变快照，不在请求期间重新访问 eig-rs 源目录。
- 单个案例的格式错误不影响其他案例；全局限制或扫描根安全错误继续使启动失败。

## 10. 测试要求

后端单元与集成测试覆盖：

- 只扫描 `rssbundle`，忽略根 `.rss` 和其他位置的 manifest。
- 多级分类目录形成确定的树，`.rssbundle` 内部目录不会成为分类节点。
- manifest 所有规范字段解析和 API 序列化。
- 非法 JSON、错误 artifact、不安全路径、重复文件、入口未声明、缺文件、符号链接和哈希不符。
- 未声明文件不进入案例 tree hash 和模型导入批次。
- 搜索匹配 metadata 并保留祖先目录。
- 快照变化、案例导入 source summary 和导入前二次哈希校验。
- Agent 搜索与检查工具返回路径和 manifest 元数据，且不泄露物理路径。

前端测试覆盖：

- 目录递归渲染、展开与收起。
- 搜索自动展开祖先目录和无结果状态。
- 选择案例显示 manifest 详情。
- 无效案例禁用导入并显示诊断。
- 窄屏目录与详情切换。

Playwright 覆盖真实浏览器从目录展开、查看详情、使用案例、确认候选 DataFrame 到形成 draft 的完整链路。
真实 eig-rs 发布测试验证全部发现案例，并对各类代表性案例执行模型导入。

## 11. 文档一致性

同步更新以下合同：

- `docs/architecture.md`：案例资源快照和 manifest 驱动导入。
- `docs/api-design.md`：目录树与详情响应。
- `docs/frontend-design.md`：目录浏览器交互。
- `docs/development-status.md`：删除根 RSS 门禁和计数描述。
- `docs/v3-deployment.md`：真实依赖验收条件与运行日志。
- 自动化功能目录：案例树、manifest 校验与完整应用链路。

旧 V2 计划和历史设计作为历史记录不回写；当前 V3 状态与部署文档必须以本设计为准。
