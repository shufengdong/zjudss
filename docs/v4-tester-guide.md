# V4 测试人员须知

本文面向在已部署环境中进行黑盒验收、探索性测试和缺陷复测的测试人员。它说明怎样使用 V4、应重点验证
什么，以及发生问题时需要保留哪些证据。开发构建、自动化测试和服务器运维不在本文范围内；相关内容见
[V4 部署与发布](v4-deployment.md)和[集成测试与功能追溯](integration-testing-design.md)。

## 1. 开始测试前

### 1.1 确认环境

DellServer 的正式试用入口是 <https://sgool.zju.edu.cn/zjudss/>。其他环境以发布负责人提供的地址为准。
开始测试前请确认：

- 发布负责人已运行部署健康检查，并明确通知本轮可以开始测试；
- 已取得本轮分配的固定用户账号；当前试用环境的初始密码通常与用户 ID 相同，禁止在截图或缺陷单中记录密码；
- 已取得本轮测试基线，包括前端 Git hash、后端 Git hash、引擎资源 snapshot 和测试时间；
- 已说明本轮是否复用历史数据库。`scripts/deploy` 会保留数据库和数据目录，部署成功不代表环境已清空；
- 使用受支持的现代浏览器，建议使用最新版 Chrome 或 Edge，并允许 Cookie；
- 测试数据不含个人信息、密钥、真实生产数据或其他敏感内容。

前端 Git hash 可在浏览器开发者工具 Console 中查看：

```text
[version] zjudss frontend git = <hash>
```

后端 Git hash 由发布负责人从服务启动日志提供：

```text
[version] zjudss backend git = <hash>
```

系统会使用配置的 DeepSeek 或百炼服务理解自然语言、生成脚本和修正脚本。测试人员只能上传已经获准发送给该
外部模型服务的测试数据。缺陷单不得包含 API Key、Cookie、服务器物理路径或完整敏感模型。

### 1.2 理解 V4 的三个对象

| 对象 | 用途 | 关键规则 |
|---|---|---|
| 模型 | 母线、线路、设备等网络结构 | 先进入可变草稿，生成版本后成为不可变版本并自动激活 |
| 数据资源 | 负荷预测、曲线、频谱、计划、`para` 等 Data/Struct DataFrame | 独立于模型版本，可查看、下载、删除、重新上传；Struct 可通过参数卡修改 |
| 脚本与运行 | `.rss` 脚本、每次执行和结果数据集 | 脚本不可修改；每次执行生成独立运行记录，结果保存为 Parquet |

静态 DSS 导入通常产生 20 张模型表和一个普通 `para` 资源，但这不是所有研究的固定模型范围。实际模型表、
Data/Struct 输入、结果和可用函数均以本次启动加载的 eig-rs 动态目录为准。

### 1.3 主要交互原则

- 模型修改、模型激活、研究选择、结果分析和比较优先通过自然语言提出。
- 上传、案例浏览、参数确认、查看、下载、删除、运行、重新尝试和取消等高频无歧义操作使用固定按钮。
- Agent 必须先把研究方法、所需模型、数据资源、Struct 参数和结果集合收集完整；用户点击“确认并生成脚本”后，
  才能生成并执行脚本。
- 不要仅因为 Agent 的文字看起来合理就判定通过，应检查配置卡、运行记录和结果数据集是否与请求一致。

## 2. 十分钟冒烟测试

每次新部署至少完成以下流程。任一步无法继续，本轮冒烟测试即失败。

1. 打开入口并登录，确认 SGOOL 标识与“配电网仿真智能体”在登录卡同一行。
2. 确认系统打开已有会话，或自动创建标题为“新会话”的会话。
3. 打开“案例库”，选择一个状态有效的案例，查看详情中的研究类型、来源函数、目标门面和 projection profile。
4. 点击“使用此案例”。完整导入确认卡应只出现在“模型工作区”，对话区只保留进度摘要和定位入口。
5. 确认案例候选。模型候选进入可变草稿；Data/Struct 候选进入“数据文件”。`para` 不得进入模型草稿。
6. 展开草稿，查看至少一张模型表，填写简短版本描述并点击“生成版本”。成功后该版本应自动成为当前模型。
7. 在对话中输入与案例匹配的研究请求，例如“使用案例声明的方法计算，并采用推荐结果集合”。
8. 检查 Agent 是否先显示研究配置卡；补齐资源和参数后，核对摘要并点击“确认并生成脚本”。
9. 等待受管工作流结束。成功时应显示最终脚本和结果数据集列表；失败时应显示阶段、错误码和可读诊断，不能一直停留在
   “运行中”。
10. 展开运行记录，打开一个结果，验证表格分页、Parquet 下载和包含 list/matrix 的单元格显示正常。
11. 刷新页面，确认当前用户、会话、消息、模型、数据资源、运行和结果均可恢复。

