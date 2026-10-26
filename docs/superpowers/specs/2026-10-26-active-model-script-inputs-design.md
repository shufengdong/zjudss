# Active 模型脚本输入设计

## 意图

Agent 为 active 模型生成研究脚本时，必须直接使用运行时注入的 DataFrame alias，不能再次调用
`import_dss`、其他导入函数或文件读取。可用 alias 以当前会话、当前 active 版本的实际表清单为准，不能在
Prompt 或 skill 中写死。

## 已确认的问题

`inspect_workspace` 已返回 active 版本的实际 `tables`，但 `rss-generation` 只笼统要求使用应用层 alias，
引擎文档又包含独立使用 tensoreval 时的 `import_dss` 示例，模型因此错误地重新导入。静态校验能识别
`script_external_dependency_rejected`，脚本服务却仍会保存这类无效 Agent 脚本。

## 方案

考虑过三种方案：只加强 Prompt 容易再次偏离；只在服务端拒绝会让 Agent 缺少正确信息并产生无效重试；
由服务端生成固定脚本会限制自然语言参数和后续研究类型。采用动态双层约束：规划结果冻结 active 版本的实际
alias，skill 指导 Agent 使用该清单，脚本服务在持久化前确定性核对。

## 行为

1. `inspect_workspace` 继续作为 active 表清单的首次来源；Agent 必须读取其 `active.tables[].tableName`。
2. `plan_study` 从同一次规划所读取的 active 版本生成 `inputAliases`，并把它写入 ready 计划及工具响应。
   该字段按表名稳定排序；规划器另从 adapter 的规范表合同中筛选本次 active 实际表，按调用顺序生成
   `modelInputOrder`。两者和 model version ID、content hash、engine snapshot 一起冻结。
3. 受管研究所需的规范输入若不在 active 表清单中，规划器返回 `missing_model_data` 并一次列全缺少的表，
   不签发 `studyPlanId`。
4. `rss-generation` 明确说明：直接引用 `inputAliases` 并按 `modelInputOrder` 传递规范模型输入，不得导入、读取
   路径或重新绑定这些输入 alias；可以用新变量派生参数表和保存函数结果。静态潮流不得为模型顺序再次查询文档，
   参数或结果语法最多使用一次搜索和一次章节读取。DataFrame 模式只包含变量声明，函数结果绑定中的叶变量直接
   成为输出，不在末尾追加裸变量表达式。
5. `create_script` 对携带 `studyPlanId` 的 Agent 脚本要求静态校验有效，并核对脚本所需输入均属于计划冻结的
   `inputAliases`。出现导入、文件读取、未知输入、覆盖输入或裸输出表达式时拒绝保存，并把可重试原因返回 Agent。
6. 手工上传和手工文本脚本维持现有兼容行为；运行仍由用户点击脚本卡片触发。

## 合同与验证

`plan_study` 工具响应包含 `inputAliases` 和 `modelInputOrder`；ready plan 的摘要/hash 同步包含这两个字段。
更新 Agent bundle/skill hash、Agent 设计文档、功能目录和行为测试。测试覆盖实际 active 表清单及动态顺序、
缺表规划、正确直用 alias 的纯声明脚本、带 `import_dss` 或末尾裸输出表达式的受管脚本拒绝保存，以及手工脚本
兼容性。
