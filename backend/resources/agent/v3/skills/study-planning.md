# 仿真方案规划

## 何时使用
用户要求静态潮流（power flow）、时序/QSTS、短路/故障（short circuit/fault）、谐波（harmonic）或暂态（transient）研究时使用，包括“计算潮流”这类短指令。

## 执行步骤
1. 先调用 `inspect_workspace`，了解当前会话是否有 active 模型和可用表；草稿、历史运行和案例不能替代 active 模型。
2. 从用户原话提取查询词。只有用户原文包含目录中的完整函数标识（例如 `ds_newton_raphson_pf`）时才原样传给 `search_study_facades.explicitFunction`；算法名称（如 Newton/牛顿法）和研究名称放在 `query` 中，`explicitFunction` 传 null。不得猜测函数标识或按拓扑替换用户选择。
3. 调用 `search_study_facades`。多个可用门面时，用 `title`、`description` 和 `differences` 一次说明差别并让用户选择；禁用门面必须显示其诊断。
4. 门面唯一后调用 `prepare_study`，完整传入用户意图。服务端从目录生成全部模型、Data、Struct 和结果卡片；工具成功后等待用户操作配置卡及最终确认，不生成脚本、不运行仿真。

## 参数语义
- 显式函数选择优先；Newton 适用于径向和非径向网络，不得因为 active 模型是径向结构而改成其他函数。
- 门面声明的 Data 输入用资源选择或列映射卡补齐；Struct 输入用带类型、单位、范围和枚举的参数卡补齐；结果只从门面声明中选择。
- 兼容资源唯一时服务端可预选；多个资源时必须由用户选择。同一门面的历史 Struct 参数只能先询问是否复用，不能静默沿用。

## 禁止推断
不得仅根据案例名选择方法，不得把历史参数静默带入新请求，不得在 Prompt 或 skill 中假定固定输入表清单，也不得用 `create_script` 绕过配置确认或自动运行脚本。

## 验收
门面候选分支只产生一次完整选择说明；配置卡必须与选定门面、当前 engine snapshot 和用户会话一致。