数值结果是否正确应按测试用例给出的容差判断；没有金标准时，只能验收流程、schema、单位、结果名称和基本物理合理性，
不能仅以“执行成功”代替数值验收。

## 3. 完整业务测试

### 3.1 登录、会话与历史

- 正确账号可登录；错误账号或密码只显示通用错误，不能泄露账号是否存在。
- 刷新页面和重新打开浏览器后，未过期登录及最后会话应恢复。
- 第一条用户消息发送成功后，新会话标题应立即更新，不需要刷新。
- 会话列表整行都应容易点击，文字不会被鼠标拖成选择状态而阻止切换。
- 历史搜索命中较早消息时，应定位到连续历史窗口；有新消息时可“返回最新消息”。
- 删除会话必须先显示影响确认框。取消不删除；确认后删除相关模型、资源、脚本、运行、结果和分析数据。
- 删除最后一个会话后，系统应自动创建一个标题为“新会话”的可用空会话。
- 退出登录后业务数据应从页面清空；另一用户不能读取前一用户的会话和资源。

### 3.2 案例库导入

案例库来自本次引擎资源 snapshot 的 `dss/rssbundle` 目录，目录层级应与资源树一致。测试时检查：

- 顶层分类、嵌套目录、搜索、详情和窄屏“返回案例目录”均可使用；
- 有效案例显示“使用此案例”；无效案例显示诊断且不能导入；
- 使用案例后不会直接发布模型、生成脚本或开始仿真；
- 完整候选卡只出现在模型工作区，不在对话区重复展示；
- “全选”只选择可见、可发布的候选，不应误选隐藏 sheet 或 diagnostic；
- 模型、Data、Struct 和 diagnostic 按 manifest/profile 分类，diagnostic 不得成为运行输入；
- 确认后模型表进入草稿，Data/Struct 进入“数据文件”，两侧部分失败时可只重试失败侧；
- 导入完成后，模型表和数据资源均能查看，且案例仍可由用户继续修改后再研究。

当前发布基线有 144 个可导入案例，自动发布门禁必须逐个执行投影并达到 144/144、零失败。人工测试不要求逐个
执行 144 个案例，但至少应按当前案例覆盖的每种研究族抽取一个代表：

| 研究族 | 当前目标门面 | 当前案例数 |
|---|---|---:|
| Newton 潮流 | `ds_newton_raphson_pf` | 1 |
| 径向执行计划 | `ds_radial_execution_plan` | 121 |
| Newton 执行计划 | `ds_newton_raphson_execution_plan` | 10 |
| 短路研究 | `ds_fault_study` | 3 |
| 谐波执行计划 | `ds_harmonic_execution_plan` | 3 |
| 网络动态 | `ds_network_dynamics` | 6 |

上述数量是当前 snapshot 的证据，不是永久写死的产品合同。若本轮 snapshot 的数量变化，以发布报告为准。案例覆盖的
研究族不等于动态目录中的全部门面；对其他启用门面，应使用与目录 schema 兼容的模型和资源另行抽测。

### 3.3 手动上传与重新上传

支持的主要输入为 `.rss`、DSS/rssbundle/ZIP、CSV、Excel 和 Parquet。测试时检查：

- `.rss` 作为手写脚本导入，不进入模型/资源用途确认；创建后不会自动执行，点击“运行”才产生运行记录；
- DSS/rssbundle/ZIP/表格文件形成统一上传批次，CSV/Excel/Parquet 可按表头和目录 schema 推断用途；
- 推断结果必须由用户显式确认，可选择“模型表”“数据资源”或“跳过”，不能静默发布；
- Excel 多 sheet、隐藏 sheet、空表、错误列类型、重复列名和不支持扩展名均有明确处理结果；
- 文件传错时可放弃/跳过候选，再上传正确文件；不会污染已经确认的模型或资源；
- 重新上传数据资源时，旧资源在确认前仍可使用；确认卡提供“覆盖当前资源”“另存为新资源”和“取消替换”；
- 覆盖只把 alias 指向新资源，历史运行仍引用原 artifact，历史结果必须保持可读；
- 模型和资源两个子事务中一侧失败时，成功侧不回滚，也不能在重试时重复发布。

默认限制为单文件 50 MiB、单批次 500 MiB、暂存 24 小时；部署可覆盖这些值。超限、配额不足或暂存过期应显示
稳定错误，不能出现白屏、无限转圈或半发布状态。

### 3.4 模型工作区

