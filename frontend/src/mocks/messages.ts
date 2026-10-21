import type { Message } from '../types'

const now = Date.now()
const MIN = 60 * 1000
const ago = (minutes: number) => new Date(now - minutes * MIN).toISOString()

// 完整示例（s1，已完成）：覆盖全部 6 种消息类型。
export const demoMessages: Message[] = [
  {
    id: 'm1',
    type: 'user',
    timestamp: ago(26),
    content: '请检查这条配电网馈线的模型，然后跑一次 24 小时时序仿真，重点看看晚高峰电压有没有越限。',
  },
  {
    id: 'm2',
    type: 'attachment',
    timestamp: ago(25),
    file: { fileName: '城区配电网模型.case', fileSize: '2.4 MB', fileType: 'case' },
  },
  {
    id: 'm3',
    type: 'tool-call',
    timestamp: ago(24),
    toolName: 'inspect_model',
    args: { model: '城区配电网模型.case' },
    status: 'success',
    summary: 'model ok',
  },
  {
    id: 'm4',
    type: 'tool-call',
    timestamp: ago(23),
    toolName: 'run_simulation',
    args: { model: '城区配电网模型.case', duration: '24h', step: '15min' },
    status: 'success',
    summary: 'task_id: sim_20261021_001',
  },
  {
    id: 'm5',
    type: 'progress',
    timestamp: ago(21),
    taskId: 'sim_20261021_001',
    stages: [
      { key: 'inspect', label: '模型检查', status: 'done' },
      { key: 'sim', label: '时序仿真', status: 'done' },
      { key: 'analyze', label: '结果分析', status: 'done' },
      { key: 'report', label: '报告生成', status: 'done' },
    ],
  },
  {
    id: 'm6',
    type: 'assistant',
    timestamp: ago(19),
    content: `### 仿真结论

24 小时时序仿真已完成，主要发现如下：

- 馈线 A 在晚高峰（18:00–21:00）出现电压跌落，最低 **0.941 p.u.**，低于 0.95 p.u. 的限值。
- 馈线 B 电压整体正常，未出现越限。
- 越限时段集中在光伏出力退出后的晚高峰。

| 指标 | 数值 | 限值 |
| --- | --- | --- |
| 最大电压 | 1.082 p.u. | ≤ 1.10 |
| 最小电压 | 0.941 p.u. | ≥ 0.95 |
| 越限节点 | 3 | 0 |

\`\`\`text
[分析] 晚高峰负荷 + 光伏退坡 → 末端节点电压跌落
建议：在节点 N-23 增配无功补偿 / 调整变压器分接头
\`\`\``,
  },
  {
    id: 'm7',
    type: 'result',
    timestamp: ago(18),
    taskId: 'sim_20261021_001',
    title: '城区配电网 24 小时时序仿真结果',
    metrics: { maxVoltage: 1.082, minVoltage: 0.941, violationNodes: 3, violationDuration: '2.5 h' },
  },
]

// 运行中（s2）：工具运行中 + 进度卡当前阶段高亮。
export const runningMessages: Message[] = [
  {
    id: 'r1',
    type: 'user',
    timestamp: ago(8),
    content: '对接入分布式光伏后的馈线跑一次时序仿真。',
  },
  {
    id: 'r2',
    type: 'tool-call',
    timestamp: ago(7),
    toolName: 'inspect_model',
    args: { model: '光伏接入模型.case' },
    status: 'success',
    summary: 'model ok',
  },
  {
    id: 'r3',
    type: 'tool-call',
    timestamp: ago(2),
    toolName: 'run_simulation',
    args: { model: '光伏接入模型.case', duration: '24h', step: '15min' },
    status: 'running',
    summary: 'task_id: sim_20261021_002',
  },
  {
    id: 'r4',
    type: 'progress',
    timestamp: ago(1),
    taskId: 'sim_20261021_002',
    stages: [
      { key: 'inspect', label: '模型检查', status: 'done' },
      { key: 'sim', label: '时序仿真', status: 'running' },
      { key: 'analyze', label: '结果分析', status: 'pending' },
      { key: 'report', label: '报告生成', status: 'pending' },
    ],
  },
]

