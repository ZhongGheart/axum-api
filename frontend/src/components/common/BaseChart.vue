<template>
  <div class="base-chart" :style="{ height: height + 'px' }">
    <v-chart
      v-if="!loading"
      :option="chartOptions"
      :autoresize="true"
      :loading="loading"
      :theme="theme"
      style="width:100%;height:100%"
    />
    <n-spin v-else size="large" style="display:flex;align-items:center;justify-content:center;height:100%" />
  </div>
</template>

<script setup lang="ts">
/**
 * 通用图表组件
 *
 * 基于 ECharts + vue-echarts，适配 Naive UI 主题。
 *
 * 使用方式：
 *   <BaseChart :options="lineOptions" :height="300" />
 */
import { computed } from 'vue'
import VChart from 'vue-echarts'
import { use } from 'echarts/core'
import { CanvasRenderer } from 'echarts/renderers'
import { LineChart, BarChart, PieChart } from 'echarts/charts'
import {
  TitleComponent,
  TooltipComponent,
  LegendComponent,
  GridComponent,
} from 'echarts/components'
import type { EChartsOption } from 'echarts'
import { useAppStore } from '@/stores/app'

// 注册 ECharts 组件
use([
  CanvasRenderer,
  LineChart,
  BarChart,
  PieChart,
  TitleComponent,
  TooltipComponent,
  LegendComponent,
  GridComponent,
])

const props = withDefaults(
  defineProps<{
    /** ECharts 配置项 */
    options: EChartsOption
    /** 容器高度（px） */
    height?: number
    /** 加载中 */
    loading?: boolean
  }>(),
  { height: 300, loading: false },
)

const appStore = useAppStore()

/** 根据主题切换 ECharts 文字颜色 */
const theme = computed(() => (appStore.isDark ? 'dark' : 'light'))

const chartOptions = computed<EChartsOption>(() => ({
  tooltip: { trigger: 'axis' },
  grid: { left: '3%', right: '4%', bottom: '3%', containLabel: true },
  ...props.options,
}))
</script>

<style scoped>
.base-chart {
  width: 100%;
  min-height: 200px;
}
</style>