- 当前模型、可变草稿和历史版本默认收起，并显示版本/修订、表数和总行数摘要。
- 三类模型中的每张表都可查看、选择列、分页加载和下载 Parquet；查看历史表不会激活历史版本。
- 草稿非空时才可“生成版本”；可填写不超过 200 个 Unicode 字符的简短描述。
- 生成版本后内容不可修改并自动激活；相同内容应复用已有版本及原描述。
- 激活历史版本若会丢弃草稿，必须二次确认。
- 当前激活版本不能删除；删除其他版本前必须显示会影响的运行和结果数量。
- 模型的领域修改应通过自然语言进入 Agent，不应出现让浏览器直接提交任意 SQL、路径或脚本命令的入口。

### 3.5 数据文件

- 列表显示名称、稳定 alias、schema、来源、行列数、大小、状态和创建时间。
- `para` 与负荷曲线等普通资源使用相同生命周期，不存在单独的“参数预设资源”。
- 资源支持查看、规范 Parquet 下载、在保留原文件时下载原文件、删除和重新上传。
- 只有当前 schema 明确为单行 Struct 的资源显示“修改参数”；普通 Data 表不显示该按钮。
- 删除前必须显示未确认配置、冻结工作流、分析和历史引用影响；删除后资源从当前列表隐藏，但历史运行和已物化派生结果
  仍应可读。
- alias 冲突、资源已被替换、schema 不匹配或内容过期时应给出明确诊断，不能静默选错资源。

### 3.6 自然语言研究与确认卡

当前 eig-rs 动态目录包含 18 个启用门面。下面的提示词用于逐门面验收，测试人员可以直接复制；其中反引号内的
资源名表示当前会话中应选择的兼容资源，不要求实际资源必须使用同名 alias。资源不存在时，正确行为是显示上传或选择卡，
测试人员补齐资源后继续，而不是 Agent 猜测数据、改用其他函数或提前生成脚本。

#### 静态潮流与 QSTS

1. `ds_radial_pf`

   ```text
   请严格使用 ds_radial_pf 对当前 active 模型执行径向静态潮流，参数通过 para 参数卡确认，结果选择推荐的 bus_voltage。不要改用 Newton 法，也不要重新 import_dss。
   ```

2. `ds_newton_raphson_pf`

   ```text
   请严格使用 ds_newton_raphson_pf 对当前 active 模型执行 Newton-Raphson 静态潮流，参数通过 para 参数卡确认，结果选择推荐的 bus_voltage。即使模型是径向网络也不要改用 ds_radial_pf。
   ```

3. `ds_radial_qsts`

   ```text
   请严格使用 ds_radial_qsts 对当前 active 模型执行径向 QSTS。使用已上传且 schema 兼容的 load_shape、tcc_curve 和 fault 数据资源，solution 与 para 通过参数卡确认，结果选择推荐的 bus_voltage；缺少资源时请让我上传或选择，不要改用其他函数。
   ```

4. `ds_newton_raphson_qsts`

   ```text
   请严格使用 ds_newton_raphson_qsts 对当前 active 模型执行 Newton-Raphson QSTS。使用已上传且 schema 兼容的 load_shape、tcc_curve 和 fault 数据资源，solution 与 para 通过参数卡确认，结果选择推荐的 bus_voltage；不要降级为径向 QSTS。
   ```

#### 谐波、短路与状态估计

5. `ds_harmonic_study`

   ```text
   请严格使用 ds_harmonic_study 对当前 active 模型执行谐波频率扫描，使用模型中已有的 Monitor 等门面所需表，通过 para 参数卡确认扫描和结果设置，结果选择推荐的 bus_voltage。不要改用 harmonic execution plan。
   ```

6. `ds_fault_study`

   ```text
   请严格使用 ds_fault_study 对当前 active 模型执行短路研究，故障工况使用已上传且 schema 兼容的 fault 数据资源，para 通过参数卡确认，结果选择推荐的 fault；缺少故障数据时先要求上传或选择。
   ```

7. `ds_wls_se`

   ```text
   请严格使用 ds_wls_se 对当前 active 模型执行加权最小二乘状态估计，量测使用已上传且 schema 兼容的 measurement 数据资源，para 通过参数卡确认，结果选择推荐的 result。不要改用其他状态估计算法。
   ```

8. `ds_mnmr_se`

   ```text
   请严格使用 ds_mnmr_se 对当前 active 模型执行最大正常率鲁棒状态估计，量测使用已上传且 schema 兼容的 measurement 数据资源，para 通过参数卡确认，结果选择推荐的 result。不要改用 WLS 或 MCE。
   ```

9. `ds_mce_se`

   ```text
   请严格使用 ds_mce_se 对当前 active 模型执行最小交叉熵鲁棒状态估计，量测使用已上传且 schema 兼容的 measurement 数据资源，para 通过参数卡确认，结果选择推荐的 result。不要改用 WLS 或 MNMR。
   ```

#### 随机分析、恢复、保护与故障定位

