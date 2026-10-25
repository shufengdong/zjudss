# 仿真方案规划

## 何时使用
用户要求静态潮流（power flow）、时序/QSTS、短路/故障（short circuit/fault）、谐波（harmonic）或暂态（transient）研究时使用，包括“计算潮流”这类短指令。

## 执行步骤
1. 先调用 `inspect_workspace`，以当前会话 active 模型为唯一规划依据；草稿、历史运行和案例都不能替代 active 模型。
2. 将用户原话规范化成 `plan_study` 的一种 `studyType`：`static_power_flow`、`qsts`、`fault`、`harmonic` 或 `transient`。只填用户明确表达的物理参数；未说明的关键参数保留为空，不得补默认值。
3. 调用 `plan_study`。规划器会结合模型拓扑、设备、引用关系和本次引擎函数 catalog，返回 `ready`、`needs_clarification`、`missing_model_data` 或 `unsupported`。
4. 若状态为 `needs_clarification` 或 `missing_model_data`，在一条自然语言回复中覆盖全部 `missingParameters`；区分用户需要选择的参数和模型中需要补充的数据。追问后停止，不调用 `create_script`。
5. 若状态为 `unsupported`，说明当前不支持的研究类型及规划器给出的可行边界后停止。当前暂态只支持 `motor_dynamics`；EMT、暂态稳定和逆变器控制暂态不能改写成电机动态。
6. 只有状态为 `ready` 时才加载 `rss-generation`，依据当前引擎文档生成计划指定的唯一函数调用，并将返回的 `studyPlanId` 原样交给 `create_script`。
7. 脚本创建后等待用户在脚本卡片上明确点击运行。

## 参数语义
- 静态潮流允许 `solverPreference=auto`；具体径向或 Newton 选择由 active 模型事实和规划器决定。
- QSTS 必须明确模式、起始时间、步长、步数和负荷/发电曲线。
- 短路必须明确故障 case 的母线、端子、类型、金属性或阻抗，并处理理想源语义确认。
- 谐波必须明确最大谐波次数；模型必须存在注入设备实际引用的 spectrum。
- 电机动态必须明确持续时间、步长和事件；“无外部事件”规范化为显式空数组，未提事件则保持为空并追问。

## 禁止推断
不得仅根据案例名选择求解器，不得把历史运行参数静默带入新请求，不得自行构造 `studyPlanId`，也不得自动运行脚本。

## 验收
非 ready 分支只产生一次完整追问或边界说明；ready 脚本的函数、arity、结果和 `studyPlanId` 与同一次规划完全一致，且创建脚本后不产生运行或结果。
