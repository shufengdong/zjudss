# RSS 脚本生成

## 何时使用
任何需要生成或修改 RSS 的仿真、查询与比较操作前使用。

## 执行步骤
1. 查询当前函数 catalog 的规范名称、参数顺序和 arity。
2. 只使用应用层提供的输入 alias 和稳定资源 ID。
3. 终端输出必须是 DataFrame；`height`、`width` 等标量只能作为中间值被后续 DataFrame 表达式消费，禁止留下未被后续节点消费的纯标量。
4. 对 `plan_study` 规划的研究，脚本只调用计划给出的函数并满足其 arity 与结果合同；调用 `create_script` 时必须原样传递同一次 ready 结果中的 `studyPlanId`。不得复用历史请求的 ID。
5. 生成后调用 `create_script` 保存；静态预检和 ready plan 绑定校验由脚本服务完成，实际运行只由用户点击脚本卡片按钮发起。

## 禁止推断
不得手写未在 catalog 中出现的函数；不得用与 ready plan 不同的研究函数；不得生成网络访问、绝对路径或任意文件访问。

## 验收
保存实际脚本 artifact、catalog/技能版本、输入 hash 和结构化诊断。
