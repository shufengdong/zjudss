import type { CurveData } from '../types'

// 24 小时 × 每 15 分钟一个点 = 96 个点。
// 组件接口按「真实数据可能上千点」设计，时间轴与数值全部通过 props 传入。
const POINTS = 96

function timeAxis(): string[] {
  const out: string[] = []
  for (let i = 0; i < POINTS; i++) {
    const total = i * 15
    const h = Math.floor(total / 60)
    const m = total % 60
    out.push(`${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}`)
  }
  return out
}

function voltageSeries(base: number, phase: number): number[] {
  const out: number[] = []
  for (let i = 0; i < POINTS; i++) {
    const t = i / (POINTS - 1)
    // 晚高峰 / 早高峰电压跌落，中午光伏抬升，叠加轻微纹波
    const evening = Math.exp(-((t - 0.8) ** 2) / 0.0064)
    const morning = Math.exp(-((t - 0.34) ** 2) / 0.008)
    const pv = 0.028 * Math.exp(-((t - 0.5) ** 2) / 0.0144)
    const ripple = 0.006 * Math.sin(2 * Math.PI * t * 3 + phase)
    const v = base - 0.06 * evening - 0.03 * morning + pv + ripple
    out.push(Number(v.toFixed(4)))
  }
  return out
}

function powerSeries(peak: number, phase: number): number[] {
  const out: number[] = []
  for (let i = 0; i < POINTS; i++) {
    const t = i / (POINTS - 1)
    const evening = Math.exp(-((t - 0.8) ** 2) / 0.0064)
    const morning = Math.exp(-((t - 0.34) ** 2) / 0.008)
    const pv = 0.25 * Math.exp(-((t - 0.5) ** 2) / 0.0144)
    const ripple = 0.03 * Math.sin(2 * Math.PI * t * 2 + phase)
    const v = peak * (0.55 + 0.5 * evening + 0.25 * morning - pv) + peak * ripple
    out.push(Math.round(v))
  }
  return out
}

export const voltageCurve: CurveData = {
  variable: '电压',
  unit: 'p.u.',
  timeAxis: timeAxis(),
  series: [
    { name: '馈线 A', values: voltageSeries(1.0, 0) },
    { name: '馈线 B', values: voltageSeries(1.015, 1.6) },
  ],
}

export const powerCurve: CurveData = {
  variable: '有功功率',
  unit: 'kW',
  timeAxis: timeAxis(),
  series: [
    { name: '馈线 A', values: powerSeries(820, 0) },
    { name: '馈线 B', values: powerSeries(640, 1.2) },
  ],
}
