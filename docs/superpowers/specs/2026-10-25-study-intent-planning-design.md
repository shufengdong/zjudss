# 多研究类型意图规划设计

状态：已批准

## 背景与目标

用户导入并激活配电网模型后，会用“计算潮流”“短路计算”“谐波分析”“做一天时序模拟”或“暂态仿真”等自然语言提出研究需求。当前 Agent 只依赖通用 Prompt、按需文档检索和自由工具选择，既可能拼错函数名，也无法稳定区分模型已经具备的网络数据与仍需用户提供的研究参数。

本设计增加一个只读、确定性的研究规划层。LLM 负责把自然语言归一化为受控研究请求；规划层读取当前 active 模型、当前引擎 catalog 和显式用户参数，返回统一的就绪状态、准确函数、求解器、模型证据和缺失项。只有就绪计划可以授权 Agent 创建相应研究脚本，实际运行仍由用户点击脚本卡片按钮发起。

本规格取代同日仅覆盖静态潮流的 `2026-10-25-power-flow-intent-reliability-design.md`。

## 已选方案

采用新增 `plan_study` 工具的方案，不使用纯 Prompt 路由，也不在主 Agent 前增加第二次 LLM 调用。

- LLM 擅长识别用户表达和填充结构化候选参数。
- 应用层负责模型范围、拓扑、设备、物理前置条件、引擎入口和结果名的确定性校验。
- `create_script` 继续负责脚本静态预检和持久化，但对受管研究函数要求一个当前请求内生成的 ready plan 证明。
- 不自动运行、不接收服务器路径、不从历史运行静默继承参数。

## 研究分类

受控研究类型为：

```text
static_power_flow
qsts
fault
harmonic
transient
  └── motor_dynamics
```

Agent 在 `study-planning` skill 中使用以下语义映射：

- 潮流、负荷流、稳态电压 → `static_power_flow`
- 时序、逐时、日曲线、QSTS、Daily、Yearly → `qsts`
- 短路、接地故障、相间故障 → `fault`
- 谐波、频谱、各次谐波 → `harmonic`
- 电机启动、堵转、电机故障切除动态 → `transient` + `motor_dynamics`
- 只有“暂态仿真” → `transient`，不猜测子类型

当前引擎支持的暂态能力仅为 `ds_motor_dynamics` 电机—网络动态，不把 EMT、发电机暂态稳定或逆变器控制暂态错误映射到该函数。

## 受控请求合同

`plan_study` 绑定当前用户、会话和请求消息，不接收路径、命令、脚本、SQL 或任意 JSON。参数采用 tagged enum：

```rust
enum StudyRequest {
    StaticPowerFlow {
        solver_preference: SolverPreference,
        requested_results: Vec<String>,
    },
    Qsts {
        solver_preference: SolverPreference,
        mode: Option<QstsMode>,
        start_seconds: Option<f64>,
        step_size_seconds: Option<f64>,
        number: Option<u32>,
        profile: Option<QstsProfile>,
        requested_results: Vec<String>,
    },
    Fault {
        cases: Option<Vec<FaultCase>>,
        allow_ideal_sources: Option<bool>,
        requested_results: Vec<String>,
    },
    Harmonic {
        h_max: Option<u32>,
        monitor_elements: Vec<String>,
        requested_results: Vec<String>,
    },
    Transient {
        subtype: Option<TransientSubtype>,
        motor_dynamics: Option<MotorDynamicsRequest>,
        requested_results: Vec<String>,
    },
}
```

所有数组、字符串和数值都有 schema 上限。QSTS 的 `profile` 必须显式表示常数倍率或有界的内联全局曲线；`None` 表示未提供，不能用默认曲线补齐。故障 case 使用 `bolted=true` 显式表示零阻抗，否则必须提供有限的阻抗分量；只有模型确实存在理想源时才要求用户通过 `allow_ideal_sources` 表态。电机动态以正的时长、步长和 `Option<Vec<Event>>` 表达，`Some([])` 明确表示无外部事件，`None` 表示用户尚未说明。

结果名不是物理关键参数：空列表使用产品默认；非空列表必须属于当前 `functions.json` 对应入口声明的结果集合。

## 模型能力摘要

规划只读取 current active version。存在草稿但没有 active 模型时返回 `missing_model_data`；绝不基于草稿生成可运行计划。

