import type { Session } from '../types'

const now = new Date()
const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
const DAY = 24 * 60 * 60 * 1000
const MIN = 60 * 1000

// 今天：相对 now 回退若干分钟；昨天 / 更早：锚定到日历日，保证分组稳定。
const todayMinus = (minutes: number) => new Date(now.getTime() - minutes * MIN).toISOString()
const daysAgoAt = (days: number, hour: number, minute: number) =>
  new Date(startOfToday - days * DAY + (hour * 60 + minute) * MIN).toISOString()

export const sessions: Session[] = [
  {
    id: 's1',
    title: '城区配电网电压越限分析',
    status: 'completed',
    updatedAt: todayMinus(25),
  },
  {
    id: 's2',
    title: '分布式光伏接入时序仿真',
    status: 'running',
    updatedAt: todayMinus(120),
  },
  {
    id: 's3',
    title: '馈线 N-1 校验',
    status: 'completed',
    updatedAt: daysAgoAt(1, 10, 30),
  },
  {
    id: 's4',
    title: '春节高峰负荷仿真',
    status: 'failed',
    updatedAt: daysAgoAt(1, 16, 20),
  },
  {
    id: 's5',
    title: '配变台区三相不平衡分析',
    status: 'completed',
    updatedAt: daysAgoAt(3, 9, 15),
  },
]
