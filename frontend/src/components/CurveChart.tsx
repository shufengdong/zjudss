import ReactECharts from 'echarts-for-react'

// 占位示例：接入真实数据时替换掉 option 即可
const option = {
  xAxis: {
    type: 'category' as const,
    data: ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'],
  },
  yAxis: { type: 'value' as const },
  series: [
    {
      type: 'line' as const,
      data: [820, 932, 901, 934, 1290, 1330, 1320],
      smooth: true,
    },
  ],
}

export default function CurveChart() {
  return <ReactECharts option={option} style={{ height: 400 }} />
}