`StudyPlanningService` 以用户作用域取得 active version 的 21 张规范表，通过 ArtifactService 打开自有 Parquet，并在阻塞线程中按所需列流式扫描。它产生有界摘要：

```text
activeVersionId, contentHash
busCount, voltageSourceCount, generatorCount, ibrCount
motorCount, storageCount, spectrumCount
isConnected, hasLoops, hasParallelBranches
hasPvGenerator, hasGfm
faultSourceEquivalentCount, referencedSpectrumCount
```

拓扑是忽略相别的母线图，使用有效 line、简单/多绕组 transformer 和未完全断开的 switch 连接。重复的有效母线对标记并联支路；并查集和边计数确定连通性与环路。完整母线、端子、元件和频谱索引只保留在服务内部用于引用校验；工具输出只包含计数、布尔值和针对用户输入的少量建议，不把全网完整行返回给 LLM。

求解器选择规则：

1. 用户显式选择 Radial 或 Newton 时先验证模型兼容性；不兼容返回缺失/冲突说明。
2. 单固定电压源、连通、无环、无并联支路、无 `CONSTANT_PV` generator、无 GFM 时选择 Radial。
3. 环网、并联支路、多固定电压源、PV generator 或 GFM 时选择 Newton。
4. 读取失败、schema 不符合当前模型合同或 catalog 缺失不会降级为猜测，而是返回稳定工具错误。

## 研究就绪策略

### 静态潮流

- active 模型必须含非空 `bus` 和 `voltage_source`。
- “计算潮流”本身已经足够，自动选择求解器。
- 空结果列表使用 `bus_phase,summary`。
- 函数为 `ds_radial_pf` 或 `ds_newton_raphson_pf`，完整形式 arity 为 21。

### QSTS 时序模拟

- 除静态网络条件外，必须显式提供 mode、起始秒、步长、步数和 profile。
- profile 可以明确为常数负荷/发电倍率，或提供有界内联全局曲线；规划器不发明曲线。
- 缺少任一关键时间或曲线字段时返回 `needs_clarification`，一次列出全部缺失项。
- 函数为 `ds_radial_qsts` 或 `ds_newton_raphson_qsts`，生成完整 33 参数形式；未配置的控制、保护、监视和计量子系统使用合法零行表。

### 短路计算

- 至少一个 fault case；每个 case 必须包含已存在的母线、合法端子、故障类型，以及显式金属性选择或阻抗。
- 不默认三相、不默认母线、不默认零阻抗。
- 规划器检查电压源和发电机是否提供短路容量、序阻抗或受支持的短路等值；若模型包含理想源而用户未明确接受理想源语义，则一并追问，不能静默声称与 OpenDSS 短路容量模型等价。
- 函数为 `ds_fault_study`，构造显式 `fault` DataFrame，完整形式 arity 为 22。

### 谐波分析

- 模型 `spectrum` 必须非空，且至少一个注入设备实际引用有效频谱。
- 必须显式提供 `h_max`；监视对象若提供则必须能在模型中解析。
- 缺少模型频谱返回 `missing_model_data`，缺少 `h_max` 返回 `needs_clarification`。
- 函数为 `ds_harmonic_study`，完整形式 arity 为 22。

### 暂态与电机动态

- `transient` 未给 subtype 时返回 `needs_clarification`，说明当前支持范围。
- EMT、发电机暂态稳定、逆变器控制暂态返回 `unsupported`。
- `motor_dynamics` 要求模型有 motor、独立 storage 为空、正的时长和步长，并显式说明事件列表或无外部事件。
- 所有事件引用的母线、元件或保护设备必须存在。
- 函数为 `ds_motor_dynamics`，完整形式 arity 为 28。

## 规划结果与状态

返回状态固定为：

- `ready`：包含准确函数、求解器、规范化参数、模型证据、默认结果和 ready plan ID。
- `needs_clarification`：用户参数缺失、冲突或引用不存在；包含一次性追问所需的字段、原因和受控选项。
- `missing_model_data`：active 模型不存在或缺少该研究所需设备/数据。
- `unsupported`：用户目标超出当前引擎支持边界。

输出不包含服务器路径、完整母线清单或原始 Parquet 行。引用不存在时只返回用户提供的值和少量精确建议，避免放大上下文。

## Ready plan 证明与脚本创建

`plan_study` 只为 `ready` 结果生成请求内 opaque `studyPlanId`，并把 `ReadyStudyPlan` 存入 `ToolServices` 的共享内存注册表。证明绑定：