10. `ds_monte_carlo`

    ```text
    请严格使用 ds_monte_carlo 对当前 active 模型执行 Monte Carlo 研究，使用已上传且 schema 兼容的 multipliers 数据资源，spec 和 para 通过参数卡确认，结果选择推荐的 voltage_statistics；不要替换为普通潮流。
    ```

11. `ds_restore`

    ```text
    请严格使用 ds_restore 对当前 active 模型执行故障隔离与负荷恢复，故障集合使用已上传且 schema 兼容的 fault 数据资源，para 通过参数卡确认，结果选择推荐的 switches；不要只运行短路研究。
    ```

12. `ds_protection_coordination`

    ```text
    请严格使用 ds_protection_coordination 执行保护配合审计。使用已上传且 schema 兼容的 tcc_curve、fault_case、current_sample、pair 和 observed_action 数据资源，para 通过参数卡确认，结果选择推荐的 operations；任一资源缺失时先要求补齐。
    ```

13. `ds_fault_location`

    ```text
    请严格使用 ds_fault_location 执行故障定位。使用已上传且 schema 兼容的 measurement、hypothesis_switch 和 hypothesis_line 数据资源，para 通过参数卡确认，结果选择推荐的 result；不要改为 ds_fault_study。
    ```

#### 动态研究

14. `ds_network_dynamics`

    ```text
    请严格使用 ds_network_dynamics 对当前 active 模型执行完整网络动态研究，使用已上传且 schema 兼容的 timeline、event、tcc_curve 和 fault 数据资源，spec 与 para 通过参数卡确认，结果选择推荐的 bus_voltage；不要改用电机专用旧版门面。
    ```

15. `ds_motor_dynamics`

    ```text
    请严格使用 ds_motor_dynamics 对当前 active 模型执行以电机为中心的 RMS/phasor 动态研究，使用已上传且 schema 兼容的 timeline、event 和 tcc_curve 数据资源，spec 与 para 通过参数卡确认，结果选择推荐的 motor_state；不要改用完整网络动态门面。
    ```

#### 路径无关执行计划

16. `ds_radial_execution_plan`

    ```text
    请严格使用 ds_radial_execution_plan 重放当前案例的径向执行计划。使用案例导入产生且 schema 兼容的 solution、load_shape、tcc_curve、fault、execution_epoch、execution_setting、execution_mutation、post_solve_action、execution_reference、execution_event 和 projection_diagnostic 资源，para 通过参数卡确认，结果选择推荐的 bus_voltage；不要改用 Newton 执行计划。
    ```

17. `ds_newton_raphson_execution_plan`

    ```text
    请严格使用 ds_newton_raphson_execution_plan 重放当前案例的 Newton-Raphson 执行计划。使用案例导入产生且 schema 兼容的 solution、load_shape、tcc_curve、fault、execution_epoch、execution_setting、execution_mutation、post_solve_action、execution_reference、execution_event 和 projection_diagnostic 资源，para 通过参数卡确认，结果选择推荐的 bus_voltage；不要改用径向执行计划。
    ```

18. `ds_harmonic_execution_plan`

    ```text
    请严格使用 ds_harmonic_execution_plan 执行当前 Run_Scan 类案例的整数或非整数频率扫描。使用案例导入产生且 schema 兼容的 load_shape、tcc_curve、fault、frequency_axis、frequency_spectrum 和 harmonic_snapshot 数据资源，solution、harmonic_plan 与 para 通过参数卡确认，并使用门面要求的当前模型和谐波快照模型表，结果选择推荐的 bus_voltage；不要退回路径型 import 或 ds_harmonic_study。
    ```

#### 144 个发布案例对应的提示词

案例与提示词的对应关系以各案例 `manifest.json` 中的 `projection.target_function` 为准。当前 144 个案例只覆盖
6 个门面，其余 12 个门面没有发布案例可直接导入：

| 提示词 | 目标门面 | 案例数 |
|---:|---|---:|
| 2 | `ds_newton_raphson_pf` | 1 |
| 6 | `ds_fault_study` | 3 |
| 14 | `ds_network_dynamics` | 6 |
| 16 | `ds_radial_execution_plan` | 121 |
| 17 | `ds_newton_raphson_execution_plan` | 10 |
| 18 | `ds_harmonic_execution_plan` | 3 |

测试人员应先导入下列案例并完成模型/资源确认，再使用其所在分组标注的提示词。路径从案例库根目录开始；同名案例
必须用完整相对路径区分。

<details>
<summary>提示词 2：ds_newton_raphson_pf（1 个）</summary>

```text
IEEETestCases/IEEE 30 Bus/Run_IEEE30.rssbundle
```

</details>

<details>
<summary>提示词 6：ds_fault_study（3 个）</summary>