// 失败（s4）：工具失败 + 进度卡失败标红。
export const failedMessages: Message[] = [
  {
    id: 'f1',
    type: 'user',
    timestamp: ago(30 * 60),
    content: '春节高峰负荷下的馈线 N-1 校验。',
  },
  {
    id: 'f2',
    type: 'tool-call',
    timestamp: ago(30 * 60 - 1),
    toolName: 'inspect_model',
    args: { model: '高峰负荷模型.case' },
    status: 'success',
    summary: 'model ok',
  },
  {
    id: 'f3',
    type: 'tool-call',
    timestamp: ago(30 * 60 - 2),
    toolName: 'run_simulation',
    args: { model: '高峰负荷模型.case', scenario: 'N-1' },
    status: 'failed',
    summary: 'ERR_SOLVER_DIVERGED：潮流计算不收敛',
  },
  {
    id: 'f4',
    type: 'progress',
    timestamp: ago(30 * 60 - 3),
    taskId: 'sim_20261018_003',
    stages: [
      { key: 'inspect', label: '模型检查', status: 'done' },
      { key: 'sim', label: '时序仿真', status: 'failed' },
      { key: 'analyze', label: '结果分析', status: 'pending' },
      { key: 'report', label: '报告生成', status: 'pending' },
    ],
  },
]

// 已完成（s3）：第二个完整示例。
export const completedMessages: Message[] = [
  {
    id: 'c1',
    type: 'user',
    timestamp: ago(26 * 60),
    content: '做一次馈线 N-1 校验，看联络开关转供是否满足供电可靠性。',
  },
  {
    id: 'c2',
    type: 'tool-call',
    timestamp: ago(26 * 60 - 1),
    toolName: 'inspect_model',
    args: { model: '配电网拓扑模型.case' },
    status: 'success',
    summary: 'model ok',
  },
  {
    id: 'c3',
    type: 'tool-call',
    timestamp: ago(26 * 60 - 2),
    toolName: 'run_simulation',
    args: { model: '配电网拓扑模型.case', scenario: 'N-1' },
    status: 'success',
    summary: 'task_id: sim_20261020_004',
  },
  {
    id: 'c4',
    type: 'progress',
    timestamp: ago(26 * 60 - 3),
    taskId: 'sim_20261020_004',
    stages: [
      { key: 'inspect', label: '模型检查', status: 'done' },
      { key: 'sim', label: '时序仿真', status: 'done' },
      { key: 'analyze', label: '结果分析', status: 'done' },
      { key: 'report', label: '报告生成', status: 'done' },
    ],
  },
  {
    id: 'c5',
    type: 'result',
    timestamp: ago(26 * 60 - 4),
    taskId: 'sim_20261020_004',
    title: '馈线 N-1 校验结果',
    metrics: { maxVoltage: 1.045, minVoltage: 0.972, violationNodes: 0, violationDuration: '0 h' },
  },
]

// 更早（s5）：简短的已完成示例。
export const briefMessages: Message[] = [
  {
    id: 'b1',
    type: 'user',
    timestamp: ago(3 * 24 * 60),
    content: '分析配变台区的三相不平衡情况。',
  },
  {
    id: 'b2',
    type: 'result',
    timestamp: ago(3 * 24 * 60 - 1),
    taskId: 'sim_20261018_005',
    title: '台区三相不平衡分析结果',
    metrics: { maxVoltage: 1.011, minVoltage: 0.988, violationNodes: 0, violationDuration: '0 h' },
  },
]

// 会话 id 与 sessions.ts 保持一致。
export const messagesBySession: Record<string, Message[]> = {
  s1: demoMessages,
  s2: runningMessages,
  s3: completedMessages,
  s4: failedMessages,
  s5: briefMessages,
}
