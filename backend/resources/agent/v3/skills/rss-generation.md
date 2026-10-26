# RSS 脚本生成

## 何时使用
任何需要生成或修改 RSS 的仿真、查询与比较操作前使用。

## 执行步骤
1. 对受管研究，以 `plan_study.inputAliases` 作为本次脚本唯一可用的模型输入清单，按 `plan_study.modelInputOrder` 放置规范模型输入。不得查询 active 模型输入顺序，也不得从示例或文档里的导入语句重建该顺序。
2. 运行时已经把 active 模型 DataFrame 注入为 `inputAliases` 中的同名变量；脚本直接引用这些变量。可以使用新变量派生参数表和承接结果。静态潮流的函数、arity、结果和模型输入顺序均已由 ready plan 给出；若仍需确认参数派生或结果选择语法，最多一次 `search_engine_docs` 和一次 `read_engine_doc_section`，随后立即生成并保存脚本。
3. DataFrame 脚本只能包含变量声明。函数结果绑定本身就是输出；最后一个或多个未被后续声明消费的 DataFrame 变量会由引擎写入结果目录。函数调用后不要追加 `summary_result;`、`voltage` 等裸变量表达式。`height`、`width` 等标量只能作为中间值被后续 DataFrame 表达式消费，禁止留下未被后续节点消费的纯标量。
4. 对 `plan_study` 规划的研究，脚本只调用计划给出的函数并满足其 arity 与结果合同；调用 `create_script` 时必须原样传递同一次 ready 结果中的 `studyPlanId`。不得复用历史请求的 ID。
5. 生成后调用 `create_script` 保存；静态预检和 ready plan 绑定校验由脚本服务完成，实际运行只由用户点击脚本卡片按钮发起。

## 禁止推断
不得手写未在 catalog 中出现的函数；不得用与 ready plan 不同的研究函数；不得调用 `import_dss` 或任何 `import_` 函数，不得进行网络访问、路径或其他文件读取，不得重新绑定 `plan_study.inputAliases` 中的输入 alias。

## 验收
受管脚本只直接读取 `plan_study.inputAliases`，规范模型参数顺序与 `plan_study.modelInputOrder` 一致，所有输出使用新变量名；保存实际脚本 artifact、catalog/技能版本、输入 hash 和结构化诊断。