```text
IEEETestCases/ShortCircuitCases/IEEE123Master-SC.rssbundle
IEEETestCases/ShortCircuitCases/ieee34Mod2_SC_Case_II.rssbundle
IEEETestCases/ShortCircuitCases/ieee37_SC_Currents.rssbundle
```

</details>

<details>
<summary>提示词 14：ds_network_dynamics（6 个）</summary>

```text
Examples/DistanceRelays/DistanceRelayTest.rssbundle
Examples/DistanceRelays/ReverseDistanceRelayTest.rssbundle
Examples/DistanceRelays/ReverseTD21RelayTest.rssbundle
Examples/DistanceRelays/TD21RelayTest.rssbundle
Examples/InductionMachine/Run.rssbundle
Examples/Scripts/59NRelayDemo.rssbundle
```

</details>

<details>
<summary>提示词 16：ds_radial_execution_plan（121 个）</summary>

```text
EPRITestCircuits/epri_dpv/J1/Master_noPV.rssbundle
EPRITestCircuits/epri_dpv/J1/Master_withPV.rssbundle
Examples/InverterModels/PVSystem/InvControl/DRC/Daily_DRC-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/DRC/Daily_DRC_avg-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/DRC/Daily_DRC_kVAlimitation_P-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/DRC/Daily_DRC_kVAlimitation_Q-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/DRC/Daily_DRC_kvarlimitation-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/MonitoredVoltage/Local_voltage_2-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/MonitoredVoltage/Local_voltage_average-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/MonitoredVoltage/Local_voltage_max-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/MonitoredVoltage/Local_voltage_min-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/MonitoredVoltage/Mon_voltage_MAX_Mix-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/MonitoredVoltage/Mon_voltage_MAX_Mix_avg-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/MonitoredVoltage/Mon_voltage_MAX_Mix_avg_VVDRC-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/MonitoredVoltage/Mon_voltage_MAX_Mix_ravg-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/MonitoredVoltage/Mon_voltage_average-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/MonitoredVoltage/Mon_voltage_average_LL-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/MonitoredVoltage/Mon_voltage_average_Mix-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_DRC/Daily_VVDRC-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_DRC/Daily_VVDRC_avg-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_DRC/Daily_VVDRC_kVAlimitation_P-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_DRC/Daily_VVDRC_kVAlimitation_Q-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_DRC/Daily_VVDRC_kvarlimitation-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_DRC/Daily_VVDRC_ravg-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_VW/Daily_VVVW_VWnooperation-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_VW/Daily_VVVW_varAval_PMPPPU_kvarlimit-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_VW/Daily_VVVW_varMax_PMPPPU-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_VW/Daily_VVVW_varMax_PMPPPU_kVAlimitation_Ppriority-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_VW/Daily_VVVW_varMax_PMPPPU_kVAlimitation_Qpriority-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_VW/Daily_VVVW_varMax_PMPPPU_pctPmpp60-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_VW/Daily_voltvar_pctPmpp60-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_VW/SnapShot_VVVW_VWnooperation-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_VW/SnapShot_VVVW_varAval_PMPPPU_kvarlimit-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_VW/SnapShot_VVVW_varMax_PMPPPU-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_VW/SnapShot_VVVW_varMax_PMPPPU_kVAlimitation_Ppriority-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_VW/SnapShot_VVVW_varMax_PMPPPU_kVAlimitation_Qpriority-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/VV_VW/SnapShot_VVVW_varMax_PMPPPU_pctPmpp60-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-var/Daily_voltvar_Standard-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-var/Daily_voltvar_avg-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-var/Daily_voltvar_greater_kVA_ppriority-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-var/Daily_voltvar_greater_kVA_qpriority-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-var/Daily_voltvar_kvarlimit-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-var/Daily_voltvar_pctPmpp60-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-var/Daily_voltvar_ravg-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-var/SnapShot_voltvar_Standard-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-var/SnapShot_voltvar_Standard_varaval-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-var/SnapShot_voltvar_Standard_varaval_kvarlimitation-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-var/SnapShot_voltvar_greater_kvA_ppriority-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-var/SnapShot_voltvar_greater_kvA_qpriority-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-var/SnapShot_voltvar_pctPmpp60-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/Daily_voltwatt_PAVAILABLEPU-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/Daily_voltwatt_PAVAILABLEPU_pctPmpp60-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/Daily_voltwatt_PCTPMPPPU-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/Daily_voltwatt_PMPPPU-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/Daily_voltwatt_PMPPPU_PF-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/Daily_voltwatt_PMPPPU_PFP_kVAlimitation-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/Daily_voltwatt_PMPPPU_PFP_kvarlimitation-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/Daily_voltwatt_PMPPPU_pctPmpp60-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/Daily_voltwatt_PMPPPU_pctPmpp60_PF-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/Daily_voltwatt_PMPPPU_wattP_kVAlimitation-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/Daily_voltwatt_noOperation-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/Daily_voltwatt_noOperation_PF-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_KVARATINGPU-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_PAVAILABLEPU-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_PAVAILABLEPU_PF-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_PAVAILABLEPU_pctPmpp60-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_PCTPMPPPU-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_PCTPMPPPU_PF-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_PMPPPU-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_PMPPPU_PF-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_PMPPPU_PFP_kVAlimitation-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_PMPPPU_PFP_kvarlimitation-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_PMPPPU_pctPmpp60-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_PMPPPU_pctPmpp60_PF-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_PMPPPU_pmpp_greater_kva-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_PMPPPU_varP_kVAlimitation-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_PMPPPU_wattP_kVAlimitation-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_noOperation-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_noOperation_PF-2.rssbundle
Examples/InverterModels/PVSystem/InvControl/volt-watt/SnapShot_voltwatt_pf-2.rssbundle
Examples/InverterModels/PVSystem/NewFeatures/ConstantPF/SnapShot_PFP_Standard-PV2.rssbundle
Examples/InverterModels/PVSystem/NewFeatures/ConstantPF/SnapShot_PFP_kVAlimitation-PV2.rssbundle
Examples/InverterModels/PVSystem/NewFeatures/ConstantPF/SnapShot_PFP_kvarlimitation-PV2.rssbundle
Examples/InverterModels/PVSystem/NewFeatures/ConstantPF/SnapShot_PFP_pctPmpplimitation-PV2.rssbundle
Examples/InverterTechNote/kVA/11_1_kVAViolation.rssbundle
Examples/Scripts/IEEE-TIA-LV Model/Split-Phase_IEEE_TIA.rssbundle
Examples/Scripts/IEEE-TIA-LV Model/master_large.rssbundle
Examples/Scripts/IEEE-TIA-LV Model/master_large_1phase.rssbundle
Examples/Scripts/IEEE-TIA-LV Model/master_small.rssbundle
Examples/Scripts/IEEE-TIA-LV Model/master_small_1phase.rssbundle
Examples/StorageControllerTechNote/Follow/FollowRun.rssbundle
Examples/StorageControllerTechNote/LoadShape/LoadShapeRun.rssbundle
Examples/StorageControllerTechNote/PeakShave/PeakShaveMonPhaseRun.rssbundle
Examples/StorageControllerTechNote/PeakShave/PeakShaveRun.rssbundle
Examples/StorageControllerTechNote/PeakShaveDch_PeakShaveLow_Ch/PeakShaveDch_PeakShaveLow_ChRun.rssbundle
Examples/StorageControllerTechNote/Schedule/ScheduleRun.rssbundle
Examples/StorageControllerTechNote/Support/SupportRun.rssbundle
Examples/StorageControllerTechNote/Time/TimeRun.rssbundle
Examples/StorageTechNote/Example_9_1_Default/Storage_default.rssbundle
Examples/StorageTechNote/Example_9_2_Follow/Storage_follow.rssbundle
Examples/StorageTechNote/Example_9_3_Price/Storage_price.rssbundle
Examples/StorageTechNote/Example_9_4_External/Storage_external.rssbundle
Examples/StorageTechNote/Example_9_5_ReactivePower/ConstantPF_9_5_2/Storage_follow_PF.rssbundle
Examples/StorageTechNote/Example_9_5_ReactivePower/Constantkvar_9_5_1/Storage_follow_kvar.rssbundle
Examples/VSConverter/SG2012.rssbundle
Examples/WampServer/testcommandline.rssbundle
IEEETestCases/123Bus/P174_Run_SolarRamp.rssbundle
IEEETestCases/123Bus/P174_Run_VoltageProfile.rssbundle
IEEETestCases/123Bus/Run_IEEE123Bus.rssbundle
IEEETestCases/123Bus/SolarRamp-Mack.rssbundle
IEEETestCases/123Bus/SolarRamp.rssbundle
IEEETestCases/13Bus/IEEE13Nodeckt.rssbundle
IEEETestCases/34Bus/Run_IEEE34Mod1.rssbundle
IEEETestCases/34Bus/Run_IEEE34Mod2.rssbundle
IEEETestCases/37Bus/ieee37.rssbundle
IEEETestCases/4Bus-DY-Bal/4Bus-DY-Bal.rssbundle
IEEETestCases/4Bus-GrdYD-Bal/4Bus-GrdYD-Bal.rssbundle
IEEETestCases/4Bus-OYOD-Bal/4Bus-OYOD-Bal.rssbundle
IEEETestCases/4Bus-YD-Bal/4Bus-YD-Bal.rssbundle
IEEETestCases/4Bus-YY-Bal/4Bus-YY-Bal.rssbundle
IEEETestCases/LVTestCase/Master.rssbundle
```

