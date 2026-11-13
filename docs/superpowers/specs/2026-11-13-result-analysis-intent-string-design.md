# 结果分析 intent 字符串合同设计

## 目的

修复百炼模型调用 `run_result_analysis` 时把自然语言 `intent` 生成为非法 JSON 或空缺字段，导致分析尚未进入应用服务就反复失败的问题。`intent` 当前只用于持久化和审计，不参与脚本执行、输入选择或输出校验，因此采用最简单、最明确的字符串合同。

## 设计

- `run_result_analysis.intent` 从任意 JSON 对象改为字符串。
- 工具 JSON Schema 明确声明字符串类型、用途说明和 1～1000 字符边界。
- 后端去除首尾空白，以 Unicode 字符数校验非空和上限；不合法时返回稳定领域错误，不启动分析。
- 合法值以 JSON 字符串形式写入既有 `result_analysis_runs.intent_json`，分析详情 API 继续通过原有 `intent` 字段返回，不修改数据库 schema 或公共路由。
- `result-analysis` skill 增加一份完整 `run_result_analysis` 参数示例，明确 `intent`、RSS `script`、唯一输入和 `expectedOutputs` 的关系。
- 更新 Agent 资源版本/摘要、工具合同测试、行为测试、功能目录和当前设计文档。

## 备选方案

- 单字段对象 `{"request":"..."}`：保留对象扩展性，但当前没有结构化消费方，会增加模型生成复杂度，未采用。
- 自动修补非法 JSON：可能误改用户意图或脚本，掩盖 provider 合同错误，未采用。
- 删除 `intent`：可以减少参数，但会丢失已有分析审计语义，未采用。

## 边界

- 不改变 `script`、`inputs`、`expectedOutputs` 的含义。
- 不接受 Python、SQL、服务器路径或任意结果行作为 `intent`。
- 不在本次修改中增加 provider 专用 JSON 修复或全局 Agent 重试策略。
- 确定性测试证明字符串参数可解析、对象参数被拒绝、空白/超长值失败关闭；真实百炼复验只在本地配置允许时执行，并与确定性门禁分开报告。
