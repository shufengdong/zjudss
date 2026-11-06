# 受管脚本 RSS 语法文档与多结果防线设计

## Intent

修复受管研究在选择多个结果时，ScriptAuthor 生成不合法的
`a, b = facade(...);`，合同检查误放行后由 tensoreval 报
`Unexpected char`，并且相同脚本在修正循环中重复执行的问题。
eig-rs/sparrowzz 的 RSS 文档继续是语法权威来源。

## Goals

- generation 与 repair 都携带当前 engine snapshot 中的函数文档，以及
  `RSS 语言参考`的“顶层程序约束”和“多结果声明”章节。
- 单结果继续接受 `result = facade(...);`；多结果只接受
  `[first, second] = facade(...);`。
- 与父 attempt 内容 hash 相同的 repair candidate 仍保存为不可变审计记录并消耗修正次数，
  但标记为合同拒绝，不再调用引擎。
- 保留现有最多 3 次修正、4 个 attempt、脱敏诊断、稳定 ID、Parquet 和 adapter 边界。

## Non-goals

- 不改为服务端直接拼接完整研究脚本，不移除独立 ScriptAuthor。
- 不修改 REST/SSE DTO、数据库 schema、前端、Agent 工具或 eig-rs 资源树。
- 不恢复任意路径、命令或原始 stderr 到模型请求。

## Approaches

1. **只加强 Prompt。** 改动最小，但模型仍可能忽略规则，且合同检查继续误放行；不采用。
2. **语法文档注入 + Prompt + 合同校验 + 重复 hash 防线。** 保持现有架构，并在模型、应用合同和执行边界形成三层防线；采用。
3. **服务端确定性生成完整 RSS。** 能消除本类模型随机性，但会替换当前 ScriptAuthor 架构并扩大范围；留作后续设计。

## Design

`EngineKnowledge` 增加 ScriptAuthor 专用文档选择：先从当前 snapshot 中选择标题包含
“顶层程序约束”或“多结果声明”的必需章节，再追加按锁定函数名检索的章节，按 section ID 去重并保持总字符预算。
缺少必需章节时仍由显式 Prompt 语法和合同检查失败关闭，不读取 snapshot 外文件。

`ScriptGenerationRequest` 与 `ScriptRepairRequest` 都携带同一组 `EngineDocRef`。ScriptAuthor
规则明确区分单结果与多结果左值形式。运行 attempt 继续保存实际 engine snapshot、bundle、skill refs 和脚本 hash；
本次不扩展数据库以持久化完整文档片段。

`managed_rss::AssignmentCall` 记录左值是否使用方括号。`ScriptContractInspector` 在锁定结果超过一个时，
拒绝未使用方括号的调用，并通过现有 `repair_contract_violated` 诊断进入修正循环。

repair candidate 保存后，将其 `script_content_hash` 与直接父 attempt 比较。相同时使用现有
`contract_rejected` attempt 状态和 `repair_contract_violated` 诊断，跳过 `SimEngine::run`；
因此审计链、修正预算和前端 `scriptChangedFromParent=false` 均保留。

## Verification

- 单元测试覆盖 ScriptAuthor 文档选择、方括号标记和多结果合同。
- 集成测试覆盖 generation/repair 请求实际携带语法章节，以及相同 hash repair 不增加引擎调用。
- 运行后端完整 e2e-test、格式、Clippy、功能追溯；真实服务重启后再用当前案例验证。