</details>

<details>
<summary>提示词 17：ds_newton_raphson_execution_plan（10 个）</summary>

```text
Examples/Microgrid/GridFormingInverter/GFM_IEEE123/Run_IEEE123Bus_GFMSnap-A.rssbundle
Examples/Microgrid/GridFormingInverter/GFM_IEEE8500/Run_8500Node_Unbal.rssbundle
Examples/Microgrid/GridFormingInverter/GFM_IEEE8500/Run_RecloserSiting.rssbundle
Examples/StoCtrl_Current_PeakShave/master.rssbundle
IEEETestCases/123Bus/RevRegTest.rssbundle
IEEETestCases/4Bus-YYD/YYD-Master-step1.rssbundle
IEEETestCases/8500-Node/P174_Run_Voltage_Profile.rssbundle
IEEETestCases/8500-Node/Run_8500Node.rssbundle
IEEETestCases/8500-Node/Run_8500Node_Unbal.rssbundle
IEEETestCases/8500-Node/Run_RecloserSiting.rssbundle
```

</details>

<details>
<summary>提示词 18：ds_harmonic_execution_plan（3 个）</summary>

```text
Examples/FreqScan/Run_Scan.rssbundle
Examples/HarmonicsTMode/IEEE_519.rssbundle
Examples/HarmonicsVariableLoad/IEEE_519.rssbundle
```

