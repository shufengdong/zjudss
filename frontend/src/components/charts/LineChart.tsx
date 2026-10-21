import { useEffect, useRef } from 'react'
import * as echarts from 'echarts'
import type { EChartsOption } from 'echarts'
import type { CurveData } from '../../types'

interface LineChartProps {
  data: CurveData
  height?: number
}

// 可复用的折线图：时间轴 / 数值 / 变量名 / 单位全部通过 props 传入，
// 支持 dataZoom 缩放与图例切换，容器尺寸变化时自动 resize。
export default function LineChart({ data, height = 300 }: LineChartProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const chartRef = useRef<echarts.ECharts | null>(null)

  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    const chart = echarts.init(el)
    chartRef.current = chart
    const observer = new ResizeObserver(() => chart.resize())
    observer.observe(el)
    return () => {
      observer.disconnect()
      chart.dispose()
      chartRef.current = null
    }
  }, [])

  useEffect(() => {
    const chart = chartRef.current
    if (!chart) return
    const option: EChartsOption = {
      color: ['#4d6bfe', '#12b76a', '#f79009', '#f04438'],
      tooltip: { trigger: 'axis' },
      legend: {
        data: data.series.map((s) => s.name),
        top: 0,
        left: 4,
        icon: 'roundRect',
        itemWidth: 14,
        itemHeight: 8,
      },
      grid: { left: 52, right: 20, top: 40, bottom: 56 },
      dataZoom: [
        { type: 'inside', throttle: 50 },
        { type: 'slider', height: 18, bottom: 8 },
      ],
      xAxis: { type: 'category', data: data.timeAxis, boundaryGap: false },
      yAxis: { type: 'value', name: `${data.variable} (${data.unit})`, scale: true },
      series: data.series.map((s) => ({
        name: s.name,
        type: 'line' as const,
        data: s.values,
        smooth: true,
        showSymbol: false,
      })),
    }
    chart.setOption(option, true)
  }, [data])

  return <div ref={containerRef} style={{ height }} className="w-full" />
}