- request message ID；
- active version ID 与 content hash；
- engine resource snapshot ID；
- 规范化研究请求；
- 选定函数、arity 和结果名。

`create_script` 增加可选 `studyPlanId`。adapter 静态校验结果增加 `calledFunctions`。当 Agent 脚本调用 `ds_radial_pf`、`ds_newton_raphson_pf`、`ds_radial_qsts`、`ds_newton_raphson_qsts`、`ds_fault_study`、`ds_harmonic_study` 或 `ds_motor_dynamics` 时：

1. 必须提供当前请求生成的 ready plan ID；
2. 调用函数必须与计划一致；
3. active version content hash 和 engine snapshot 必须未变化；
4. 否则在持久化脚本前返回稳定错误。

不含受管研究函数的通用 RSS、用户手工上传脚本和已有脚本不要求该证明。成功创建的 Agent 脚本在现有 `agent_skill_refs_json` 中记录 `study-planning`、`rss-generation`、计划 hash、模型 hash 和函数，避免数据库迁移。

## Agent 工作流与追问行为

统一工作流为：

```text
inspect_workspace
→ load_agent_skill(study-planning)
→ plan_study
   ├─ ready
   │  → load_agent_skill(rss-generation)
   │  → search/read current engine docs
   │  → create_script(studyPlanId, ...)
   └─ 其他状态
      → 基于结构化 missingParameters 一次性追问或解释能力边界
```

除静态潮流外，关键物理参数缺失时统一先追问。追问应说明当前模型已确认的证据和全部缺失项，不逐项反复询问，也不使用案例、历史运行或常见默认值静默补齐。用户补充后重新调用 `plan_study`；旧计划不跨请求复用。

脚本创建成功只表示已保存并通过静态预检。Agent 必须提示用户检查脚本并点击“运行”，不得自动创建 SimulationRun。

## 21 张模型表边界

本设计不扩大现有“20 张网络表 + `para`”版本化模型合同。DSS execution 导入虽然能临时解析 35 张表，应用当前仍只发布 21 张静态模型表，原文件中的 `solution/load_shape/shape_assignment/fault/monitor` 等研究输入不会保留在模型版本中。

因此：

- 谐波可以复用第 17 张 `spectrum` 及设备引用；
- QSTS、短路和电机动态的专用表必须由已确认的受控请求在 RSS 中构造；
- 需要无损保留原 DSS 多个 `Solve`、控制和保护命令流时，应另行设计执行研究导入，不能让 Agent 使用服务器路径绕过模型合同。

## 错误处理

- 无 active 模型、缺 motor、缺 spectrum 等是正常规划结果，不作为工具异常。
- Parquet 不可读、schema 违反已发布模型合同、用户作用域失败或 catalog 结构损坏是 `DomainToolError`，返回稳定 code 和安全说明。
- catalog 不包含目标函数时返回 `unsupported` 并记录当前 snapshot ID，不编造旧函数。
- ready plan 已过期、函数不匹配和缺少证明分别返回稳定 `study_plan_stale`、`study_plan_function_mismatch`、`study_plan_required`。
- 所有 18 个 Agent 工具通过统一 Rig 适配器把 `DomainToolError` 映射成含稳定 `code`、安全 `message` 和 `retryable=false` 的模型反馈，不退化为 `the tool failed`。
- 不把路径、命令、SQL、密钥或未经筛选的底层错误反馈给模型。

## 版本、文档与测试

这是公共工具和 Agent 行为合同变更：工具从 17 个增加到 18 个，Agent bundle 升级为 `v3.2.0`。同步更新 manifest/hash、Prompt、skills、功能目录、Agent 设计、集成测试设计和开发状态。

确定性测试覆盖五类请求及参数边界、Radial/Newton 判定、active-only、四种状态、catalog 校验、ready plan 证明、脚本兼容性、准确函数调用、先追问行为和禁止自动运行。真实依赖 release lane 使用裸自然语言评测五类意图；普通 CI 使用 Scripted Agent 保持确定性。

## 非目标

- 不新增自动运行能力或新的前端固定仿真按钮。
- 不支持通用 EMT、发电机暂态稳定或任意逆变器控制暂态。
- 不保留或执行原始服务器 DSS 路径。
- 不在本次改动中扩展版本化模型到 35 张表。
- 不用提高全局 Agent 轮次上限掩盖规划错误。
