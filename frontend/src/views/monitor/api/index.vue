<template>
  <div class="monitor-page">
    <n-page-header title="接口监控" subtitle="接口性能排行 / 报错统计">
      <template #extra>
        <n-button size="tiny" @click="resetMetrics">重置统计</n-button>
        <n-button :loading="loading" @click="fetchData">刷新</n-button>
      </template>
    </n-page-header>

    <n-alert v-if="errorMsg" type="error" closable @close="errorMsg = ''" style="margin-bottom:12px">
      {{ errorMsg }}
    </n-alert>

    <!-- 概览卡片 -->
    <n-grid :cols="4" :x-gap="12" style="margin-bottom:12px">
      <n-gi><n-card size="small"><template #header>总接口数</template>{{ summary.total_endpoints }}</n-card></n-gi>
      <n-gi><n-card size="small"><template #header>总请求数</template>{{ summary.total_calls }}</n-card></n-gi>
      <n-gi><n-card size="small"><template #header>错误数</template><span :style="{ color: summary.total_errors > 0 ? 'red' : '' }">{{ summary.total_errors }}</span></n-card></n-gi>
      <n-gi><n-card size="small"><template #header>平均响应</template>{{ summary.avg_response_ms }} ms</n-card></n-gi>
    </n-grid>

    <n-card title="接口性能排行" size="small" style="margin-bottom:12px">
      <n-data-table :columns="columns" :data="metrics" :loading="loading" :bordered="true" size="small" :max-height="400" />
    </n-card>

    <n-card v-if="alerts.length > 0" title="告警信息" size="small">
      <n-timeline>
        <n-timeline-item v-for="(a, i) in alerts" :key="i"
          :type="a.level === 'critical' ? 'error' : 'warning'" :content="a.message" />
      </n-timeline>
    </n-card>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, onMounted, onBeforeUnmount, h } from 'vue'
import { NTag } from 'naive-ui'
import type { DataTableColumn } from 'naive-ui'
import { monitorApi } from '@/api/monitor'
import type { ApiMetric, AlertItem } from '@/api/monitor'
import { showSuccess } from '@/utils/message'

const loading = ref(false)
const errorMsg = ref('')
const metrics = ref<Record<string, unknown>[]>([])
const alerts = ref<AlertItem[]>([])
const summary = reactive({ total_endpoints: 0, total_calls: 0, total_errors: 0, avg_response_ms: 0 })
let timer: ReturnType<typeof setInterval> | null = null

const columns: DataTableColumn[] = [
  { title: '#', key: 'index', width: 50, render(_r: unknown, index: number) { return index + 1 } },
  {
    title: '方法', key: 'method', width: 80,
    render(row: Record<string, unknown>) {
      const m = row.method as string
      const color = m === 'GET' ? 'success' : m === 'POST' ? 'primary' : m === 'PUT' ? 'warning' : 'error'
      return h(NTag, { type: color as 'success' | 'primary' | 'warning' | 'error', size: 'tiny' }, () => m)
    },
  },
  { title: '路径', key: 'path', ellipsis: { tooltip: true } },
  { title: '调用次数', key: 'call_count', width: 90,
    sorter: (a: Record<string, unknown>, b: Record<string, unknown>) => (a.call_count as number) - (b.call_count as number) },
  { title: '错误数', key: 'error_count', width: 80,
    sorter: (a: Record<string, unknown>, b: Record<string, unknown>) => (a.error_count as number) - (b.error_count as number) },
  { title: '平均(ms)', key: 'avg_duration_ms', width: 90,
    sorter: (a: Record<string, unknown>, b: Record<string, unknown>) => (a.avg_duration_ms as number) - (b.avg_duration_ms as number) },
  { title: '最慢(ms)', key: 'max_duration_ms', width: 90 },
  { title: '最快(ms)', key: 'min_duration_ms', width: 90 },
]

async function fetchData() {
  loading.value = true
  errorMsg.value = ''
  try {
    const res = await monitorApi.getApiMetrics()
    const d = res as unknown as { metrics: ApiMetric[]; summary: Record<string, unknown> }
    metrics.value = d.metrics as unknown as Record<string, unknown>[]
    Object.assign(summary, d.summary)

    const alertRes = await monitorApi.getAlerts()
    const ad = alertRes as unknown as { alerts: AlertItem[] }
    alerts.value = ad.alerts
  } catch (e) {
    errorMsg.value = (e as Error).message || '加载失败'
  } finally {
    loading.value = false
  }
}

async function resetMetrics() {
  try {
    await monitorApi.resetMetrics()
    showSuccess('指标已重置')
    fetchData()
  } catch { /* */ }
}

onMounted(() => {
  fetchData()
  timer = setInterval(fetchData, 5_000)
})

onBeforeUnmount(() => {
  if (timer) clearInterval(timer)
})
</script>