</details>

提示词 1、3、4、5、7、8、9、10、11、12、13 和 15 当前没有发布案例直接对应。测试这些门面时必须另行准备符合
当前目录 schema 的 active 模型和 Data/Struct 资源，不能拿上述案例强行套用，也不能把“无案例”误报为门面不可用。

还应补充一轮不指定函数名的自然语言测试，例如“计算当前模型的潮流”或“对 Run_Scan 案例做谐波扫描”。当多个门面
都合理时，Agent 应显示方法差异并让用户选择；这轮测试不能替代上面 18 条显式函数测试。

应重点检查：

- 用户明确指定 Newton 等方法时，Agent 不得因为拓扑而擅自替换为其他算法；
- 有多个可用函数时，应告诉用户差异并要求选择；只有一个明确匹配时可以自动预选；
- Agent 查询当前 active 模型实际存在的表和当前数据资源，不得写死固定 20/21 张表，也不得生成 `import_dss(...)`
  重新读取已经导入的模型；
- 缺少 Data 输入时要求上传或从已上传资源中选择；多个兼容资源不得猜选；
- DataFrame 表示连续/批量数据时通过文件资源提供；表示 Struct 参数时使用类型化参数卡；
- 参数卡按 bool、number、enum、string、date、单位、默认值和范围显示，非法值不能进入最终确认；
- 同一会话再次进行同门面研究时，系统可以询问是否复用上次 Struct 参数，但不能静默复用；
- 修改已有 Struct 时必须选择覆盖或另存，保存卡片草稿不能提前改写资源；
- 结果集合必须让用户确认；Agent 不应在用户确认前生成或执行脚本；
- 只有全部输入完整时才显示“确认并生成脚本”，自然语言中的“确认”不能代替点击该按钮。

### 3.7 生成、执行、修正和重跑

受管研究与手写脚本的行为不同：

| 场景 | 首次执行 | 失败处理 | 后续按钮 |
|---|---|---|---|
| 确认卡生成的受管脚本 | 确认后自动生成并立即执行 | 对可修正错误自动修正，默认最多 3 次修正、最多 4 次引擎执行 | 成功脚本显示“运行”；失败流程显示“重新尝试” |
| 手写、粘贴或上传的 `.rss` | 创建后不执行，用户点击“运行” | 单次执行，直接显示错误，不自动修正 | 继续使用“运行” |
| 成功 Agent 脚本再次运行 | 用户点击“运行” | 使用当前最新所需资源，失败直接显示错误，不自动修正 | “运行” |

每次受管执行应显示当前阶段、attempt 序号和耗时；每轮脚本和错误默认折叠。失败记录应显示服务端清洗后的错误码、
原因、退出码和 tensoreval 输出，服务器绝对路径不得出现在界面。超时、取消、配额、路径安全和基础设施错误不会进入
自动修正。点击“取消”后必须停止当前及后续执行，保留诊断并允许后续“重新尝试”。

“重新尝试”会比较原冻结输入与当前输入。输入未变化可直接派生新流程；发生变化时必须先展示差异并再次确认。服务重启
期间的非终态工作流应收敛为明确失败或按恢复合同重新排队，不能永久显示“运行中”。

### 3.8 运行记录、结果、分析与比较

- 运行记录默认收起，显示中文研究标题、状态、脚本名、模型版本、创建时间和耗时。
- 排队或运行中的记录显示阶段，但不提前请求结果。
- 成功记录展开后列出全部存活结果数据集；只显示数据集列表，不自动打开第一个结果。
- 点击运行记录主体应定位到同一会话中对应的脚本消息；“详情”是独立按钮。
- 结果表支持列投影、分页、加载更多和 Parquet 下载；字符串、`LargeList`、matrix 等类型应正确显示。
- 图表由自然语言分析请求产生，只接受服务端校验的受控视图 spec；前端不得执行任意 ECharts option 或脚本。
- 结果分析、物化视图和比较必须使用稳定资源 ID；来源结果后来删除时，已物化的派生结果仍应可解释、可读取。
- 删除单个结果前应确认；删除后运行和兄弟结果仍保留。
- 只有终态运行可删除。删除前应显示结果数量、空间及派生/提升引用影响；确认后该运行及其全部直接结果消失。
- 取消删除不应产生任何变化；影响已变化时应要求重新确认。

## 4. 刷新、并发与恢复

以下情况至少各测试一次：

- 上传解析中刷新页面：批次和进度能够恢复，不重复发布；
- 配置卡填写后刷新：revision、已选资源和参数草稿能够恢复；
- 工作流执行中刷新：只出现一个工作流和一组消息，不重复生成脚本或运行；
- 运行成功后刷新：脚本“运行”可用，结果列表和运行记录一致；
- 切换会话：旧会话上传队列、资源页和结果面板不能泄漏到新会话；
- 两个浏览器同时修改同一配置：旧 revision 返回 `configuration_stale` 并刷新卡片，不合并旧输入；
- 同一会话启动第二个非终态工作流：返回 `workflow_in_progress`，不能并发覆盖；
- 删除或替换恰逢执行：冻结中的运行继续使用确认时输入，后续运行才使用最新 alias。

默认每个用户最多同时执行 2 个 Agent 工作、每个会话最多 1 个非终态受管工作流；排队不一定是缺陷。实际限制可由
部署配置覆盖。

## 5. 错误归类

测试人员应记录界面显示的稳定错误码和中文原因，不要只写“失败了”。常见处理方式如下：

| 现象或错误码 | 优先判断 | 测试人员动作 |
|---|---|---|
| 缺少模型、资源或参数 | 测试输入未完整 | 按配置卡上传/选择资源或补齐参数 |
| `configuration_stale` / `resource_stale` | 页面数据已过期 | 刷新卡片或资源后重新确认；若循环出现则报缺陷 |
| `workflow_in_progress` | 同会话已有任务 | 等待或取消当前任务后重试 |
| `resource_alias_conflict` | alias 已存在 | 选择另存或通过自然语言改名 |
| `model_import_unsupported` | 当前文件无法完整投影 | 核对文件/profile；有效案例仍出现则报引擎合同缺陷 |
| `engine_timeout` | 引擎超时 | 保存运行 ID 和诊断，交由发布/引擎负责人分析 |
| `engine_contract_mismatch` | binary 与资源目录不兼容 | 停止本轮研究测试，通知发布负责人 |
| `engine_resource_hash_mismatch` | 已发布资源 snapshot 损坏 | 停止测试，通知发布负责人重启或重新部署 |
| `run_busy` / `result_busy` | 正被执行或派生任务占用 | 等待任务终态后重试删除 |
| HTTP 401 或返回登录页 | 登录过期或已退出 | 重新登录；若频繁发生，记录发生时间和网络状态 |
| 一直“运行中”但引擎已失败 | 产品缺陷 | 立即记录运行 ID、时间、阶段、Console 和网络请求 |
| 白屏、控件无法点击、确认框缺失 | 前端缺陷 | 保留截图、Console、Network HAR 和复现步骤 |

服务端返回的服务器绝对路径、密钥、Cookie、完整 prompt 或其他用户数据若出现在普通界面或下载内容中，按安全缺陷
处理，不要继续转发这些内容。

## 6. 验收结论

一次完整验收至少需要满足：

- 十分钟冒烟测试全部通过；
- 当前案例 snapshot 的自动投影报告为全部成功、零失败；
- 六个当前案例研究族各有代表案例通过导入，计划覆盖的研究族完成自然语言到结果的全链路；
- 明确函数选择、缺失资源、Struct 参数卡、确认后执行、自动修正、再次运行和失败诊断均符合合同；
- 刷新恢复、会话/运行/结果删除、历史脚本定位和用户隔离通过；
- 没有 P0/P1 缺陷，其他遗留问题已记录影响范围和规避方式；
- 验收报告保存版本与 snapshot 证据，但不保存 secret、密码、Cookie、物理路径、原始 stderr 或完整脚本正文。

自动化的 100% 功能追溯与人工业务验收是互补关系：自动化门禁通过不能替代人工交互和数值合理性检查，人工抽测也不能
替代 144 个案例投影、公共合同和回归测试门禁。
